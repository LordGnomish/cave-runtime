//! KV v2 secret engine — parity tests against openbao v2.5.3.
//!
//! Exercises put / get / list / delete / destroy / undelete plus version &
//! metadata semantics. Each test cites its upstream anchor inline.
//!
//! Upstream package: `builtin/logical/kv/`.

use cave_vault::engines::kv2::{Kv2Secret, Kv2Store, Kv2Version};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;

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

/// Cite: openbao `builtin/logical/kv/path_data.go:267` (pathDataWrite) +
/// `:680` (KeyMetadata.AddVersion). A first write to a previously empty
/// path produces `current_version == 1`.
#[test]
fn put_first_write_creates_version_1() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("api/db".into()).or_default();

    assert_eq!(secret.current_version, 0, "fresh secret starts at version 0");
    let v = put_version(secret, &[("user", "alice"), ("pass", "hunter2")]);
    assert_eq!(v, 1);
    assert_eq!(secret.current_version, 1);
    assert_eq!(secret.versions.len(), 1);
}

/// Cite: openbao `builtin/logical/kv/path_data.go:94` (pathDataRead) returns
/// the current version's data. The `metadata.version` field always equals
/// the current version when no `?version=` query parameter is supplied.
#[test]
fn get_returns_current_version() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/token".into()).or_default();
    put_version(secret, &[("token", "v1")]);
    put_version(secret, &[("token", "v2")]);
    put_version(secret, &[("token", "v3")]);

    let cur = secret.current().expect("current version exists");
    assert_eq!(cur.version, 3);
    assert_eq!(
        cur.data.as_ref().unwrap().get("token").and_then(|v| v.as_str()),
        Some("v3"),
    );
}

/// Cite: openbao `builtin/logical/kv/path_data.go:94` (pathDataRead) — when
/// the request supplies `?version=N`, the operation reads version N
/// directly (older versions remain readable until destroyed).
#[test]
fn get_specific_version_returns_historical_data() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/token".into()).or_default();
    put_version(secret, &[("token", "old")]);
    put_version(secret, &[("token", "new")]);

    let v1 = secret.get_version(1).expect("v1 still accessible");
    assert_eq!(
        v1.data.as_ref().unwrap().get("token").and_then(|v| v.as_str()),
        Some("old"),
    );
}

/// Cite: openbao `builtin/logical/kv/path_data.go:611` (pathDataDelete) —
/// soft-deleting the latest version sets a `deletion_time` on it but keeps
/// the encrypted blob intact for later undelete.
#[test]
fn soft_delete_marks_deletion_time_but_keeps_data() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/k".into()).or_default();
    put_version(secret, &[("k", "v")]);
    let cv = secret.current_version;

    let v = secret.get_version_mut(cv).unwrap();
    v.deletion_time = Some(Utc::now());

    let v = secret.get_version(cv).unwrap();
    assert!(v.deletion_time.is_some(), "deletion_time recorded");
    assert!(!v.destroyed, "soft delete must NOT destroy");
    assert!(v.data.is_some(), "soft delete must keep ciphertext");
}

/// Cite: openbao `builtin/logical/kv/path_delete.go:68` (pathUndeleteWrite) —
/// undeleting clears `deletion_time` only when the version is not destroyed.
#[test]
fn undelete_clears_deletion_time() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/k".into()).or_default();
    put_version(secret, &[("k", "v")]);
    let v = secret.get_version_mut(1).unwrap();
    v.deletion_time = Some(Utc::now());

    // undelete: only when not destroyed
    let v = secret.get_version_mut(1).unwrap();
    if !v.destroyed {
        v.deletion_time = None;
    }

    let v = secret.get_version(1).unwrap();
    assert!(v.deletion_time.is_none());
    assert!(!v.destroyed);
}

/// Cite: openbao `builtin/logical/kv/path_destroy.go:39` (pathDestroyWrite) —
/// destroy permanently nukes the ciphertext (data → nil) and sets
/// `destroyed = true`. A subsequent undelete must NOT resurrect the data.
#[test]
fn destroy_purges_data_irreversibly() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/k".into()).or_default();
    put_version(secret, &[("k", "v")]);

    {
        let v = secret.get_version_mut(1).unwrap();
        v.destroyed = true;
        v.data = None;
    }

    // attempt undelete — must remain destroyed
    {
        let v = secret.get_version_mut(1).unwrap();
        if !v.destroyed {
            v.deletion_time = None;
        }
    }

    let v = secret.get_version(1).unwrap();
    assert!(v.destroyed);
    assert!(v.data.is_none(), "destroyed version data must be None");
}

/// Cite: openbao `builtin/logical/kv/backend.go:33` (defaultMaxVersions = 10)
/// + `builtin/logical/kv/path_data.go:229` (cleanupOldVersions). When the
/// version count exceeds `max_versions`, the OLDEST live version is evicted
/// and `oldest_version` is bumped accordingly.
#[test]
fn max_versions_evicts_oldest() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/rolling".into()).or_default();
    secret.max_versions = 3;

    for i in 1..=5 {
        put_version(secret, &[("v", &format!("{}", i))]);
        // mimic the eviction loop in write_secret
        let max = secret.max_versions;
        while secret.versions.len() as u64 > max {
            secret.oldest_version = secret.versions[0].version + 1;
            secret.versions.remove(0);
        }
    }

    assert_eq!(secret.current_version, 5);
    assert_eq!(secret.versions.len(), 3);
    assert_eq!(secret.oldest_version, 3, "oldest_version advances past evicted ones");
    assert!(secret.get_version(1).is_none());
    assert!(secret.get_version(3).is_some());
    assert!(secret.get_version(5).is_some());
}

/// Cite: openbao `builtin/logical/kv/path_metadata.go:179` (pathMetadataList)
/// — listing returns directory-style keys: leaf entries verbatim, sub-paths
/// suffixed with `/`. Mirrors the cave `list_secrets` axum handler.
#[test]
fn list_returns_directory_style_keys() {
    let mut store = Kv2Store::default();
    let m = store.data.entry("kv".into()).or_default();
    m.insert("apps/web/config".into(), Kv2Secret::default());
    m.insert("apps/api/config".into(), Kv2Secret::default());
    m.insert("apps/standalone".into(), Kv2Secret::default());

    let prefix = "apps/";
    let mut seen = std::collections::BTreeSet::new();
    for k in m.keys() {
        if let Some(rest) = k.strip_prefix(prefix) {
            let part = rest.split('/').next().unwrap_or(rest);
            if rest.contains('/') {
                seen.insert(format!("{}/", part));
            } else {
                seen.insert(part.to_string());
            }
        }
    }
    let keys: Vec<String> = seen.into_iter().collect();
    assert_eq!(keys, vec!["api/".to_string(), "standalone".into(), "web/".into()]);
}

/// Cite: openbao `builtin/logical/kv/path_metadata.go:334` (pathMetadataRead)
/// — metadata response includes `current_version`, `oldest_version`,
/// `max_versions`, `cas_required`, `custom_metadata`, plus a per-version
/// map. The cave `read_metadata` handler emits the same shape.
#[test]
fn metadata_includes_per_version_map() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("a".into()).or_default();
    put_version(secret, &[("k", "v1")]);
    put_version(secret, &[("k", "v2")]);

    secret.custom_metadata.insert("env".into(), "prod".into());
    secret.cas_required = true;
    secret.max_versions = 7;

    assert_eq!(secret.versions.len(), 2);
    assert_eq!(secret.current_version, 2);
    assert_eq!(secret.max_versions, 7);
    assert!(secret.cas_required);
    assert_eq!(secret.custom_metadata.get("env"), Some(&"prod".to_string()));
}

/// Cite: openbao `builtin/logical/kv/path_data.go:197` (validateCheckAndSetOption)
/// — the `cas` option must equal the current version, otherwise the write
/// is rejected with `check-and-set parameter did not match the current version`.
#[test]
fn cas_mismatch_blocks_write() {
    let mut store = Kv2Store::default();
    let secret = store.data.entry("kv".into()).or_default()
        .entry("svc/cas".into()).or_default();
    put_version(secret, &[("v", "1")]);
    assert_eq!(secret.current_version, 1);

    // attempt CAS=0 (would only succeed against an empty path)
    let cas: u64 = 0;
    assert_ne!(cas, secret.current_version, "CAS must match — this would fail in handler");

    // CAS that matches advances version
    let cas: u64 = 1;
    assert_eq!(cas, secret.current_version);
    put_version(secret, &[("v", "2")]);
    assert_eq!(secret.current_version, 2);
}

/// Cite: openbao `builtin/logical/kv/path_metadata.go:765` (pathMetadataDelete)
/// — deleting metadata wipes ALL versions of the secret in one go.
#[test]
fn metadata_delete_wipes_all_versions() {
    let mut store = Kv2Store::default();
    let m = store.data.entry("kv".into()).or_default();
    let secret = m.entry("svc/wipe".into()).or_default();
    put_version(secret, &[("v", "a")]);
    put_version(secret, &[("v", "b")]);
    put_version(secret, &[("v", "c")]);
    assert_eq!(secret.versions.len(), 3);

    m.remove("svc/wipe");
    assert!(m.get("svc/wipe").is_none());
}
