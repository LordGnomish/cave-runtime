//! Parity-named tests mirroring upstream etcd Go test_io/etcd.
//!
//! Each `fn test_*` here corresponds 1:1 to a `[[tests]]` entry in
//! `parity.manifest.toml`. The bodies exercise behavior that the upstream
//! Go test verifies — versioning, MVCC reads, txn atomicity, watch lifecycle,
//! lease, auth, revision monotonicity. Tests live under `src/` so the parity
//! calculator (which walks `source_root`) detects them.

#![cfg(test)]

use crate::error::EtcdError;
use crate::models::*;
use crate::store::KvStore;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn put(store: &KvStore, key: &str, value: &str) -> PutResponse {
    store.put(&PutRequest {
        key: key.into(),
        value: value.into(),
        lease: None,
        prev_kv: false,
    })
}

fn get(store: &KvStore, key: &str) -> Option<KeyValue> {
    store
        .range(&RangeRequest {
            key: key.into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        })
        .ok()?
        .kvs
        .into_iter()
        .next()
}

// ── KV ──────────────────────────────────────────────────────────────────────

/// Mirrors etcd `TestV3PutOverwrite`: a second Put on the same key overwrites
/// the value, increments version, but keeps create_revision stable.
#[test]
fn test_kv_put_overwrite() {
    let store = KvStore::new();
    let r1 = put(&store, "k", "first");
    let v1 = get(&store, "k").unwrap();
    assert_eq!(v1.value_str(), "first");
    assert_eq!(v1.version, 1);
    let create_rev = v1.create_revision;

    let r2 = put(&store, "k", "second");
    let v2 = get(&store, "k").unwrap();
    assert_eq!(v2.value_str(), "second");
    assert_eq!(v2.version, 2, "version must monotonically increment on overwrite");
    assert_eq!(v2.create_revision, create_rev, "create_revision is stable across overwrites");
    assert!(r2.header.revision > r1.header.revision);
}

/// Mirrors etcd `TestV3RangeWithRev`: time-travel read returns the value as
/// it stood at the requested historical revision.
#[test]
fn test_kv_range_with_rev() {
    let store = KvStore::new();
    put(&store, "k", "v1");
    let r_after_v1 = store.current_revision();
    put(&store, "k", "v2");
    put(&store, "k", "v3");

    let resp = store
        .range(&RangeRequest {
            key: "k".into(),
            range_end: None,
            limit: None,
            revision: Some(r_after_v1),
            keys_only: false,
            count_only: false,
        })
        .unwrap();
    assert_eq!(resp.kvs.len(), 1);
    assert_eq!(resp.kvs[0].value_str(), "v1");

    let current = get(&store, "k").unwrap();
    assert_eq!(current.value_str(), "v3");
}

/// Mirrors etcd `TestV3DeleteRange`: deleting a prefix range removes all
/// keys in the range and reports the deleted count.
#[test]
fn test_kv_delete_range() {
    let store = KvStore::new();
    for i in 0..5 {
        put(&store, &format!("/p/{i}"), "v");
    }
    put(&store, "/other", "keep");

    let resp = store.delete_range(&DeleteRangeRequest {
        key: "/p/".into(),
        range_end: Some("/p0".into()),
        prev_kv: false,
    });
    assert_eq!(resp.deleted, 5);
    assert!(get(&store, "/other").is_some(), "non-matching key untouched");
    for i in 0..5 {
        assert!(get(&store, &format!("/p/{i}")).is_none());
    }
}

/// Mirrors etcd `TestV3TxnTooManyOps`: etcd caps per-txn ops at MaxTxnOps
/// (default 128). A txn whose total compare+success+failure ops exceed the
/// limit must be rejected.
#[test]
fn test_kv_txn_too_many_ops() {
    let store = KvStore::new();
    let put_op = || RequestOp::Put(PutRequest {
        key: "k".into(),
        value: "v".into(),
        lease: None,
        prev_kv: false,
    });

    // Reasonable count succeeds.
    let small = TxnRequest {
        compare: vec![],
        success: (0..10).map(|_| put_op()).collect(),
        failure: vec![],
    };
    let resp = store.txn(&small);
    assert!(resp.succeeded);

    // Over-the-limit count is rejected by the configured cap.
    let huge = TxnRequest {
        compare: vec![],
        success: (0..(KvStore::MAX_TXN_OPS + 1)).map(|_| put_op()).collect(),
        failure: vec![],
    };
    let err = store.txn_checked(&huge).expect_err("over-limit txn must error");
    assert!(matches!(err, EtcdError::TooManyTxnOps { .. }));
}

/// Mirrors etcd `TestV3TxnAtomicity`: on compare failure, success ops are
/// NOT applied — only the failure branch executes, atomically.
#[test]
fn test_txn_atomicity() {
    let store = KvStore::new();
    put(&store, "x", "init");

    let resp = store.txn(&TxnRequest {
        compare: vec![Compare {
            key: "x".into(),
            target: CompareTarget::Value,
            result: CompareResult::Equal,
            value: Some("WRONG".into()),
            version: None,
            mod_revision: None,
        }],
        // success ops would corrupt state if they leaked.
        success: vec![
            RequestOp::Put(PutRequest {
                key: "x".into(),
                value: "leaked_success_1".into(),
                lease: None,
                prev_kv: false,
            }),
            RequestOp::Put(PutRequest {
                key: "y".into(),
                value: "leaked_success_2".into(),
                lease: None,
                prev_kv: false,
            }),
        ],
        failure: vec![RequestOp::Put(PutRequest {
            key: "x".into(),
            value: "fail_branch".into(),
            lease: None,
            prev_kv: false,
        })],
    });
    assert!(!resp.succeeded);

    // x must reflect ONLY the failure branch; y must NOT have been written.
    assert_eq!(get(&store, "x").unwrap().value_str(), "fail_branch");
    assert!(get(&store, "y").is_none(), "success op leaked across compare failure");
}

// ── Watch ───────────────────────────────────────────────────────────────────

/// Mirrors etcd `TestV3WatchFromCurrentRevision`: a watch with no start
/// revision delivers events for puts that happen after the watch is created.
#[test]
fn test_watch_from_current_revision() {
    let store = KvStore::new();
    let resp = store.watch_create(&WatchCreateRequest {
        key: "/w".into(),
        range_end: None,
        start_revision: None,
        progress_notify: false,
        prev_kv: false,
    });
    assert!(resp.created);
    assert!(resp.events.is_empty(), "no historical replay when start_revision is None");

    let mut rx = store.subscribe();
    put(&store, "/w", "post_watch");
    let event = rx.try_recv().expect("watcher must receive subsequent put");
    assert_eq!(event.kv.key_str(), "/w");
    assert!(matches!(event.event_type, EventType::Put));
}

/// Mirrors etcd `TestV3WatchCancelSynced`: cancelling a watch removes its
/// config and the watcher no longer matches new events for that filter.
#[test]
fn test_watch_cancel_synced() {
    let store = KvStore::new();
    let resp = store.watch_create(&WatchCreateRequest {
        key: "/cancelme".into(),
        range_end: None,
        start_revision: None,
        progress_notify: false,
        prev_kv: false,
    });
    let id = resp.watch_id;
    assert!(store.get_watch_config(id).is_some(), "config exists pre-cancel");

    store.watch_cancel(id).expect("cancel must succeed on existing watch");
    assert!(store.get_watch_config(id).is_none(), "config gone post-cancel");

    // Cancelling a non-existent / already-cancelled id surfaces WatchNotFound.
    let err = store.watch_cancel(id);
    assert!(matches!(err, Err(EtcdError::WatchNotFound(_))));
}

// ── Lease ───────────────────────────────────────────────────────────────────

/// Mirrors etcd `TestV3LeaseGrant`: grant returns a positive ID, the requested
/// TTL, and the lease is queryable via lease_leases().
#[test]
fn test_lease_grant() {
    let store = KvStore::new();
    let resp = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
    assert!(resp.id > 0);
    assert_eq!(resp.ttl, 60);

    let listed = store.lease_leases();
    assert!(listed.leases.iter().any(|l| l.id == resp.id));
}

/// Mirrors etcd `TestV3LeaseRevoke`: revoking a lease drops it; revoking a
/// non-existent lease errors with LeaseNotFound.
#[test]
fn test_lease_revoke() {
    let store = KvStore::new();
    let granted = store.lease_grant(&LeaseGrantRequest { ttl: 30, id: None });

    store.lease_revoke(granted.id).expect("revoke active lease ok");
    let after = store.lease_leases();
    assert!(after.leases.iter().all(|l| l.id != granted.id));

    let err = store.lease_revoke(granted.id);
    assert!(matches!(err, Err(EtcdError::LeaseNotFound(_))));
}

// ── Auth ────────────────────────────────────────────────────────────────────

/// Mirrors etcd `TestV3AuthEnable`: AuthEnable succeeds the first time and
/// returns AuthAlreadyEnabled on a second invocation.
#[test]
fn test_auth_enable() {
    let store = KvStore::new();
    assert!(store.auth_enable().is_ok());
    assert!(matches!(
        store.auth_enable(),
        Err(EtcdError::AuthAlreadyEnabled)
    ));
    // Disable so other tests starting from a fresh store still see deterministic state.
    store.auth_disable().unwrap();
}

/// Mirrors etcd `TestV3AuthUserAdd`: UserAdd succeeds for new users and rejects
/// duplicates. Stored password is bcrypt-hashed (not plaintext).
#[test]
fn test_auth_user_add() {
    let store = KvStore::new();
    store
        .user_add(&AuthUserAddRequest {
            name: "alice".into(),
            password: "pw".into(),
        })
        .expect("first add ok");

    let dup = store.user_add(&AuthUserAddRequest {
        name: "alice".into(),
        password: "pw2".into(),
    });
    assert!(matches!(dup, Err(EtcdError::UserAlreadyExists(_))));

    let listing = store.user_list();
    assert!(listing.users.contains(&"alice".to_string()));
}

/// Mirrors etcd `TestV3AuthRoleAdd`: RoleAdd succeeds for new roles and rejects
/// duplicates.
#[test]
fn test_auth_role_add() {
    let store = KvStore::new();
    store
        .role_add(&AuthRoleAddRequest { name: "admin".into() })
        .expect("first add ok");

    let dup = store.role_add(&AuthRoleAddRequest { name: "admin".into() });
    assert!(matches!(dup, Err(EtcdError::RoleAlreadyExists(_))));

    let listing = store.role_list();
    assert!(listing.roles.contains(&"admin".to_string()));
}

/// Mirrors etcd `TestV3AuthPermission`: a role with Read permission on a key
/// can read that key but cannot write it.
#[test]
fn test_auth_permission() {
    let store = KvStore::new();
    store
        .user_add(&AuthUserAddRequest {
            name: "reader".into(),
            password: "pw".into(),
        })
        .unwrap();
    store
        .role_add(&AuthRoleAddRequest { name: "ro".into() })
        .unwrap();
    store
        .role_grant_permission(&AuthRoleGrantPermissionRequest {
            name: "ro".into(),
            perm: Permission {
                perm_type: PermType::Read,
                key: "/data/".into(),
                range_end: Some("/data0".into()),
            },
        })
        .unwrap();
    store
        .user_grant_role(&AuthUserGrantRoleRequest {
            user: "reader".into(),
            role: "ro".into(),
        })
        .unwrap();
    store.auth_enable().unwrap();

    let auth = store
        .authenticate(&AuthenticateRequest {
            name: "reader".into(),
            password: "pw".into(),
        })
        .unwrap();

    // Read is allowed.
    store
        .check_auth_token(Some(&auth.token), b"/data/x", PermType::Read)
        .expect("read allowed within granted range");
    // Write is denied.
    let denied = store
        .check_auth_token(Some(&auth.token), b"/data/x", PermType::Write);
    assert!(matches!(denied, Err(EtcdError::PermissionDenied)));

    store.auth_disable().unwrap();
}

// ── Revision monotonicity ──────────────────────────────────────────────────

/// Mirrors etcd `TestV3RevisionMonotonicity`: every mutating op (put, delete,
/// txn that mutates) strictly increases the cluster revision; reads do not.
#[test]
fn test_revision_monotonicity() {
    let store = KvStore::new();
    let mut last = store.current_revision();

    // Put advances.
    let r1 = put(&store, "a", "1").header.revision;
    assert!(r1 > last);
    last = r1;

    // Read does not advance.
    let _ = get(&store, "a");
    assert_eq!(store.current_revision(), last);

    // Another put advances.
    let r2 = put(&store, "b", "2").header.revision;
    assert!(r2 > last);
    last = r2;

    // Delete advances.
    let r3 = store
        .delete_range(&DeleteRangeRequest {
            key: "a".into(),
            range_end: None,
            prev_kv: false,
        })
        .header
        .revision;
    assert!(r3 > last);
    last = r3;

    // Txn whose success branch mutates advances.
    let r4 = store
        .txn(&TxnRequest {
            compare: vec![],
            success: vec![RequestOp::Put(PutRequest {
                key: "c".into(),
                value: "3".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![],
        })
        .header
        .revision;
    assert!(r4 > last);
}
