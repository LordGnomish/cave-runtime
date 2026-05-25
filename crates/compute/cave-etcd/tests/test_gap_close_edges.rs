// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge / failure / boundary coverage for cave-etcd — KvStore (KV/range/delete/
//! txn/lease/auth), error variants, base64 helpers, lease-id generator.

use cave_etcd::b64;
use cave_etcd::error::{EtcdError, EtcdResult};
use cave_etcd::lease_id_gen::{LeaseIdGenerator, MAX_LEASE_ID_RETRIES};
use cave_etcd::models::{
    AuthUserAddRequest, AuthUserDeleteRequest, AuthUserGetRequest, AuthenticateRequest, Compare,
    CompareResult, CompareTarget, DeleteRangeRequest, EventType, LeaseGrantRequest,
    LeaseKeepAliveRequest, LeaseTTLRequest, PutRequest, RangeRequest, RequestOp, TxnRequest,
    WatchCreateRequest,
};
use cave_etcd::store::KvStore;
use std::collections::HashSet;

fn put(store: &KvStore, key: &str, value: &str) {
    store.put(&PutRequest {
        key: key.into(),
        value: value.into(),
        lease: None,
        prev_kv: false,
    });
}

fn range(store: &KvStore, key: &str) -> Vec<(String, String)> {
    let r = store
        .range(&RangeRequest {
            key: key.into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        })
        .unwrap();
    r.kvs.into_iter().map(|kv| (kv.key_str(), kv.value_str())).collect()
}

// ---------------------------------------------------------------------------
// Base64 helpers
// ---------------------------------------------------------------------------

#[test]
fn b64_roundtrip_arbitrary_bytes() {
    for bytes in [
        b"".as_slice(),
        b"hello",
        b"\x00\x01\x02\xff",
        b"foo/bar/baz",
    ] {
        assert_eq!(b64::decode(&b64::encode(bytes)), bytes);
    }
}

#[test]
fn b64_decode_invalid_falls_back_to_plain_bytes() {
    // Invalid base64 → fall through to s.as_bytes()
    let plain = "not!!!valid$$$";
    assert_eq!(b64::decode(plain), plain.as_bytes());
}

#[test]
fn b64_decode_opt_some_and_none() {
    let some = Some("aGVsbG8=".to_string()); // "hello"
    assert_eq!(b64::decode_opt(&some), Some(b"hello".to_vec()));
    let none: Option<String> = None;
    assert!(b64::decode_opt(&none).is_none());
}

// ---------------------------------------------------------------------------
// EtcdError display formatting
// ---------------------------------------------------------------------------

#[test]
fn etcd_error_display_includes_context() {
    assert!(EtcdError::KeyNotFound("k".into()).to_string().contains("k"));
    assert!(EtcdError::LeaseNotFound(42).to_string().contains("42"));
    assert!(EtcdError::UserNotFound("alice".into()).to_string().contains("alice"));
    let s = EtcdError::RevisionCompacted { requested: 5, compacted: 10 }.to_string();
    assert!(s.contains("5") && s.contains("10"));
    let s = EtcdError::TooManyTxnOps { ops: 200, max: 128 }.to_string();
    assert!(s.contains("200") && s.contains("128"));
    let s = EtcdError::NotLeader { term: 7, leader: Some(2) }.to_string();
    assert!(s.contains("7"));
    assert!(s.contains("2"));
}

#[test]
fn etcd_error_quorum_lost_formats() {
    let e = EtcdError::QuorumLost { required: 3, healthy: 1 };
    let s = e.to_string();
    assert!(s.contains("required=3"));
    assert!(s.contains("healthy=1"));
}

#[test]
fn etcd_result_ok_err_alias() {
    fn ok() -> EtcdResult<i64> { Ok(1) }
    fn err() -> EtcdResult<i64> { Err(EtcdError::AuthNotEnabled) }
    assert_eq!(ok().unwrap(), 1);
    assert!(matches!(err(), Err(EtcdError::AuthNotEnabled)));
}

// ---------------------------------------------------------------------------
// LeaseIdGenerator
// ---------------------------------------------------------------------------

#[test]
fn lease_id_gen_next_always_positive_and_distinct() {
    let g = LeaseIdGenerator::new(0xdeadbeef);
    let mut seen = HashSet::new();
    for _ in 0..500 {
        let id = g.next();
        assert!(id > 0);
        seen.insert(id);
    }
    assert!(seen.len() >= 495, "expected ~no collisions in 63-bit space");
}

#[test]
fn lease_id_gen_with_same_seed_is_deterministic() {
    let a = LeaseIdGenerator::new(7);
    let b = LeaseIdGenerator::new(7);
    let seq_a: Vec<i64> = (0..10).map(|_| a.next()).collect();
    let seq_b: Vec<i64> = (0..10).map(|_| b.next()).collect();
    assert_eq!(seq_a, seq_b);
}

#[test]
fn lease_id_gen_allocate_when_all_taken_errors() {
    let g = LeaseIdGenerator::new(0xc0ffee);
    let err = g.allocate(|_| true);
    assert!(err.is_err());
}

#[test]
fn lease_id_gen_max_retries_constant_documented() {
    // The contract guarantees MAX_LEASE_ID_RETRIES is small and constant.
    assert!(MAX_LEASE_ID_RETRIES >= 1);
    assert!(MAX_LEASE_ID_RETRIES <= 32);
}

#[test]
fn lease_id_gen_enqueue_fixed_returns_in_lifo_order() {
    let g = LeaseIdGenerator::new(11);
    g.enqueue_fixed(1);
    g.enqueue_fixed(2);
    g.enqueue_fixed(3);
    // Implementation is `Vec::pop` (LIFO)
    assert_eq!(g.next(), 3);
    assert_eq!(g.next(), 2);
    assert_eq!(g.next(), 1);
}

// ---------------------------------------------------------------------------
// KvStore — KV semantics
// ---------------------------------------------------------------------------

#[test]
fn store_put_then_range_returns_value() {
    let s = KvStore::new();
    put(&s, "foo", "bar");
    let r = range(&s, "foo");
    assert_eq!(r, vec![("foo".to_string(), "bar".to_string())]);
}

#[test]
fn store_put_overwrites_value_and_bumps_version() {
    let s = KvStore::new();
    put(&s, "k", "v1");
    let rev_before = s.current_revision();
    put(&s, "k", "v2");
    assert!(s.current_revision() > rev_before);
    let kv = &s
        .range(&RangeRequest {
            key: "k".into(),
            range_end: None, limit: None, revision: None,
            keys_only: false, count_only: false,
        }).unwrap().kvs[0];
    assert_eq!(kv.value_str(), "v2");
    assert_eq!(kv.version, 2);
}

#[test]
fn store_range_unknown_key_returns_empty() {
    let s = KvStore::new();
    let r = range(&s, "missing");
    assert!(r.is_empty());
}

#[test]
fn store_range_count_only_returns_zero_kvs_with_count() {
    let s = KvStore::new();
    for k in &["a", "b", "c"] {
        put(&s, k, "v");
    }
    let resp = s.range(&RangeRequest {
        key: "a".into(),
        range_end: Some("z".into()),
        limit: None, revision: None,
        keys_only: false, count_only: true,
    }).unwrap();
    assert_eq!(resp.count, 3);
    assert!(resp.kvs.is_empty());
}

#[test]
fn store_range_limit_sets_more_when_truncated() {
    let s = KvStore::new();
    for k in &["a", "b", "c", "d", "e"] {
        put(&s, k, "v");
    }
    let resp = s.range(&RangeRequest {
        key: "a".into(),
        range_end: Some("z".into()),
        limit: Some(2),
        revision: None,
        keys_only: false, count_only: false,
    }).unwrap();
    assert_eq!(resp.kvs.len(), 2);
    assert!(resp.more);
}

#[test]
fn store_range_keys_only_strips_values() {
    let s = KvStore::new();
    put(&s, "k", "secret");
    let resp = s.range(&RangeRequest {
        key: "k".into(),
        range_end: None,
        limit: None, revision: None,
        keys_only: true, count_only: false,
    }).unwrap();
    assert_eq!(resp.kvs.len(), 1);
    assert!(resp.kvs[0].value.is_empty(), "keys_only must clear values");
}

#[test]
fn store_range_with_compacted_revision_errors() {
    let s = KvStore::new();
    put(&s, "k", "v1");
    put(&s, "k", "v2");
    let _ = s.compaction(&cave_etcd::models::CompactionRequest { revision: s.current_revision(), physical: false });
    // Now request a revision below the compaction watermark.
    let resp = s.range(&RangeRequest {
        key: "k".into(),
        range_end: None, limit: None,
        revision: Some(1),
        keys_only: false, count_only: false,
    });
    assert!(matches!(resp, Err(EtcdError::RevisionCompacted { .. })));
}

#[test]
fn store_delete_range_removes_keys_and_returns_count() {
    let s = KvStore::new();
    for k in &["a", "b", "c"] {
        put(&s, k, "v");
    }
    let resp = s.delete_range(&DeleteRangeRequest {
        key: "a".into(),
        range_end: Some("z".into()),
        prev_kv: false,
    });
    assert_eq!(resp.deleted, 3);
    assert!(range(&s, "a").is_empty());
}

#[test]
fn store_delete_unknown_returns_zero() {
    let s = KvStore::new();
    let resp = s.delete_range(&DeleteRangeRequest {
        key: "absent".into(),
        range_end: None,
        prev_kv: false,
    });
    assert_eq!(resp.deleted, 0);
}

#[test]
fn store_delete_range_with_prev_kv_returns_old_values() {
    let s = KvStore::new();
    put(&s, "k", "v");
    let resp = s.delete_range(&DeleteRangeRequest {
        key: "k".into(),
        range_end: None,
        prev_kv: true,
    });
    assert_eq!(resp.prev_kvs.len(), 1);
    assert_eq!(resp.prev_kvs[0].value_str(), "v");
}

// ---------------------------------------------------------------------------
// Transactions — compare-and-swap
// ---------------------------------------------------------------------------

#[test]
fn store_txn_compare_value_equal_takes_success_branch() {
    let s = KvStore::new();
    put(&s, "k", "v");
    let resp = s.txn(&TxnRequest {
        compare: vec![Compare {
            key: "k".into(),
            target: CompareTarget::Value,
            result: CompareResult::Equal,
            value: Some("v".into()),
            version: None,
            mod_revision: None,
        }],
        success: vec![RequestOp::Put(PutRequest {
            key: "k".into(), value: "new".into(), lease: None, prev_kv: false,
        })],
        failure: vec![],
    });
    assert!(resp.succeeded);
    assert_eq!(range(&s, "k")[0].1, "new");
}

#[test]
fn store_txn_compare_value_not_equal_takes_failure_branch() {
    let s = KvStore::new();
    put(&s, "k", "actual");
    let resp = s.txn(&TxnRequest {
        compare: vec![Compare {
            key: "k".into(),
            target: CompareTarget::Value,
            result: CompareResult::Equal,
            value: Some("expected".into()),
            version: None,
            mod_revision: None,
        }],
        success: vec![RequestOp::Put(PutRequest {
            key: "k".into(), value: "won".into(), lease: None, prev_kv: false,
        })],
        failure: vec![RequestOp::Put(PutRequest {
            key: "marker".into(), value: "fallback".into(), lease: None, prev_kv: false,
        })],
    });
    assert!(!resp.succeeded);
    assert_eq!(range(&s, "marker")[0].1, "fallback");
    assert_eq!(range(&s, "k")[0].1, "actual");
}

#[test]
fn store_txn_version_zero_means_key_does_not_exist() {
    let s = KvStore::new();
    // Compare version == 0 → key absent (etcd convention)
    let resp = s.txn(&TxnRequest {
        compare: vec![Compare {
            key: "fresh".into(),
            target: CompareTarget::Version,
            result: CompareResult::Equal,
            value: None,
            version: Some(0),
            mod_revision: None,
        }],
        success: vec![RequestOp::Put(PutRequest {
            key: "fresh".into(), value: "new-key".into(), lease: None, prev_kv: false,
        })],
        failure: vec![],
    });
    assert!(resp.succeeded);
    assert_eq!(range(&s, "fresh")[0].1, "new-key");
}

#[test]
fn store_txn_checked_rejects_excessive_ops() {
    let s = KvStore::new();
    let many: Vec<RequestOp> = (0..(KvStore::MAX_TXN_OPS + 50))
        .map(|i| RequestOp::Put(PutRequest {
            key: format!("k{}", i), value: "v".into(), lease: None, prev_kv: false,
        }))
        .collect();
    let resp = s.txn_checked(&TxnRequest {
        compare: vec![],
        success: many,
        failure: vec![],
    });
    assert!(matches!(resp, Err(EtcdError::TooManyTxnOps { .. })));
}

// ---------------------------------------------------------------------------
// Watch
// ---------------------------------------------------------------------------

#[test]
fn watch_create_returns_unique_watch_ids() {
    let s = KvStore::new();
    let r1 = s.watch_create(&WatchCreateRequest {
        key: "a".into(), range_end: None,
        start_revision: None, progress_notify: false, prev_kv: false,
    });
    let r2 = s.watch_create(&WatchCreateRequest {
        key: "a".into(), range_end: None,
        start_revision: None, progress_notify: false, prev_kv: false,
    });
    assert!(r1.watch_id != r2.watch_id);
    assert!(r1.created && r2.created);
}

#[test]
fn watch_config_lookup_returns_some_after_create() {
    let s = KvStore::new();
    let r = s.watch_create(&WatchCreateRequest {
        key: "/foo".into(), range_end: Some("/foo0".into()),
        start_revision: None, progress_notify: false, prev_kv: true,
    });
    let cfg = s.get_watch_config(r.watch_id).expect("config must exist");
    assert!(cfg.prev_kv);
    assert_eq!(cfg.key, b"/foo".to_vec());
}

#[test]
fn watch_key_matches_pattern_helpers() {
    use cave_etcd::models::WatchConfig;
    use cave_etcd::store::KvStore;
    // Single-key watch
    let single = WatchConfig {
        watch_id: 1,
        key: b"x".to_vec(),
        range_end: None,
        start_revision: None,
        prev_kv: false,
    };
    assert!(KvStore::key_matches_watch(b"x", &single));
    assert!(!KvStore::key_matches_watch(b"xy", &single));
    // Range watch
    let range = WatchConfig {
        watch_id: 2,
        key: b"a".to_vec(),
        range_end: Some(b"c".to_vec()),
        start_revision: None,
        prev_kv: false,
    };
    assert!(KvStore::key_matches_watch(b"a", &range));
    assert!(KvStore::key_matches_watch(b"b", &range));
    assert!(!KvStore::key_matches_watch(b"c", &range)); // exclusive
}

// ---------------------------------------------------------------------------
// Leases
// ---------------------------------------------------------------------------

#[test]
fn lease_grant_assigns_id_or_uses_requested() {
    let s = KvStore::new();
    let r1 = s.lease_grant(&LeaseGrantRequest { ttl: 30, id: None });
    assert!(r1.id > 0);
    assert_eq!(r1.ttl, 30);
    let r2 = s.lease_grant(&LeaseGrantRequest { ttl: 60, id: Some(424242) });
    assert_eq!(r2.id, 424242);
}

#[test]
fn lease_revoke_unknown_lease_errors() {
    let s = KvStore::new();
    let res = s.lease_revoke(999_999);
    assert!(matches!(res, Err(EtcdError::LeaseNotFound(999_999))));
}

#[test]
fn lease_keepalive_refreshes_known_lease() {
    let s = KvStore::new();
    let grant = s.lease_grant(&LeaseGrantRequest { ttl: 100, id: None });
    let resp = s.lease_keepalive(&LeaseKeepAliveRequest { id: grant.id }).unwrap();
    assert_eq!(resp.id, grant.id);
    assert_eq!(resp.ttl, 100);
}

#[test]
fn lease_keepalive_unknown_lease_errors() {
    let s = KvStore::new();
    let res = s.lease_keepalive(&LeaseKeepAliveRequest { id: 12345 });
    assert!(matches!(res, Err(EtcdError::LeaseNotFound(12345))));
}

#[test]
fn lease_timetolive_returns_granted_ttl() {
    let s = KvStore::new();
    let grant = s.lease_grant(&LeaseGrantRequest { ttl: 42, id: None });
    let resp = s.lease_timetolive(&LeaseTTLRequest {
        id: grant.id, keys: false,
    }).unwrap();
    assert_eq!(resp.id, grant.id);
    assert_eq!(resp.granted_ttl, 42);
    assert!(resp.ttl <= 42 && resp.ttl >= 0);
}

#[test]
fn lease_leases_lists_active() {
    let s = KvStore::new();
    for _ in 0..3 {
        s.lease_grant(&LeaseGrantRequest { ttl: 30, id: None });
    }
    let resp = s.lease_leases();
    assert_eq!(resp.leases.len(), 3);
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[test]
fn auth_enable_disable_state_machine() {
    let s = KvStore::new();
    assert!(s.auth_enable().is_ok());
    assert!(matches!(s.auth_enable(), Err(EtcdError::AuthAlreadyEnabled)));
    assert!(s.auth_disable().is_ok());
    assert!(matches!(s.auth_disable(), Err(EtcdError::AuthNotEnabled)));
}

#[test]
fn auth_user_add_duplicate_errors() {
    let s = KvStore::new();
    s.user_add(&AuthUserAddRequest { name: "alice".into(), password: "pw".into() }).unwrap();
    let res = s.user_add(&AuthUserAddRequest { name: "alice".into(), password: "pw".into() });
    assert!(matches!(res, Err(EtcdError::UserAlreadyExists(_))));
}

#[test]
fn auth_user_get_unknown_errors() {
    let s = KvStore::new();
    let res = s.user_get(&AuthUserGetRequest { name: "ghost".into() });
    assert!(matches!(res, Err(EtcdError::UserNotFound(_))));
}

#[test]
fn auth_user_delete_unknown_errors() {
    let s = KvStore::new();
    let res = s.user_delete(&AuthUserDeleteRequest { name: "ghost".into() });
    assert!(matches!(res, Err(EtcdError::UserNotFound(_))));
}

#[test]
fn auth_user_list_sorted() {
    let s = KvStore::new();
    for name in &["charlie", "alice", "bob"] {
        s.user_add(&AuthUserAddRequest { name: (*name).into(), password: "x".into() }).unwrap();
    }
    let resp = s.user_list();
    assert_eq!(resp.users, vec!["alice", "bob", "charlie"]);
}

#[test]
fn auth_authenticate_without_enable_issues_token() {
    let s = KvStore::new();
    let resp = s.authenticate(&AuthenticateRequest {
        name: "anon".into(), password: "any".into(),
    }).unwrap();
    assert!(!resp.token.is_empty());
}

#[test]
fn auth_authenticate_after_enable_requires_user() {
    let s = KvStore::new();
    s.auth_enable().unwrap();
    let res = s.authenticate(&AuthenticateRequest {
        name: "ghost".into(), password: "x".into(),
    });
    assert!(matches!(res, Err(EtcdError::UserNotFound(_))));
}

// ---------------------------------------------------------------------------
// Compaction
// ---------------------------------------------------------------------------

#[test]
fn compaction_updates_watermark() {
    let s = KvStore::new();
    put(&s, "k", "v1");
    put(&s, "k", "v2");
    let target = s.current_revision();
    let _ = s.compaction(&cave_etcd::models::CompactionRequest { revision: target, physical: false });
    // After compaction, an older-revision read fails.
    let res = s.range(&RangeRequest {
        key: "k".into(),
        range_end: None,
        limit: None,
        revision: Some(1),
        keys_only: false, count_only: false,
    });
    assert!(matches!(res, Err(EtcdError::RevisionCompacted { .. })));
}

// ---------------------------------------------------------------------------
// KvStore::current_revision monotonic invariant
// ---------------------------------------------------------------------------

#[test]
fn store_revision_only_grows() {
    let s = KvStore::new();
    let mut last = s.current_revision();
    for i in 0..20 {
        put(&s, &format!("k{}", i), "v");
        let now = s.current_revision();
        assert!(now > last);
        last = now;
    }
}

#[test]
fn store_event_type_serialization() {
    // Ensures EventType encodes/decodes via serde (used over the wire)
    let put_str = serde_json::to_string(&EventType::Put).unwrap();
    let del_str = serde_json::to_string(&EventType::Delete).unwrap();
    assert!(put_str.contains("Put"));
    assert!(del_str.contains("Delete"));
}
