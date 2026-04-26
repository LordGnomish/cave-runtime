//! Namespace + tenant_id — parity tests against openbao v2.5.3.
//!
//! Upstream package: `helper/namespace/namespace.go`. cave-vault adds a
//! `tenant_id` field to bind a Vault namespace to a cave-runtime tenant.

use cave_vault::{Namespace, NamespaceStore};

/// Cite: openbao `helper/namespace/namespace.go:259` (Canonicalize) — a
/// non-empty namespace path is normalised to a trailing slash. cave's
/// `Namespace::new` mirrors this so `get_by_path` works regardless of
/// caller-side trailing-slash hygiene.
#[test]
fn namespace_path_canonicalised_with_trailing_slash() {
    let ns = Namespace::new("ns-1", "team-payments", "tenant-acme");
    assert_eq!(ns.path, "team-payments/");
    assert_eq!(ns.id, "ns-1");
    assert_eq!(ns.tenant_id, "tenant-acme");

    let ns2 = Namespace::new("ns-2", "team-billing/", "tenant-acme");
    assert_eq!(ns2.path, "team-billing/", "already canonical → unchanged");
}

/// Cite: openbao `helper/namespace/namespace.go:54` (Namespace.Validate) +
/// `:23` (reservedNames) — paths matching reserved system mounts (sys/,
/// audit/, auth/, cubbyhole/, identity/) or the literal "root" must be
/// rejected.
#[test]
fn namespace_rejects_reserved_paths() {
    for bad in &["sys/", "audit/", "auth/", "cubbyhole/", "identity/"] {
        let ns = Namespace::new("x", *bad, "t");
        assert!(ns.validate().is_err(), "{} must be reserved", bad);
    }
    let root_ns = Namespace::new("x", "root", "t");
    assert!(root_ns.validate().is_err(), "literal 'root' is forbidden");

    let ok = Namespace::new("x", "team-x", "t");
    assert!(ok.validate().is_ok(), "regular tenant path is fine");
}

/// Cite: openbao `helper/namespace/namespace.go:220` (FromContext) —
/// cave's `NamespaceStore::get` is the resolution point that the request
/// pipeline uses after extracting the namespace ID from the context.
#[test]
fn store_round_trips_namespace_by_id_and_path() {
    let mut store = NamespaceStore::default();
    store.create(Namespace::new("ns-prod", "prod", "tenant-acme")).unwrap();
    store.create(Namespace::new("ns-stage", "stage", "tenant-acme")).unwrap();

    let by_id = store.get("ns-prod").expect("by id");
    assert_eq!(by_id.path, "prod/");

    let by_path = store.get_by_path("stage").expect("by path; trailing slash inferred");
    assert_eq!(by_path.id, "ns-stage");

    let none = store.get_by_path("does-not-exist");
    assert!(none.is_none());
}

/// Cite: cave extension on top of openbao `helper/namespace/namespace.go:40`
/// (Namespace) — multi-tenant filtering. A single tenant may own many
/// namespaces; `for_tenant` returns them sorted by canonical path.
#[test]
fn for_tenant_filters_and_sorts_namespaces() {
    let mut store = NamespaceStore::default();
    store.create(Namespace::new("a-prod", "alpha-prod", "tenant-alpha")).unwrap();
    store.create(Namespace::new("a-stage", "alpha-stage", "tenant-alpha")).unwrap();
    store.create(Namespace::new("b-prod", "beta-prod", "tenant-beta")).unwrap();

    let alpha = store.for_tenant("tenant-alpha");
    assert_eq!(alpha.len(), 2);
    assert_eq!(alpha[0].path, "alpha-prod/");
    assert_eq!(alpha[1].path, "alpha-stage/");

    let beta = store.for_tenant("tenant-beta");
    assert_eq!(beta.len(), 1);
    assert_eq!(beta[0].id, "b-prod");

    assert_eq!(store.for_tenant("tenant-nobody").len(), 0);

    assert!(store.delete("a-prod"));
    assert_eq!(store.for_tenant("tenant-alpha").len(), 1);
}
