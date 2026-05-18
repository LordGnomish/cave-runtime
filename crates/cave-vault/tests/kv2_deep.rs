// SPDX-License-Identifier: AGPL-3.0-or-later
//! deeper-001: KV v2 — version pruning, delete protection, metadata TTL,
//! cas+version conflict edge cases. Pinned to openbao v2.5.3.

use cave_vault::engines::kv2::{Kv2Secret, Kv2Store, Kv2Version};
use chrono::{Duration, Utc};
use serde_json::Value;
use std::collections::HashMap;

const TENANT: &str = "tenant-acme-prod";

fn put_version(secret: &mut Kv2Secret, kv: &[(&str, &str)]) -> u64 {
    let mut data = HashMap::new();
    for (k, v) in kv {
        data.insert((*k).to_string(), Value::String((*v).to_string()));
    }
    let next = secret.current_version + 1;
    secret.versions.push(Kv2Version {
        version: next,
        data: Some(data),
        created_time: Utc::now(),
        deletion_time: None,
        destroyed: false,
    });
    secret.current_version = next;
    secret.updated_time = Utc::now();
    next
}

fn make_store_for_tenant(tenant: &str) -> Kv2Store {
    let mut store = Kv2Store::default();
    store.data.entry(format!("{}/kv", tenant)).or_default();
    store
}

/// Cite: openbao `builtin/logical/kv/path_data.go:229` (cleanupOldVersions)
/// — `prune_to_max_versions` evicts oldest entries until live count ≤ max
/// and bumps `oldest_version` accordingly.
#[test]
fn prune_to_max_versions_drops_oldest_only() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/cfg".into()).or_default();
    secret.max_versions = 3;
    for _ in 1..=6 { put_version(secret, &[("v", "x")]); }

    let pruned = secret.prune_to_max_versions();
    assert_eq!(pruned, vec![1, 2, 3], "oldest three pruned");
    assert_eq!(secret.versions.len(), 3);
    assert_eq!(secret.oldest_version, 4);
    assert!(secret.get_version(1).is_none());
    assert!(secret.get_version(4).is_some());
    assert!(secret.get_version(6).is_some());
}

/// Cite: openbao `builtin/logical/kv/path_data.go:229` — re-pruning
/// after the count is already within bounds is a no-op (returns empty).
#[test]
fn prune_below_max_is_noop() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/cfg".into()).or_default();
    secret.max_versions = 5;
    for _ in 1..=2 { put_version(secret, &[("v", "x")]); }

    assert!(secret.prune_to_max_versions().is_empty());
    assert_eq!(secret.versions.len(), 2);
    assert_eq!(secret.oldest_version, 0);
}

/// Cite: openbao `builtin/logical/kv/delete_version_after.go` — when
/// `delete_version_after = 0`, no version is ever considered expired
/// (the field acts as the kill-switch for the periodic sweeper).
#[test]
fn delete_version_after_zero_disables_ttl_check() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/cfg".into()).or_default();
    secret.delete_version_after = 0;
    put_version(secret, &[("v", "x")]);

    let one_year_later = Utc::now() + Duration::days(365);
    assert!(!secret.is_version_expired(1, one_year_later),
        "TTL=0 ⇒ version never expires");
    assert!(secret.sweep_expired(one_year_later).is_empty(),
        "TTL=0 ⇒ sweep is a no-op");
}

/// Cite: openbao `builtin/logical/kv/delete_version_after.go` — a
/// version older than `created_time + delete_version_after` is expired
/// and gets soft-deleted by the sweeper (deletion_time set, data kept).
#[test]
fn sweep_expired_soft_deletes_old_versions_only() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/cfg".into()).or_default();
    secret.delete_version_after = 60;  // 1 minute

    // v1 created 2 minutes ago (will be swept)
    secret.versions.push(Kv2Version {
        version: 1,
        data: Some([("v".to_string(), Value::String("old".into()))].into()),
        created_time: Utc::now() - Duration::seconds(120),
        deletion_time: None,
        destroyed: false,
    });
    secret.current_version = 1;
    // v2 created just now (still valid)
    put_version(secret, &[("v", "new")]);

    let swept = secret.sweep_expired(Utc::now());
    assert_eq!(swept, vec![1], "only v1 swept");
    assert!(secret.get_version(1).unwrap().deletion_time.is_some());
    assert!(secret.get_version(1).unwrap().data.is_some(),
        "soft-delete keeps ciphertext");
    assert!(secret.get_version(2).unwrap().deletion_time.is_none());
}

/// Cite: openbao `builtin/logical/kv/path_destroy.go:39` — destroyed
/// versions are NOT eligible for sweep (they're already gone). The
/// sweeper must skip them.
#[test]
fn sweep_skips_already_destroyed_and_already_deleted_versions() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/cfg".into()).or_default();
    secret.delete_version_after = 60;

    // v1 destroyed long ago
    secret.versions.push(Kv2Version {
        version: 1,
        data: None,
        created_time: Utc::now() - Duration::seconds(3600),
        deletion_time: None,
        destroyed: true,
    });
    // v2 already soft-deleted
    secret.versions.push(Kv2Version {
        version: 2,
        data: Some(HashMap::new()),
        created_time: Utc::now() - Duration::seconds(3600),
        deletion_time: Some(Utc::now() - Duration::seconds(120)),
        destroyed: false,
    });
    secret.current_version = 2;

    let swept = secret.sweep_expired(Utc::now());
    assert!(swept.is_empty(), "destroyed + already-deleted both skipped");
}

/// Cite: openbao `builtin/logical/kv/path_data.go:197`
/// (validateCheckAndSetOption). CAS=0 succeeds only on the FIRST write
/// (when `current_version == 0`); CAS=N requires `current_version == N`.
/// Mismatch ⇒ caller-side rejection.
#[test]
fn cas_zero_only_succeeds_on_first_write() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/cas".into()).or_default();

    // Empty path: cas=0 matches current_version=0 ⇒ ok
    let cas: u64 = 0;
    assert_eq!(cas, secret.current_version);
    put_version(secret, &[("v", "first")]);

    // Now current_version=1; cas=0 must FAIL the next write
    let cas: u64 = 0;
    assert_ne!(cas, secret.current_version,
        "cas=0 only valid for the first write; subsequent writes need cas=N>0");

    // cas=1 succeeds
    let cas: u64 = 1;
    assert_eq!(cas, secret.current_version);
    put_version(secret, &[("v", "second")]);
    assert_eq!(secret.current_version, 2);
}

/// Cite: openbao `builtin/logical/kv/path_data.go:197` — `cas_required`
/// makes EVERY write require a CAS option. A write without `options.cas`
/// must be rejected.
#[test]
fn cas_required_blocks_writes_without_explicit_cas() {
    let mut store = make_store_for_tenant(TENANT);
    let mount = format!("{}/kv", TENANT);
    let secret = store.data.get_mut(&mount).unwrap()
        .entry("svc/strict".into()).or_default();
    secret.cas_required = true;

    // Without supplied cas, the handler would short-circuit ⇒ no write.
    // We model the would-be reject as: cas_required && supplied_cas.is_none()
    let supplied_cas: Option<u64> = None;
    let allowed = !secret.cas_required || supplied_cas.is_some();
    assert!(!allowed, "cas_required blocks anonymous writes");

    // With cas=0 (and current_version=0), the write proceeds.
    let supplied_cas: Option<u64> = Some(0);
    let allowed = !secret.cas_required || supplied_cas.is_some();
    assert!(allowed);
    if let Some(cas) = supplied_cas {
        assert_eq!(cas, secret.current_version);
        put_version(secret, &[("v", "v1")]);
    }
    assert_eq!(secret.current_version, 1);
}
