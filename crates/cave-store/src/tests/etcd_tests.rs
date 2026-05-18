// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for the etcd v3 API (MVCC engine).

use crate::engine::{Compare, CompareResult, CompareTarget, MvccEngine, TxnOp, TxnRequest};
use crate::wal::WalWriter;
use std::sync::Arc;
use tempfile::TempDir;

fn make_engine(dir: &TempDir) -> Arc<MvccEngine> {
    let wal = WalWriter::open(dir.path()).unwrap();
    Arc::new(MvccEngine::new(wal))
}

#[tokio::test]
async fn test_put_and_get() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let r = engine
        .put(b"foo".to_vec(), b"bar".to_vec(), 0, false)
        .await
        .unwrap();
    assert_eq!(r.header.revision, 1);

    let range = engine
        .range(b"foo".to_vec(), vec![], 0, 0, false, false)
        .await
        .unwrap();
    assert_eq!(range.kvs.len(), 1);
    assert_eq!(range.kvs[0].key, b"foo");
    assert_eq!(range.kvs[0].value, b"bar");
    assert_eq!(range.kvs[0].version, 1);
    assert_eq!(range.kvs[0].create_revision, 1);
}

#[tokio::test]
async fn test_delete_range() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"key1".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"key2".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();
    engine.put(b"key3".to_vec(), b"v3".to_vec(), 0, false).await.unwrap();

    // Delete range [key1, key3)
    let del = engine
        .delete_range(b"key1".to_vec(), b"key3".to_vec(), true)
        .await
        .unwrap();
    assert_eq!(del.deleted, 2);
    assert_eq!(del.prev_kvs.len(), 2);

    // key3 still exists
    let range = engine
        .range(b"key3".to_vec(), vec![], 0, 0, false, false)
        .await
        .unwrap();
    assert_eq!(range.kvs.len(), 1);
}

#[tokio::test]
async fn test_mvcc_revision_tracking() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"k".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"k".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();
    engine.put(b"k".to_vec(), b"v3".to_vec(), 0, false).await.unwrap();

    // Current value
    let r = engine.range(b"k".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"v3");
    assert_eq!(r.kvs[0].version, 3);

    // Historical value at revision 1
    let r_hist = engine.range(b"k".to_vec(), vec![], 1, 0, false, false).await.unwrap();
    assert_eq!(r_hist.kvs[0].value, b"v1");
}

#[tokio::test]
async fn test_prefix_range() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"foo/a".to_vec(), b"1".to_vec(), 0, false).await.unwrap();
    engine.put(b"foo/b".to_vec(), b"2".to_vec(), 0, false).await.unwrap();
    engine.put(b"foo/c".to_vec(), b"3".to_vec(), 0, false).await.unwrap();
    engine.put(b"bar/a".to_vec(), b"4".to_vec(), 0, false).await.unwrap();

    // Range [foo/, foo0) captures all foo/* keys (0 = '/' + 1 in ASCII)
    let range_end = {
        let mut v = b"foo/".to_vec();
        *v.last_mut().unwrap() += 1;
        v
    };
    let r = engine
        .range(b"foo/".to_vec(), range_end, 0, 0, false, false)
        .await
        .unwrap();
    assert_eq!(r.kvs.len(), 3);
}

#[tokio::test]
async fn test_transaction_success() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"balance".to_vec(), b"100".to_vec(), 0, false).await.unwrap();

    // CAS: if version == 1, set to 200
    let txn = TxnRequest {
        compare: vec![Compare {
            key: b"balance".to_vec(),
            result: CompareResult::Equal,
            target: CompareTarget::Version(1),
        }],
        success: vec![TxnOp::Put {
            key: b"balance".to_vec(),
            value: b"200".to_vec(),
            lease_id: 0,
        }],
        failure: vec![],
    };

    let resp = engine.txn(txn).await.unwrap();
    assert!(resp.succeeded);

    let r = engine.range(b"balance".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"200");
}

#[tokio::test]
async fn test_transaction_failure() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"balance".to_vec(), b"100".to_vec(), 0, false).await.unwrap();

    // CAS: if version == 99 (wrong), run failure branch
    let txn = TxnRequest {
        compare: vec![Compare {
            key: b"balance".to_vec(),
            result: CompareResult::Equal,
            target: CompareTarget::Version(99),
        }],
        success: vec![TxnOp::Put {
            key: b"balance".to_vec(),
            value: b"WRONG".to_vec(),
            lease_id: 0,
        }],
        failure: vec![TxnOp::Put {
            key: b"balance".to_vec(),
            value: b"FALLBACK".to_vec(),
            lease_id: 0,
        }],
    };

    let resp = engine.txn(txn).await.unwrap();
    assert!(!resp.succeeded);

    let r = engine.range(b"balance".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"FALLBACK");
}

#[tokio::test]
async fn test_compaction() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"k".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"k".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();
    engine.put(b"k".to_vec(), b"v3".to_vec(), 0, false).await.unwrap();

    // Compact at revision 2 — history before rev 2 is pruned
    engine.compact(2).await.unwrap();

    // Accessing revision 1 should fail
    let err = engine.range(b"k".to_vec(), vec![], 1, 0, false, false).await;
    assert!(err.is_err());

    // Current value still accessible
    let r = engine.range(b"k".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"v3");
}

#[tokio::test]
async fn test_watch() {
    let dir = TempDir::new().unwrap();
    let engine = Arc::new(MvccEngine::new(WalWriter::open(dir.path()).unwrap()));

    let (_watch_id, mut rx) = engine
        .watch_create(b"foo".to_vec(), vec![], 0, false, false, false, false)
        .await;

    // Trigger a put on a separate task
    let engine_clone = engine.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        engine_clone
            .put(b"foo".to_vec(), b"bar".to_vec(), 0, false)
            .await
            .unwrap();
    });

    let event = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(event.key, b"foo");
    assert_eq!(event.value.as_deref(), Some(b"bar".as_slice()));
}

#[tokio::test]
async fn test_lease_lifecycle() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    // Grant a lease
    let grant = engine.lease_grant(60, 0).await.unwrap();
    let lease_id = grant.id;
    assert_eq!(grant.ttl, 60);

    // Attach a key to the lease
    engine
        .put(b"leased-key".to_vec(), b"value".to_vec(), lease_id, false)
        .await
        .unwrap();

    // Key exists
    let r = engine.range(b"leased-key".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs.len(), 1);

    // Keep alive
    let ka = engine.lease_keep_alive(lease_id).await.unwrap();
    assert_eq!(ka.id, lease_id);

    // TTL
    let ttl = engine.lease_time_to_live(lease_id, true).await.unwrap();
    assert!(!ttl.keys.is_empty());

    // Revoke — key should disappear
    engine.lease_revoke(lease_id).await.unwrap();
    let r2 = engine.range(b"leased-key".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r2.kvs.len(), 0);
}

#[tokio::test]
async fn test_auth_user_role() {
    use crate::etcd::auth::AuthManager;

    let auth = AuthManager::default();

    auth.user_add("alice".to_string(), "password123".to_string()).await.unwrap();
    auth.role_add("reader".to_string()).await.unwrap();
    auth.user_grant_role("alice", "reader").await.unwrap();

    let user = auth.user_get("alice").await.unwrap();
    assert!(user.roles.contains(&"reader".to_string()));

    auth.user_revoke_role("alice", "reader").await.unwrap();
    let user2 = auth.user_get("alice").await.unwrap();
    assert!(user2.roles.is_empty());

    // Enable auth
    auth.enable().await.unwrap();
    assert!(auth.is_enabled().await);

    // Authenticate
    let tok = auth.authenticate("alice", "password123").await.unwrap();
    assert!(!tok.is_empty());

    // Wrong password
    let bad = auth.authenticate("alice", "wrong").await;
    assert!(bad.is_err());
}
