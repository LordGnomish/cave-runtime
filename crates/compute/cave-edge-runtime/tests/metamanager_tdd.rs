// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the offline-first MetaManager.
//!
//! Faithful port of KubeEdge `edge/pkg/metamanager` semantics:
//!   - the metaDB local cache (Insert / Update / Delete / Query operations)
//!   - cache-through on a query miss while the cloud link is up (the request
//!     is forwarded, then the response is stored locally)
//!   - serve-from-cache while offline (no cloud round-trip)
//!   - resource-version monotonicity (a stale update is rejected)
//!   - list-by-type (prefix) query
//!
//! Pure store logic — no SQLite, no network.

use cave_edge_runtime::metamanager::{MetaManager, QueryOutcome};

#[test]
fn insert_then_query_is_a_local_hit() {
    let mut m = MetaManager::new();
    m.insert("pod/default/web", "spec-v1", 10);
    assert_eq!(
        m.query("pod/default/web"),
        QueryOutcome::Hit("spec-v1".to_string())
    );
}

#[test]
fn query_miss_while_online_signals_forward_to_cloud() {
    let mut m = MetaManager::new();
    m.set_online(true);
    assert_eq!(m.query("pod/default/missing"), QueryOutcome::ForwardToCloud);
}

#[test]
fn query_miss_while_offline_is_not_found() {
    let mut m = MetaManager::new();
    m.set_online(false);
    assert_eq!(m.query("pod/default/missing"), QueryOutcome::NotFound);
}

#[test]
fn cloud_response_is_cached_and_then_served_locally() {
    let mut m = MetaManager::new();
    m.set_online(true);
    assert_eq!(m.query("cm/default/cfg"), QueryOutcome::ForwardToCloud);
    // The hub hands the cloud's response back; metamanager caches it.
    m.cache_cloud_response("cm/default/cfg", "data-v1", 7);
    assert_eq!(
        m.query("cm/default/cfg"),
        QueryOutcome::Hit("data-v1".to_string())
    );
}

#[test]
fn offline_serves_previously_cached_value() {
    let mut m = MetaManager::new();
    m.set_online(true);
    m.cache_cloud_response("secret/default/tok", "s3cr3t", 3);
    m.set_online(false);
    // Cloud is gone, but the cached copy still answers.
    assert_eq!(
        m.query("secret/default/tok"),
        QueryOutcome::Hit("s3cr3t".to_string())
    );
}

#[test]
fn update_with_newer_resource_version_applies() {
    let mut m = MetaManager::new();
    m.insert("pod/default/web", "spec-v1", 10);
    assert!(m.update("pod/default/web", "spec-v2", 12));
    assert_eq!(
        m.query("pod/default/web"),
        QueryOutcome::Hit("spec-v2".to_string())
    );
    assert_eq!(m.resource_version("pod/default/web"), Some(12));
}

#[test]
fn update_with_stale_resource_version_is_rejected() {
    let mut m = MetaManager::new();
    m.insert("pod/default/web", "spec-v2", 12);
    // A late-arriving older write must not clobber the newer cached spec.
    assert!(!m.update("pod/default/web", "spec-v1", 10));
    assert_eq!(
        m.query("pod/default/web"),
        QueryOutcome::Hit("spec-v2".to_string())
    );
    assert_eq!(m.resource_version("pod/default/web"), Some(12));
}

#[test]
fn delete_removes_local_record() {
    let mut m = MetaManager::new();
    m.insert("pod/default/web", "spec-v1", 10);
    m.delete("pod/default/web");
    m.set_online(false);
    assert_eq!(m.query("pod/default/web"), QueryOutcome::NotFound);
}

#[test]
fn list_by_type_returns_all_matching_keys_sorted() {
    let mut m = MetaManager::new();
    m.insert("pod/default/b", "2", 1);
    m.insert("pod/default/a", "1", 1);
    m.insert("cm/default/x", "9", 1);
    let pods = m.list_by_prefix("pod/");
    assert_eq!(
        pods,
        vec![
            ("pod/default/a".to_string(), "1".to_string()),
            ("pod/default/b".to_string(), "2".to_string()),
        ]
    );
}
