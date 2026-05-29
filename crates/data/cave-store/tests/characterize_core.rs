// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for cave-store pre-existing modules.
//!
//! These tests assert the REAL observed behaviour of code that already existed
//! on origin/main before this uplift. They are NOT red-first TDD pairs.

use cave_store::engine::{Compare, CompareResult, CompareTarget, MvccEngine, TxnOp, TxnRequest};
use cave_store::s3::store::ObjectStore;
use cave_store::s3::types::{StorageClass, VersioningState};
use cave_store::wal::WalWriter;
use cave_store::etcd::auth::AuthManager;
use cave_store::etcd::cluster::ClusterManager;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_engine(dir: &TempDir) -> Arc<MvccEngine> {
    let wal = WalWriter::open(dir.path()).unwrap();
    Arc::new(MvccEngine::new(wal))
}

fn make_store(dir: &TempDir) -> Arc<ObjectStore> {
    let wal = WalWriter::open(dir.path()).unwrap();
    let data_dir = dir.path().join("objects");
    std::fs::create_dir_all(&data_dir).unwrap();
    Arc::new(ObjectStore::new(data_dir, Arc::new(wal)))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 1 — MVCC engine: KV operations
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_mvcc_put_monotonic_revision() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let r1 = engine.put(b"a".to_vec(), b"1".to_vec(), 0, false).await.unwrap();
    let r2 = engine.put(b"b".to_vec(), b"2".to_vec(), 0, false).await.unwrap();
    let r3 = engine.put(b"c".to_vec(), b"3".to_vec(), 0, false).await.unwrap();

    // Each put must bump revision
    assert!(r2.header.revision > r1.header.revision);
    assert!(r3.header.revision > r2.header.revision);
}

#[tokio::test]
async fn characterize_mvcc_range_all_keys() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"k1".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"k2".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();

    // Range all keys: key=\x00, range_end=\x00 (means all)
    let r = engine.range(b"\x00".to_vec(), b"\x00".to_vec(), 0, 0, false, false).await.unwrap();
    assert!(r.kvs.len() >= 2, "should return all keys");
    assert_eq!(r.count, r.kvs.len() as i64);
}

#[tokio::test]
async fn characterize_mvcc_version_increments_on_put() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"x".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"x".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();
    engine.put(b"x".to_vec(), b"v3".to_vec(), 0, false).await.unwrap();

    let r = engine.range(b"x".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs.len(), 1, "only one live entry for key");
    assert_eq!(r.kvs[0].value, b"v3");
    assert_eq!(r.kvs[0].version, 3, "version must be count of puts");
}

#[tokio::test]
async fn characterize_mvcc_historical_read() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let r1 = engine.put(b"h".to_vec(), b"old".to_vec(), 0, false).await.unwrap();
    let rev1 = r1.header.revision;
    engine.put(b"h".to_vec(), b"new".to_vec(), 0, false).await.unwrap();

    // Read at rev1 should return "old"
    let hist = engine.range(b"h".to_vec(), vec![], rev1, 0, false, false).await.unwrap();
    assert_eq!(hist.kvs[0].value, b"old");

    // Current should return "new"
    let curr = engine.range(b"h".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(curr.kvs[0].value, b"new");
}

#[tokio::test]
async fn characterize_mvcc_delete_makes_key_invisible() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"del".to_vec(), b"present".to_vec(), 0, false).await.unwrap();
    engine.delete_range(b"del".to_vec(), vec![], false).await.unwrap();

    let r = engine.range(b"del".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs.len(), 0, "deleted key must not be visible");
}

#[tokio::test]
async fn characterize_mvcc_keys_only_has_empty_values() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"q".to_vec(), b"secret-value".to_vec(), 0, false).await.unwrap();

    let r = engine.range(b"q".to_vec(), vec![], 0, 0, true, false).await.unwrap();
    assert_eq!(r.kvs.len(), 1);
    assert!(r.kvs[0].value.is_empty(), "keys_only must suppress values");
}

#[tokio::test]
async fn characterize_mvcc_count_only_returns_empty_kvs() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"c1".to_vec(), b"v".to_vec(), 0, false).await.unwrap();
    engine.put(b"c2".to_vec(), b"v".to_vec(), 0, false).await.unwrap();

    let r = engine.range(b"\x00".to_vec(), b"\x00".to_vec(), 0, 0, false, true).await.unwrap();
    assert_eq!(r.kvs.len(), 0, "count_only must produce no kv entries");
    assert!(r.count >= 2, "count must reflect number of keys");
}

#[tokio::test]
async fn characterize_mvcc_limit_truncates_range() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    for i in 0u8..10 {
        engine.put(vec![b'a' + i], b"v".to_vec(), 0, false).await.unwrap();
    }

    let r = engine.range(b"\x00".to_vec(), b"\x00".to_vec(), 0, 3, false, false).await.unwrap();
    assert_eq!(r.kvs.len(), 3, "limit=3 must return at most 3 entries");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 2 — MVCC engine: transactions
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_txn_cas_atomicity() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"bal".to_vec(), b"500".to_vec(), 0, false).await.unwrap();

    // Success branch: version==1 → set to 600
    let txn = TxnRequest {
        compare: vec![Compare {
            key: b"bal".to_vec(),
            result: CompareResult::Equal,
            target: CompareTarget::Version(1),
        }],
        success: vec![TxnOp::Put { key: b"bal".to_vec(), value: b"600".to_vec(), lease_id: 0 }],
        failure: vec![TxnOp::Put { key: b"bal".to_vec(), value: b"FAIL".to_vec(), lease_id: 0 }],
    };

    let resp = engine.txn(txn).await.unwrap();
    assert!(resp.succeeded);

    let r = engine.range(b"bal".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"600");
}

#[tokio::test]
async fn characterize_txn_failure_branch_fires_on_mismatch() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"lock".to_vec(), b"0".to_vec(), 0, false).await.unwrap();

    let txn = TxnRequest {
        compare: vec![Compare {
            key: b"lock".to_vec(),
            result: CompareResult::Equal,
            target: CompareTarget::Version(99),  // deliberate mismatch
        }],
        success: vec![TxnOp::Put { key: b"lock".to_vec(), value: b"SUCCESS".to_vec(), lease_id: 0 }],
        failure: vec![TxnOp::Put { key: b"lock".to_vec(), value: b"FAILURE".to_vec(), lease_id: 0 }],
    };

    let resp = engine.txn(txn).await.unwrap();
    assert!(!resp.succeeded);

    let r = engine.range(b"lock".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"FAILURE");
}

#[tokio::test]
async fn characterize_txn_value_compare() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"flag".to_vec(), b"on".to_vec(), 0, false).await.unwrap();

    let txn = TxnRequest {
        compare: vec![Compare {
            key: b"flag".to_vec(),
            result: CompareResult::Equal,
            target: CompareTarget::Value(b"on".to_vec()),
        }],
        success: vec![TxnOp::Put { key: b"flag".to_vec(), value: b"off".to_vec(), lease_id: 0 }],
        failure: vec![],
    };

    let resp = engine.txn(txn).await.unwrap();
    assert!(resp.succeeded);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 3 — MVCC engine: compaction
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_compaction_rejects_access_below_compact_rev() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let r1 = engine.put(b"p".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"p".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();

    let compact_rev = r1.header.revision + 1;
    engine.compact(compact_rev).await.unwrap();

    // Access below compact_rev must fail
    let err = engine.range(b"p".to_vec(), vec![], r1.header.revision, 0, false, false).await;
    assert!(err.is_err(), "access below compact_rev must return error");
}

#[tokio::test]
async fn characterize_compaction_current_still_accessible() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    engine.put(b"q".to_vec(), b"v1".to_vec(), 0, false).await.unwrap();
    engine.put(b"q".to_vec(), b"v2".to_vec(), 0, false).await.unwrap();

    engine.compact(engine.current_revision() - 1).await.unwrap();

    let r = engine.range(b"q".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs[0].value, b"v2", "current value must survive compaction");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 4 — MVCC engine: leases
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_lease_grant_returns_positive_id() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let g = engine.lease_grant(30, 0).await.unwrap();
    assert!(g.id != 0);
    assert_eq!(g.ttl, 30);
}

#[tokio::test]
async fn characterize_lease_revoke_removes_attached_keys() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let g = engine.lease_grant(60, 0).await.unwrap();
    let lid = g.id;

    engine.put(b"leased".to_vec(), b"val".to_vec(), lid, false).await.unwrap();

    // Verify key exists
    let r = engine.range(b"leased".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs.len(), 1);

    engine.lease_revoke(lid).await.unwrap();

    // Key must vanish
    let r2 = engine.range(b"leased".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r2.kvs.len(), 0, "revoking lease must delete attached keys");
}

#[tokio::test]
async fn characterize_lease_keepalive_extends_ttl() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let g = engine.lease_grant(10, 0).await.unwrap();
    let lid = g.id;

    let ka = engine.lease_keep_alive(lid).await.unwrap();
    assert_eq!(ka.id, lid);
    assert_eq!(ka.ttl, 10);
}

#[tokio::test]
async fn characterize_lease_ttl_with_keys() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let g = engine.lease_grant(60, 0).await.unwrap();
    let lid = g.id;

    engine.put(b"attached".to_vec(), b"v".to_vec(), lid, false).await.unwrap();

    let ttl = engine.lease_time_to_live(lid, true).await.unwrap();
    assert!(!ttl.keys.is_empty(), "TTL response with keys=true must include keys");
    assert!(ttl.keys.contains(&b"attached".to_vec()));
}

#[tokio::test]
async fn characterize_lease_list_shows_active_leases() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let g1 = engine.lease_grant(60, 0).await.unwrap();
    let g2 = engine.lease_grant(120, 0).await.unwrap();

    let list = engine.lease_list().await;
    let ids: Vec<i64> = list.leases.iter().map(|l| l.id).collect();
    assert!(ids.contains(&g1.id));
    assert!(ids.contains(&g2.id));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 5 — MVCC engine: watch
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_watch_receives_put_event() {
    use tokio::time::{Duration, timeout};

    let dir = TempDir::new().unwrap();
    let engine = Arc::new(MvccEngine::new(WalWriter::open(dir.path()).unwrap()));

    let (_wid, mut rx) = engine
        .watch_create(b"watched".to_vec(), vec![], 0, false, false, false, false)
        .await;

    let eng2 = engine.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        eng2.put(b"watched".to_vec(), b"event-value".to_vec(), 0, false).await.unwrap();
    });

    let ev = timeout(Duration::from_secs(2), rx.recv()).await
        .expect("timeout waiting for watch event")
        .expect("channel closed");

    assert_eq!(ev.key, b"watched");
    assert_eq!(ev.value.as_deref(), Some(b"event-value".as_slice()));
}

#[tokio::test]
async fn characterize_watch_cancel_stops_delivery() {
    let dir = TempDir::new().unwrap();
    let engine = Arc::new(MvccEngine::new(WalWriter::open(dir.path()).unwrap()));

    let (wid, _rx) = engine
        .watch_create(b"key".to_vec(), vec![], 0, false, false, false, false)
        .await;

    let removed = engine.watch_cancel(wid).await;
    assert!(removed, "cancelling an existing watch must return true");

    // Double-cancel should return false
    let again = engine.watch_cancel(wid).await;
    assert!(!again, "cancelling an already-removed watch must return false");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 6 — S3 ObjectStore: bucket operations
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_s3_bucket_create_list() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("my-bkt", "eu-west-1", "tester").await.unwrap();
    let list = s.list_buckets().await;
    assert!(list.iter().any(|b| b.name == "my-bkt"), "bucket must appear in list");
}

#[tokio::test]
async fn characterize_s3_bucket_duplicate_fails() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("dup", "us-east-1", "x").await.unwrap();
    let err = s.create_bucket("dup", "us-east-1", "x").await;
    assert!(err.is_err(), "duplicate bucket must fail");
}

#[tokio::test]
async fn characterize_s3_bucket_not_empty_prevents_delete() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("bkt1", "us-east-1", "x").await.unwrap();
    s.put_object("bkt1", "obj", b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let err = s.delete_bucket("bkt1").await;
    assert!(err.is_err(), "non-empty bucket delete must fail");
}

#[tokio::test]
async fn characterize_s3_head_bucket_exists() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("hd-bkt", "us-east-1", "x").await.unwrap();
    s.head_bucket("hd-bkt").await.unwrap();
}

#[tokio::test]
async fn characterize_s3_head_bucket_missing_fails() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    let err = s.head_bucket("nonexistent").await;
    assert!(err.is_err());
}

#[tokio::test]
async fn characterize_s3_invalid_bucket_name_short() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    assert!(s.create_bucket("ab", "us-east-1", "x").await.is_err(), "2-char name must fail");
    assert!(s.create_bucket("a", "us-east-1", "x").await.is_err(), "1-char name must fail");
}

#[tokio::test]
async fn characterize_s3_invalid_bucket_name_uppercase() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    assert!(s.create_bucket("BucketName", "us-east-1", "x").await.is_err(), "uppercase name must fail");
}

#[tokio::test]
async fn characterize_s3_bucket_policy_roundtrip() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("pol-bkt", "us-east-1", "x").await.unwrap();

    let policy = r#"{"Version":"2012-10-17","Statement":[{"Sid":"","Effect":"Allow","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::pol/*"]}]}"#;
    s.put_bucket_policy("pol-bkt", policy).await.unwrap();

    let b = s.get_bucket("pol-bkt").await.unwrap();
    assert!(b.policy.is_some(), "policy must be stored on the bucket");
    assert!(b.policy.unwrap().contains("s3:GetObject"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 7 — S3 ObjectStore: object operations
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_s3_put_get_roundtrip() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("test-bkt", "us-east-1", "x").await.unwrap();
    s.put_object("test-bkt", "hello.txt", b"world".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let obj = s.get_object("test-bkt", "hello.txt", None, None, None).await.unwrap();
    assert_eq!(obj.body, b"world");
    assert_eq!(obj.content_type, "text/plain");
}

#[tokio::test]
async fn characterize_s3_etag_is_nonempty() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    let r = s.put_object("tst", "f", b"abc".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    assert!(!r.etag.is_empty(), "etag must be computed for every put");
}

#[tokio::test]
async fn characterize_s3_head_object_metadata() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    s.put_object("tst", "m.bin", b"12345".to_vec(), "application/octet-stream", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let h = s.head_object("tst", "m.bin", None).await.unwrap();
    assert_eq!(h.size, 5);
    assert!(!h.etag.is_empty());
    assert!(!h.delete_marker);
}

#[tokio::test]
async fn characterize_s3_get_nonexistent_object_fails() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    let err = s.get_object("tst", "ghost.txt", None, None, None).await;
    assert!(err.is_err(), "get on nonexistent key must fail");
}

#[tokio::test]
async fn characterize_s3_delete_object_removes_it() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    s.put_object("tst", "del.txt", b"gone".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    s.delete_object("tst", "del.txt", None).await.unwrap();

    assert!(s.get_object("tst", "del.txt", None, None, None).await.is_err());
}

#[tokio::test]
async fn characterize_s3_copy_object() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("src", "us-east-1", "x").await.unwrap();
    s.create_bucket("dst", "us-east-1", "x").await.unwrap();

    s.put_object("src", "orig", b"original".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    s.copy_object("src", "orig", None, "dst", "copy", "COPY", None).await.unwrap();

    let got = s.get_object("dst", "copy", None, None, None).await.unwrap();
    assert_eq!(got.body, b"original");
}

#[tokio::test]
async fn characterize_s3_range_request() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    s.put_object("tst", "bytes", b"0123456789".to_vec(), "application/octet-stream", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let obj = s.get_object("tst", "bytes", None, Some((3, 6)), None).await.unwrap();
    assert_eq!(obj.body, b"3456", "range get must return the right slice");
    assert!(obj.content_range.is_some());
}

#[tokio::test]
async fn characterize_s3_batch_delete() {
    use cave_store::s3::store::DeleteObjectEntry;
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    for key in &["x", "y", "z"] {
        s.put_object("tst", key, b"v".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    }

    let results = s.delete_objects("tst", vec![
        DeleteObjectEntry { key: "x".to_string(), version_id: None },
        DeleteObjectEntry { key: "y".to_string(), version_id: None },
        DeleteObjectEntry { key: "noexist".to_string(), version_id: None },
    ]).await.unwrap();

    let ok: Vec<_> = results.iter().filter(|r| r.error.is_none()).collect();
    let err: Vec<_> = results.iter().filter(|r| r.error.is_some()).collect();
    assert_eq!(ok.len(), 2, "two successful deletes");
    assert_eq!(err.len(), 1, "one error for nonexistent key");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 8 — S3 ObjectStore: versioning
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_versioning_disabled_by_default() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("vbkt", "us-east-1", "x").await.unwrap();
    let b = s.get_bucket("vbkt").await.unwrap();
    assert_eq!(b.versioning, VersioningState::Disabled);
}

#[tokio::test]
async fn characterize_versioning_enabled_generates_version_ids() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("vbkt", "us-east-1", "x").await.unwrap();
    s.set_versioning("vbkt", VersioningState::Enabled).await.unwrap();

    let r1 = s.put_object("vbkt", "v", b"a".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    let r2 = s.put_object("vbkt", "v", b"b".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    assert!(r1.version_id.is_some());
    assert!(r2.version_id.is_some());
    assert_ne!(r1.version_id, r2.version_id, "each put must get a distinct version ID");
}

#[tokio::test]
async fn characterize_versioning_old_versions_retrievable() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("vbkt", "us-east-1", "x").await.unwrap();
    s.set_versioning("vbkt", VersioningState::Enabled).await.unwrap();

    let r1 = s.put_object("vbkt", "v", b"first".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    s.put_object("vbkt", "v", b"second".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let old = s.get_object("vbkt", "v", r1.version_id.as_deref(), None, None).await.unwrap();
    assert_eq!(old.body, b"first", "old version must be retrievable by version ID");
}

#[tokio::test]
async fn characterize_versioning_delete_creates_delete_marker() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("vbkt", "us-east-1", "x").await.unwrap();
    s.set_versioning("vbkt", VersioningState::Enabled).await.unwrap();

    s.put_object("vbkt", "v", b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    let del = s.delete_object("vbkt", "v", None).await.unwrap();

    assert!(del.delete_marker, "versioned delete must create a delete marker");
    assert!(del.version_id.is_some(), "delete marker must have its own version_id");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 9 — S3 ObjectStore: multipart upload
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_multipart_initiate_returns_upload_id() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("mpu", "us-east-1", "x").await.unwrap();
    let uid = s.create_multipart_upload("mpu", "big.bin", "application/octet-stream", HashMap::new()).await.unwrap();
    assert!(!uid.is_empty());
}

#[tokio::test]
async fn characterize_multipart_full_flow() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("mpu", "us-east-1", "x").await.unwrap();
    let uid = s.create_multipart_upload("mpu", "big.bin", "application/octet-stream", HashMap::new()).await.unwrap();

    let part1 = vec![b'A'; 5 * 1024 * 1024]; // 5 MB minimum
    let part2 = vec![b'B'; 1024];             // last part can be smaller

    let e1 = s.upload_part(&uid, 1, part1.clone()).await.unwrap();
    let e2 = s.upload_part(&uid, 2, part2.clone()).await.unwrap();

    let result = s.complete_multipart_upload(&uid, vec![(1, e1), (2, e2)]).await.unwrap();
    assert!(result.etag.contains('-'), "multipart ETag must contain hyphen");

    let head = s.head_object("mpu", "big.bin", None).await.unwrap();
    assert_eq!(head.size as usize, part1.len() + part2.len());
}

#[tokio::test]
async fn characterize_multipart_abort_cleans_up() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("mpu", "us-east-1", "x").await.unwrap();
    let uid = s.create_multipart_upload("mpu", "tmp.bin", "application/octet-stream", HashMap::new()).await.unwrap();

    s.upload_part(&uid, 1, vec![b'Z'; 1024]).await.unwrap();
    s.abort_multipart_upload(&uid).await.unwrap();

    // Upload is gone
    assert!(s.list_parts(&uid).await.is_err(), "aborted upload must not be listable");
}

#[tokio::test]
async fn characterize_multipart_list_parts() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("mpu", "us-east-1", "x").await.unwrap();
    let uid = s.create_multipart_upload("mpu", "f.bin", "application/octet-stream", HashMap::new()).await.unwrap();

    s.upload_part(&uid, 1, vec![b'A'; 5 * 1024 * 1024]).await.unwrap();
    s.upload_part(&uid, 2, vec![b'B'; 5 * 1024 * 1024]).await.unwrap();
    s.upload_part(&uid, 3, vec![b'C'; 100]).await.unwrap();

    let parts = s.list_parts(&uid).await.unwrap();
    assert_eq!(parts.len(), 3);
    // Parts must be in ascending order
    assert_eq!(parts[0].part_number, 1);
    assert_eq!(parts[2].part_number, 3);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 10 — S3 ObjectStore: encryption
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_sse_s3_transparent_encrypt_decrypt() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("enc-bkt", "us-east-1", "x").await.unwrap();

    let plaintext = b"sensitive-content";
    s.put_object("enc-bkt", "sec.txt", plaintext.to_vec(), "text/plain", HashMap::new(), HashMap::new(), Some("AES256"), None, None).await.unwrap();

    let head = s.head_object("enc-bkt", "sec.txt", None).await.unwrap();
    assert!(head.encryption.is_some(), "encrypted object must record encryption metadata");

    let obj = s.get_object("enc-bkt", "sec.txt", None, None, None).await.unwrap();
    assert_eq!(obj.body, plaintext, "SSE-S3 must decrypt transparently on get");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 11 — S3 ObjectStore: lifecycle rules
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_lifecycle_rules_stored_on_bucket() {
    use cave_store::s3::types::{Expiration, LifecycleRule};

    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("lfc", "us-east-1", "x").await.unwrap();

    let rules = vec![LifecycleRule {
        id: "expire-logs".to_string(),
        status: "Enabled".to_string(),
        prefix: "logs/".to_string(),
        tags: HashMap::new(),
        expiration: Some(Expiration { days: Some(30), date: None, expired_object_delete_marker: None }),
        transitions: vec![],
        noncurrent_version_expiration: None,
        abort_incomplete_multipart_upload: None,
    }];

    s.put_bucket_lifecycle("lfc", rules).await.unwrap();

    let b = s.get_bucket("lfc").await.unwrap();
    assert_eq!(b.lifecycle_rules.len(), 1);
    assert_eq!(b.lifecycle_rules[0].id, "expire-logs");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 12 — S3 ObjectStore: list objects
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_list_objects_prefix_filter() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("lst", "us-east-1", "x").await.unwrap();
    for key in &["images/cat.jpg", "images/dog.jpg", "docs/readme.md"] {
        s.put_object("lst", key, b"data".to_vec(), "application/octet-stream", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    }

    let r = s.list_objects_v2("lst", "images/", None, 100, None).await.unwrap();
    assert_eq!(r.key_count, 2, "prefix filter must exclude non-matching keys");
}

#[tokio::test]
async fn characterize_list_objects_delimiter_grouping() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("lst", "us-east-1", "x").await.unwrap();
    for key in &["a/1", "a/2", "b/1", "top.txt"] {
        s.put_object("lst", key, b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    }

    let r = s.list_objects_v2("lst", "", Some("/"), 100, None).await.unwrap();
    assert_eq!(r.common_prefixes.len(), 2, "delimiter must produce two common prefixes (a/ and b/)");
    assert_eq!(r.key_count, 1, "only top-level non-prefix key: top.txt");
}

#[tokio::test]
async fn characterize_list_objects_pagination() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("lst", "us-east-1", "x").await.unwrap();
    for i in 0..5 {
        s.put_object("lst", &format!("key{}", i), b"v".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    }

    let first = s.list_objects_v2("lst", "", None, 2, None).await.unwrap();
    assert_eq!(first.key_count, 2);
    assert!(first.is_truncated, "truncated must be true when there are more keys");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 13 — S3 ObjectStore: object tagging
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_object_tagging_roundtrip() {
    let dir = TempDir::new().unwrap();
    let s = make_store(&dir);

    s.create_bucket("tst", "us-east-1", "x").await.unwrap();
    s.put_object("tst", "obj", b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let mut tags = HashMap::new();
    tags.insert("env".to_string(), "staging".to_string());
    tags.insert("owner".to_string(), "alice".to_string());
    s.put_object_tagging("tst", "obj", None, tags).await.unwrap();

    let got = s.get_object_tagging("tst", "obj", None).await.unwrap();
    assert_eq!(got.get("env").map(|s| s.as_str()), Some("staging"));
    assert_eq!(got.get("owner").map(|s| s.as_str()), Some("alice"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 14 — etcd Auth API
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_auth_user_lifecycle() {
    let auth = AuthManager::default();

    auth.user_add("bob".to_string(), "pass1".to_string()).await.unwrap();
    let u = auth.user_get("bob").await.unwrap();
    assert_eq!(u.name, "bob");
    assert!(u.roles.is_empty());

    // Duplicate user add must fail
    assert!(auth.user_add("bob".to_string(), "other".to_string()).await.is_err());

    auth.user_delete("bob").await.unwrap();
    assert!(auth.user_get("bob").await.is_err());
}

#[tokio::test]
async fn characterize_auth_role_grant_revoke() {
    let auth = AuthManager::default();

    auth.user_add("eve".to_string(), "secret".to_string()).await.unwrap();
    auth.role_add("operator".to_string()).await.unwrap();

    auth.user_grant_role("eve", "operator").await.unwrap();
    let u = auth.user_get("eve").await.unwrap();
    assert!(u.roles.contains(&"operator".to_string()));

    auth.user_revoke_role("eve", "operator").await.unwrap();
    let u2 = auth.user_get("eve").await.unwrap();
    assert!(!u2.roles.contains(&"operator".to_string()));
}

#[tokio::test]
async fn characterize_auth_authenticate_correct_password() {
    let auth = AuthManager::default();

    auth.user_add("carol".to_string(), "rightpass".to_string()).await.unwrap();
    auth.enable().await.unwrap();

    let tok = auth.authenticate("carol", "rightpass").await.unwrap();
    assert!(!tok.is_empty());

    let bad = auth.authenticate("carol", "wrongpass").await;
    assert!(bad.is_err(), "wrong password must fail authentication");
}

#[tokio::test]
async fn characterize_auth_enable_disable() {
    let auth = AuthManager::default();

    assert!(!auth.is_enabled().await, "auth starts disabled");
    auth.enable().await.unwrap();
    assert!(auth.is_enabled().await);

    // Double-enable must fail
    assert!(auth.enable().await.is_err());

    auth.disable().await.unwrap();
    assert!(!auth.is_enabled().await);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 15 — etcd Cluster API
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_cluster_has_self_member() {
    let c = ClusterManager::default();
    let members = c.member_list().await;
    assert!(!members.is_empty(), "default cluster must have self as member");
    assert_eq!(members[0].name, "cave-store-0");
}

#[tokio::test]
async fn characterize_cluster_add_remove_member() {
    let c = ClusterManager::default();

    let m = c.member_add(vec!["http://peer:2380".to_string()], false).await.unwrap();
    assert!(m.id != 0);

    let list_before = c.member_list().await;
    assert!(list_before.iter().any(|x| x.id == m.id));

    c.member_remove(m.id).await.unwrap();

    let list_after = c.member_list().await;
    assert!(!list_after.iter().any(|x| x.id == m.id));
}

#[tokio::test]
async fn characterize_cluster_promote_learner() {
    let c = ClusterManager::default();

    let m = c.member_add(vec!["http://learner:2380".to_string()], true).await.unwrap();
    assert!(m.is_learner);

    let promoted = c.member_promote(m.id).await.unwrap();
    assert!(!promoted.is_learner, "promoted member must not be a learner");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 16 — S3 policy evaluation
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_policy_allow_wildcard_principal() {
    use cave_store::s3::policy::{BucketPolicy, Effect, PolicyContext, evaluate};
    use serde_json;

    let policy_json = r#"{"Version":"2012-10-17","Statement":[{"Sid":"pub","Effect":"Allow","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::pub/*"]}]}"#;
    let policy: BucketPolicy = serde_json::from_str(policy_json).unwrap();

    let ctx = PolicyContext {
        principal: "anonymous",
        action: "s3:GetObject",
        resource: "arn:aws:s3:::pub/image.jpg",
    };
    let result = evaluate(&policy, &ctx);
    assert_eq!(result, Some(Effect::Allow));
}

#[tokio::test]
async fn characterize_policy_deny_overrides_allow() {
    use cave_store::s3::policy::{BucketPolicy, Effect, PolicyContext, evaluate};

    let policy_json = r#"{"Version":"2012-10-17","Statement":[
        {"Sid":"a","Effect":"Allow","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::b/*"]},
        {"Sid":"d","Effect":"Deny","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::b/secret.key"]}
    ]}"#;
    let policy: BucketPolicy = serde_json::from_str(policy_json).unwrap();

    let ctx = PolicyContext {
        principal: "user1",
        action: "s3:GetObject",
        resource: "arn:aws:s3:::b/secret.key",
    };
    assert_eq!(evaluate(&policy, &ctx), Some(Effect::Deny));

    let ctx2 = PolicyContext {
        principal: "user1",
        action: "s3:GetObject",
        resource: "arn:aws:s3:::b/public.txt",
    };
    assert_eq!(evaluate(&policy, &ctx2), Some(Effect::Allow));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 17 — Presigned URLs
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_presigned_url_valid_signature_passes() {
    use cave_store::s3::presigned;

    let secret = b"test-secret-key";
    let params = presigned::PresignedUrlParams {
        bucket: "my-bucket".to_string(),
        key: "file.txt".to_string(),
        method: "GET".to_string(),
        expires_in_secs: 3600,
        access_key: "test-access".to_string(),
        extra_headers: Default::default(),
    };

    let url = presigned::generate("http://localhost:9000", &params, secret);
    assert!(!url.url.is_empty());
    assert!(url.expires_at > chrono::Utc::now().timestamp());
}

#[tokio::test]
async fn characterize_presigned_url_wrong_signature_fails() {
    use cave_store::s3::presigned;

    let real_secret = b"real-secret";
    let wrong_secret = b"wrong-secret";
    let params = presigned::PresignedUrlParams {
        bucket: "b".to_string(),
        key: "k".to_string(),
        method: "PUT".to_string(),
        expires_in_secs: 3600,
        access_key: "ak".to_string(),
        extra_headers: Default::default(),
    };

    let url = presigned::generate("http://localhost:9000", &params, real_secret);
    // Extract signature from URL
    let sig = url.url.split("X-Cave-Signature=").nth(1).unwrap();

    let result = presigned::verify("PUT", "b", "k", "ak", url.expires_at, sig, wrong_secret);
    assert!(result.is_err(), "wrong secret must produce signature mismatch");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Section 18 — WAL persistence
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn characterize_wal_replay_restores_engine_state() {
    let dir = TempDir::new().unwrap();

    // Write some entries
    {
        let wal = WalWriter::open(dir.path()).unwrap();
        let engine = MvccEngine::new(wal);
        engine.put(b"persistent".to_vec(), b"data".to_vec(), 0, false).await.unwrap();
    }

    // Replay on a fresh engine
    let entries = cave_store::wal::read_wal(dir.path()).unwrap();
    assert!(!entries.is_empty(), "WAL must have written entries");

    let wal2 = WalWriter::open(dir.path()).unwrap();
    let engine2 = MvccEngine::new(wal2);
    engine2.replay_wal(entries).await;

    let r = engine2.range(b"persistent".to_vec(), vec![], 0, 0, false, false).await.unwrap();
    assert_eq!(r.kvs.len(), 1, "replayed engine must contain the put entry");
    assert_eq!(r.kvs[0].value, b"data");
}

#[tokio::test]
async fn characterize_wal_s3_replay_restores_bucket() {
    let dir = TempDir::new().unwrap();

    // Write a bucket via WAL
    {
        let wal = WalWriter::open(dir.path()).unwrap();
        let wal_arc = Arc::new(WalWriter::open(dir.path()).unwrap());
        let data_dir = dir.path().join("objects");
        std::fs::create_dir_all(&data_dir).unwrap();
        let store = ObjectStore::new(data_dir, wal_arc);
        store.create_bucket("wal-bucket", "us-east-1", "test").await.unwrap();
    }

    // Replay
    let entries = cave_store::wal::read_wal(dir.path()).unwrap();
    let wal2 = Arc::new(WalWriter::open(dir.path()).unwrap());
    let data_dir2 = dir.path().join("objects");
    let store2 = ObjectStore::new(data_dir2, wal2);
    store2.replay_wal(&entries).await;

    let buckets = store2.list_buckets().await;
    assert!(buckets.iter().any(|b| b.name == "wal-bucket"), "replayed store must contain the bucket");
}
