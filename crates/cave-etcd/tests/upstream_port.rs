// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream etcd tests, cross-referenced from
//! `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: etcd-io/etcd @ v3.6.10
//!   * server/storage/mvcc/kvstore_test.go
//!   * server/storage/mvcc/kv_test.go
//!   * server/etcdserver/txn/txn_test.go
//!   * server/lease/lessor_test.go
//!   * server/storage/wal/wal_test.go
//!
//! Each test below references its upstream Go test by file path +
//! function (Go t.Run subtest split into separate `#[test]` fns).

use cave_etcd::lease_id_gen::{LeaseIdGenerator, MAX_LEASE_ID_RETRIES};
use cave_etcd::models::{
    Compare, CompareResult, CompareTarget, DeleteRangeRequest, PutRequest, RangeRequest,
    RequestOp, TxnRequest,
};
use cave_etcd::store::KvStore;
use cave_etcd::wal::{Wal, WalOp};
use std::collections::HashSet;

fn put(key: &str, value: &str) -> PutRequest {
    PutRequest {
        key: key.into(),
        value: value.into(),
        lease: None,
        prev_kv: false,
    }
}

fn range(key: &str) -> RangeRequest {
    RangeRequest {
        key: key.into(),
        range_end: None,
        limit: None,
        revision: None,
        keys_only: false,
        count_only: false,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: server/storage/mvcc/kvstore_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestPut / `put_creates_key_with_version_1`.
#[test]
fn upstream_kv_put_creates_key_at_version_one() {
    let store = KvStore::new();
    let resp = store.put(&put("/a", "1"));
    assert!(resp.header.revision >= 1);
    let r = store.range(&range("/a")).unwrap();
    assert_eq!(r.kvs.len(), 1);
    assert_eq!(r.kvs[0].version, 1);
    assert_eq!(r.kvs[0].value_str(), "1");
}

/// Upstream: TestPut / `successive_puts_bump_version_and_mod_rev`.
#[test]
fn upstream_kv_put_bumps_version_and_mod_revision() {
    let store = KvStore::new();
    let _ = store.put(&put("/a", "1"));
    let _ = store.put(&put("/a", "2"));
    let r3 = store.put(&put("/a", "3"));
    let cur = store.range(&range("/a")).unwrap();
    assert_eq!(cur.kvs[0].version, 3);
    assert!(cur.kvs[0].mod_revision >= r3.header.revision);
}

/// Upstream: TestRange / `range_keys_within_window`.
#[test]
fn upstream_kv_range_returns_keys_in_window() {
    let store = KvStore::new();
    let _ = store.put(&put("/a", "1"));
    let _ = store.put(&put("/b", "2"));
    let _ = store.put(&put("/c", "3"));
    let mut req = range("/a");
    req.range_end = Some("/c".into());
    let r = store.range(&req).unwrap();
    let keys: Vec<String> = r.kvs.iter().map(|kv| kv.key_str()).collect();
    assert_eq!(keys, vec!["/a".to_string(), "/b".to_string()]);
    assert_eq!(r.count, 2);
}

/// Upstream: TestRange / `count_only_short_circuits_payload`.
#[test]
fn upstream_kv_range_count_only_returns_count_without_payload() {
    let store = KvStore::new();
    let _ = store.put(&put("/a", "1"));
    let _ = store.put(&put("/b", "2"));
    let mut req = range("/a");
    req.range_end = Some("/z".into());
    req.count_only = true;
    let r = store.range(&req).unwrap();
    assert_eq!(r.count, 2);
    assert!(r.kvs.is_empty(), "count_only must clear kvs");
}

/// Upstream: TestDeleteRange / `delete_returns_count_and_clears_key`.
#[test]
fn upstream_kv_delete_range_removes_key_and_returns_count() {
    let store = KvStore::new();
    let _ = store.put(&put("/a", "1"));
    let _ = store.put(&put("/b", "2"));
    let _ = store.put(&put("/c", "3"));
    let req = DeleteRangeRequest {
        key: "/a".into(),
        range_end: Some("/c".into()),
        prev_kv: false,
    };
    let resp = store.delete_range(&req);
    assert_eq!(resp.deleted, 2);
    let r = store.range(&range("/a")).unwrap();
    assert_eq!(r.kvs.len(), 0);
    let r = store.range(&range("/c")).unwrap();
    assert_eq!(r.kvs.len(), 1);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: server/etcdserver/txn/txn_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestTxnIfValueEqual / `compare_equal_value_runs_success_branch`.
#[test]
fn upstream_txn_compare_equal_value_runs_success_branch() {
    let store = KvStore::new();
    let _ = store.put(&put("/lock", "owner-1"));
    let txn = TxnRequest {
        compare: vec![Compare {
            key: "/lock".into(),
            target: CompareTarget::Value,
            result: CompareResult::Equal,
            value: Some("owner-1".into()),
            version: None,
            mod_revision: None,
        }],
        success: vec![RequestOp::Put(put("/state", "ok"))],
        failure: vec![RequestOp::Put(put("/state", "fail"))],
    };
    let resp = store.txn(&txn);
    assert!(resp.succeeded);
    let r = store.range(&range("/state")).unwrap();
    assert_eq!(r.kvs[0].value_str(), "ok");
}

/// Upstream: TestTxnIfValueEqual / `compare_unequal_runs_failure_branch`.
#[test]
fn upstream_txn_compare_unequal_runs_failure_branch() {
    let store = KvStore::new();
    let _ = store.put(&put("/lock", "owner-1"));
    let txn = TxnRequest {
        compare: vec![Compare {
            key: "/lock".into(),
            target: CompareTarget::Value,
            result: CompareResult::Equal,
            value: Some("nope".into()),
            version: None,
            mod_revision: None,
        }],
        success: vec![RequestOp::Put(put("/state", "ok"))],
        failure: vec![RequestOp::Put(put("/state", "fail"))],
    };
    let resp = store.txn(&txn);
    assert!(!resp.succeeded);
    let r = store.range(&range("/state")).unwrap();
    assert_eq!(r.kvs[0].value_str(), "fail");
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: server/lease/lessor_test.go (lease ID generator subtests)
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: lessor_test.go / `assignNewLeaseID_returns_positive_int64`.
#[test]
fn upstream_lease_id_gen_returns_positive_int63() {
    let g = LeaseIdGenerator::new(0xc0ffee);
    for _ in 0..50 {
        let id = g.next();
        assert!(id > 0, "lease IDs must be in positive int63 space");
    }
}

/// Upstream: lessor_test.go / `assignNewLeaseID_avoids_taken_ids`.
#[test]
fn upstream_lease_id_gen_allocate_skips_collisions() {
    let g = LeaseIdGenerator::new(0xface);
    let taken: HashSet<i64> = (1..=3).map(|_| g.next()).collect();
    let g = LeaseIdGenerator::new(0xface);
    let allocated = g.allocate(|c| taken.contains(&c)).unwrap();
    assert!(!taken.contains(&allocated));
}

/// Upstream: lessor_test.go / `assignNewLeaseID_caps_retries`.
#[test]
fn upstream_lease_id_gen_allocate_returns_err_when_retries_exhausted() {
    let g = LeaseIdGenerator::new(7);
    let err = g.allocate(|_| true);
    assert!(err.is_err());
    // Cave-specific: cap matches upstream's hard-coded 5.
    assert_eq!(MAX_LEASE_ID_RETRIES, 5);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: server/storage/wal/wal_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestNew + TestSave / `append entry assigns monotonic index`.
#[test]
fn upstream_wal_append_entry_assigns_monotonic_index() {
    let tmp = tempfile::tempdir().unwrap();
    let mut wal = Wal::open(tmp.path()).unwrap();
    let i1 = wal
        .append_entry(1, WalOp::Put {
            key: b"/a".to_vec(),
            value: b"1".to_vec(),
            lease: None,
        })
        .unwrap();
    let i2 = wal
        .append_entry(1, WalOp::Put {
            key: b"/b".to_vec(),
            value: b"2".to_vec(),
            lease: None,
        })
        .unwrap();
    assert_eq!(i1, 1);
    assert_eq!(i2, 2);
    assert_eq!(wal.last_entry_index(), 2);
}

/// Upstream: TestReopen / `re-open replays previously written entries`.
#[test]
fn upstream_wal_reopen_preserves_last_entry_index() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let mut wal = Wal::open(tmp.path()).unwrap();
        let _ = wal
            .append_entry(1, WalOp::Put {
                key: b"/k".to_vec(),
                value: b"v".to_vec(),
                lease: None,
            })
            .unwrap();
    }
    // Reopen — etcd's contract: replay restores last_entry_index.
    let wal = Wal::open(tmp.path()).unwrap();
    assert_eq!(wal.last_entry_index(), 1);
    let records = wal.replay().unwrap();
    assert!(!records.is_empty());
}

/// Upstream: TestRelease / `truncate_through removes entries up to index`.
#[test]
fn upstream_wal_truncate_through_drops_entries_up_to_index() {
    let tmp = tempfile::tempdir().unwrap();
    let mut wal = Wal::open(tmp.path()).unwrap();
    for i in 0..5 {
        let _ = wal
            .append_entry(1, WalOp::Put {
                key: format!("/k{i}").into_bytes(),
                value: vec![i],
                lease: None,
            })
            .unwrap();
    }
    wal.truncate_through(3).unwrap();
    let records = wal.replay().unwrap();
    let kept: Vec<u64> = records
        .iter()
        .filter_map(|r| match r {
            cave_etcd::wal::WalRecord::Entry(e) => Some(e.index),
            _ => None,
        })
        .collect();
    assert_eq!(kept, vec![4, 5], "entries 1..=3 must be truncated");
}
