// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mount table — parity tests against openbao v2.5.3.
//!
//! Upstream package: `vault/mount.go`.

use cave_vault::{MountConfig, MountEntry, MountTable};

fn entry(path: &str, mtype: &str) -> MountEntry {
    MountEntry {
        path: path.into(),
        mount_type: mtype.into(),
        description: String::new(),
        config: MountConfig::default(),
        local: false,
        seal_wrap: false,
        uuid: uuid::Uuid::new_v4().to_string(),
        accessor: uuid::Uuid::new_v4().to_string(),
        namespace_id: String::new(),
    }
}

/// Cite: openbao `vault/mount.go:1705` (persistMounts) and the table
/// `register` helper — registering an entry stores it under its path key.
#[test]
fn register_and_lookup_by_exact_path() {
    let mut t = MountTable::default();
    t.register(entry("kv/", "kv"));
    let m = t.lookup("kv/").unwrap();
    assert_eq!(m.mount_type, "kv");
    assert_eq!(m.path, "kv/");
}

/// Cite: openbao `vault/mount.go:344` (MountTable.find — predicate scan
/// used to resolve API paths to mount entries). The longest matching
/// prefix wins, so a `/v1/secret/data/foo` request resolves to the `kv/`
/// mount only if no deeper mount shadows it.
#[test]
fn longest_prefix_match_resolves_request_path() {
    let mut t = MountTable::default();
    t.register(entry("kv/", "kv"));
    t.register(entry("kv/team-a/", "kv"));
    t.register(entry("transit/", "transit"));

    let m = t.longest_prefix("kv/team-a/data/foo").unwrap();
    assert_eq!(m.path, "kv/team-a/", "deeper mount shadows the broader one");

    let m = t.longest_prefix("kv/team-b/data/foo").unwrap();
    assert_eq!(m.path, "kv/", "fallback to the broader mount");

    let m = t.longest_prefix("transit/encrypt/key").unwrap();
    assert_eq!(m.path, "transit/");

    assert!(t.longest_prefix("does/not/exist/").is_none());
}

/// Cite: openbao `vault/mount.go:302` (MountTable.remove) — unregistering
/// an entry removes it and returns the prior `MountEntry` value.
#[test]
fn unregister_returns_removed_entry() {
    let mut t = MountTable::default();
    t.register(entry("ephemeral/", "kv"));
    let removed = t.unregister("ephemeral/").expect("returns the entry");
    assert_eq!(removed.mount_type, "kv");
    assert!(t.lookup("ephemeral/").is_none());
    assert!(t.unregister("ephemeral/").is_none(), "second unregister is a no-op");
}

/// Cite: openbao `vault/mount.go:328` (MountTable.findAllNamespaceMounts)
/// — when scoping the table by namespace, only the entries with a matching
/// `namespace_id` are returned, regardless of mount path overlap.
#[test]
fn for_namespace_filters_by_tenant_scope() {
    let mut t = MountTable::default();
    let mut ns_a = entry("kv/", "kv");
    ns_a.namespace_id = "tenant-a".into();
    let mut ns_b = entry("kv/", "kv");
    ns_b.namespace_id = "tenant-b".into();
    let mut root = entry("audit/", "audit");
    root.namespace_id = String::new();

    t.mounts.insert("ns-a-kv".into(), ns_a);
    t.mounts.insert("ns-b-kv".into(), ns_b);
    t.mounts.insert("audit/".into(), root);

    assert_eq!(t.for_namespace("tenant-a").len(), 1);
    assert_eq!(t.for_namespace("tenant-b").len(), 1);
    assert_eq!(t.for_namespace("").len(), 1, "root namespace mounts");
    assert_eq!(t.for_namespace("missing").len(), 0);
}

/// Cite: openbao `vault/mount.go:361` (MountTable.sortEntriesByPath) —
/// listing yields paths in lexicographic order.
#[test]
fn list_returns_sorted_paths() {
    let mut t = MountTable::default();
    for p in ["zeta/", "alpha/", "beta/"] {
        t.register(entry(p, "kv"));
    }
    let listed = t.list();
    assert_eq!(listed, vec!["alpha/".to_string(), "beta/".into(), "zeta/".into()]);
}
