// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD coverage fills for `cave-kamaji` (theme: compute).
//!
//! Upstream reference: clastix/kamaji @ v1.0.0
//!   - api/v1alpha1/tenantcontrolplane_status.go (status condition matrix)
//!   - internal/utilities / internal/resources (kubeconfig surface)
//!
//! The crate's existing suites (`phase2_deep_port.rs`, `test_gap_close_edges.rs`,
//! and inline `#[cfg(test)]` modules) already exercise the happy paths and the
//! primary error branches of `lifecycle`, `webhook`, `konnectivity`, and
//! `status`. This file deliberately adds ONLY behaviors that no existing test
//! asserts:
//!
//!   * `status::status_summary` — the non-running branch where
//!     `DataStoreHealthy` / `KonnectivityHealthy` resolve to `Unknown`
//!     (distinct from the `False`/`True` paths already covered), and the exact
//!     cardinality of the returned condition vector (5 condition types).
//!   * `lifecycle::generate_kubeconfig` — the `clusters[0].name`,
//!     `contexts[0]`, and `users[0]` substructure derived from `tcp.name`
//!     (existing tests only assert `kind`, `current-context`, and the cluster
//!     `server` URL).
//!
//! Pure consumer of the public API; never touches `src/`.

use cave_kamaji::{
    lifecycle::{generate_kubeconfig, mark_running},
    models::{TenantControlPlane, TenantPhase, TenantSpec, TenantStatus},
    status::{ConditionStatus, ConditionType, status_summary},
};
use chrono::Utc;
use uuid::Uuid;

fn tcp(name: &str) -> TenantControlPlane {
    let now = Utc::now();
    TenantControlPlane {
        id: Uuid::new_v4(),
        name: name.into(),
        namespace: "ns".into(),
        spec: TenantSpec {
            kubernetes_version: "v1.31.0".into(),
            data_store: "shared-etcd".into(),
            replicas: 2,
        },
        status: TenantStatus {
            phase: TenantPhase::Provisioning,
            api_server_endpoint: None,
            ready: false,
            message: None,
        },
        created_at: now,
        updated_at: now,
    }
}

// ── status_summary: the `Unknown` branch + exact cardinality ───────────────
//
// In `status_summary`, when `running == false` the DataStoreHealthy and
// KonnectivityHealthy conditions are emitted as `Unknown` (NOT `False`), while
// ControlPlaneHealthy and KubeconfigReady are `False`. Existing tests only
// assert the aggregate `Ready` is `False` for a provisioning TCP — none pin
// the `Unknown` status of the datastore/konnectivity sub-conditions.

#[test]
fn status_summary_non_running_datastore_and_konnectivity_are_unknown() {
    let t = tcp("t"); // Provisioning, ready=false  => running == false
    let conds = status_summary(&t);

    let ds = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::DataStoreHealthy)
        .expect("DataStoreHealthy present");
    assert_eq!(
        ds.status,
        ConditionStatus::Unknown,
        "non-running TCP leaves datastore health Unknown, not False"
    );

    let kn = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::KonnectivityHealthy)
        .expect("KonnectivityHealthy present");
    assert_eq!(
        kn.status,
        ConditionStatus::Unknown,
        "non-running TCP leaves konnectivity health Unknown, not False"
    );

    // The two phase-gated conditions remain False (distinct from Unknown).
    let cp = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::ControlPlaneHealthy)
        .expect("ControlPlaneHealthy present");
    assert_eq!(cp.status, ConditionStatus::False);
    let kc = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::KubeconfigReady)
        .expect("KubeconfigReady present");
    assert_eq!(kc.status, ConditionStatus::False);
}

#[test]
fn status_summary_emits_exactly_five_distinct_condition_types() {
    let t = tcp("t");
    let conds = status_summary(&t);
    assert_eq!(
        conds.len(),
        5,
        "status_summary must emit exactly 5 conditions"
    );

    // Every condition type appears exactly once (no duplicate/overwrite leak).
    for ct in [
        ConditionType::ControlPlaneHealthy,
        ConditionType::KubeconfigReady,
        ConditionType::DataStoreHealthy,
        ConditionType::KonnectivityHealthy,
        ConditionType::Ready,
    ] {
        let n = conds.iter().filter(|c| c.cond_type == ct).count();
        assert_eq!(n, 1, "{ct:?} must appear exactly once, found {n}");
    }
}

// ── generate_kubeconfig: name-derived cluster/context/user substructure ────
//
// The kubeconfig is built from `tcp.name`: the cluster entry, the context
// entry, and `context.cluster` all carry the tenant name, and a fixed `admin`
// user is emitted. Existing tests assert `kind`, `current-context`, and the
// cluster `server`; none assert the cluster `name`, the context substructure,
// or the user entry.

#[test]
fn generate_kubeconfig_cluster_context_and_user_derive_from_name() {
    let mut t = tcp("alpha");
    mark_running(&mut t, "https://api.alpha:6443".into());
    let kc = generate_kubeconfig(&t).expect("kubeconfig present once endpoint set");

    // apiVersion is the fixed kubeconfig schema version.
    assert_eq!(kc["apiVersion"], "v1");

    // cluster entry is named after the tenant.
    assert_eq!(kc["clusters"][0]["name"], "alpha");

    // context entry is named after the tenant and binds cluster + admin user.
    assert_eq!(kc["contexts"][0]["name"], "alpha");
    assert_eq!(kc["contexts"][0]["context"]["cluster"], "alpha");
    assert_eq!(kc["contexts"][0]["context"]["user"], "admin");

    // a single admin user entry is emitted.
    assert_eq!(kc["users"][0]["name"], "admin");
}
