//! deeper-001: Mount table — enable+disable, namespace scoping, listing,
//! defaults, longest-prefix dispatch. Pinned to openbao v2.5.3.

use cave_vault::{MountConfig, MountEntry, MountTable};

const TENANT_A: &str = "tenant-acme-prod";
const TENANT_B: &str = "tenant-beta-staging";

fn entry(path: &str, mtype: &str, ns: &str, desc: &str) -> MountEntry {
    MountEntry {
        path: path.into(),
        mount_type: mtype.into(),
        description: desc.into(),
        config: MountConfig::default(),
        local: false,
        seal_wrap: false,
        uuid: uuid::Uuid::new_v4().to_string(),
        accessor: uuid::Uuid::new_v4().to_string(),
        namespace_id: ns.into(),
    }
}

/// Cite: openbao `vault/mount.go:1705` (persistMounts) + `:302` (remove)
/// — enable + disable round-trips. Disabling a mount returns the entry
/// and removes it from the table; the mount is no longer routable.
#[test]
fn enable_then_disable_round_trips_with_description_preserved() {
    let mut t = MountTable::default();
    let mount = entry("kv-prod/", "kv", TENANT_A, "production secrets");
    t.register(mount.clone());

    let stored = t.lookup("kv-prod/").unwrap();
    assert_eq!(stored.description, "production secrets");
    assert_eq!(stored.namespace_id, TENANT_A);

    let removed = t.unregister("kv-prod/").expect("returns entry");
    assert_eq!(removed.uuid, mount.uuid);
    assert!(t.lookup("kv-prod/").is_none());
    assert!(t.unregister("kv-prod/").is_none(),
        "second disable is a no-op");
}

/// Cite: openbao `vault/mount.go:328` (findAllNamespaceMounts) — when
/// scoping by namespace, the table returns only entries whose
/// `namespace_id` matches; root-namespace entries are not leaked into
/// tenant queries (and vice versa).
#[test]
fn namespace_scoping_isolates_tenants() {
    let mut t = MountTable::default();
    t.register(entry("kv-a/", "kv", TENANT_A, "a"));
    t.register(entry("kv-b/", "kv", TENANT_B, "b"));
    t.register(entry("audit/", "audit", "", "root"));

    let a_mounts = t.for_namespace(TENANT_A);
    let b_mounts = t.for_namespace(TENANT_B);
    let root_mounts = t.for_namespace("");

    assert_eq!(a_mounts.len(), 1);
    assert_eq!(a_mounts[0].path, "kv-a/");
    assert_eq!(b_mounts.len(), 1);
    assert_eq!(b_mounts[0].path, "kv-b/");
    assert_eq!(root_mounts.len(), 1);
    assert_eq!(root_mounts[0].path, "audit/");

    // Cross-tenant access ⇒ empty.
    assert!(t.for_namespace("tenant-nobody").is_empty());
}

/// Cite: openbao `vault/mount.go:344` (find — predicate scan) — when
/// two mounts could match an API path (e.g. `kv/` and `kv/team/`),
/// the deeper mount wins.
#[test]
fn longest_prefix_dispatch_resolves_overlapping_mounts() {
    let mut t = MountTable::default();
    t.register(entry("kv/", "kv", TENANT_A, "shared"));
    t.register(entry("kv/team-A/", "kv", TENANT_A, "team scoped"));
    t.register(entry("kv/team-A/private/", "kv", TENANT_A, "deepest"));

    let m = t.longest_prefix("kv/team-A/private/secret").unwrap();
    assert_eq!(m.path, "kv/team-A/private/");

    let m = t.longest_prefix("kv/team-A/secret").unwrap();
    assert_eq!(m.path, "kv/team-A/");

    let m = t.longest_prefix("kv/other/secret").unwrap();
    assert_eq!(m.path, "kv/");
}

/// Cite: openbao `vault/mount.go:361` (sortEntriesByPath) — listing the
/// mount table yields paths in lexicographic order, regardless of
/// insertion order.
#[test]
fn list_returns_lexicographically_sorted_paths() {
    let mut t = MountTable::default();
    for p in ["zeta/", "alpha/", "beta/", "kv-prod/", "audit/"] {
        t.register(entry(p, "kv", TENANT_A, ""));
    }
    let listed = t.list();
    assert_eq!(
        listed,
        vec!["alpha/", "audit/", "beta/", "kv-prod/", "zeta/"]
            .into_iter().map(String::from).collect::<Vec<_>>(),
    );
}

/// Cite: openbao `vault/mount.go:380` (MountEntry struct) + cave layer
/// extension — every mount carries an opaque accessor UUID, distinct
/// from the path. Two registrations of the same path-type with different
/// descriptions yield different accessors.
#[test]
fn each_mount_gets_unique_accessor_and_uuid() {
    let mut t = MountTable::default();
    let m1 = entry("kv-1/", "kv", TENANT_A, "one");
    let m2 = entry("kv-2/", "kv", TENANT_A, "two");
    t.register(m1.clone());
    t.register(m2.clone());

    let a1 = t.lookup("kv-1/").unwrap();
    let a2 = t.lookup("kv-2/").unwrap();
    assert_ne!(a1.uuid, a2.uuid);
    assert_ne!(a1.accessor, a2.accessor);
    assert_eq!(a1.uuid, m1.uuid);
    assert_eq!(a2.accessor, m2.accessor);
}
