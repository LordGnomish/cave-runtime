// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for incremental status reporting during reconcile — the
//! TenantControlPlane conditions flip True as their owning pipeline stages
//! converge, and `Ready` aggregates only at steady state.
//!
//! Faithful port target (Kamaji v1.0.0):
//!   internal/controllers/conditions.go — per-stage condition reporting
//!   controllers/tenantcontrolplane_controller.go — status update each Reconcile

use cave_kamaji::components::{ControlPlaneInput, DatastoreBinding, NetworkProfile};
use cave_kamaji::connection::{Driver, FakeConnection};
use cave_kamaji::ds_setup::SetupResource;
use cave_kamaji::models::{TenantControlPlane, TenantPhase, TenantSpec, TenantStatus};
use cave_kamaji::reconcile::{ReconcileContext, Reconciler};
use cave_kamaji::status::{ConditionStatus, ConditionType};
use chrono::Utc;
use uuid::Uuid;

fn tcp() -> TenantControlPlane {
    let now = Utc::now();
    TenantControlPlane {
        id: Uuid::new_v4(),
        name: "alpha".into(),
        namespace: "tenants".into(),
        spec: TenantSpec {
            kubernetes_version: "v1.31.0".into(),
            data_store: "postgres".into(),
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

fn control_plane() -> ControlPlaneInput {
    ControlPlaneInput {
        name: "alpha".into(),
        version: "v1.31.0".into(),
        advertise_address: "0.0.0.0".into(),
        network: NetworkProfile {
            service_cidr: "10.96.0.0/12".into(),
            pod_cidr: "10.244.0.0/16".into(),
            port: 6443,
        },
        datastore: DatastoreBinding {
            driver: Driver::PostgreSql,
            endpoints: vec![],
            schema: "tenants_alpha".into(),
        },
        admission_plugins: vec!["NodeRestriction".into()],
        preferred_address_types: vec!["InternalIP".into()],
    }
}

fn setup() -> SetupResource {
    SetupResource {
        schema: "tenants_alpha".into(),
        user: "tenants_alpha".into(),
        password: "pw".into(),
    }
}

fn ctx<'a>(conn: &'a mut FakeConnection) -> ReconcileContext<'a> {
    ReconcileContext {
        connection: conn,
        setup: setup(),
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(),
    }
}

#[test]
fn no_conditions_before_first_reconcile() {
    let r = Reconciler::new();
    assert!(r.conditions().is_empty());
    assert_eq!(r.condition_status(ConditionType::Ready), None);
}

#[test]
fn first_pass_reports_subconditions_true_but_not_ready() {
    let mut t = tcp();
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let mut r = Reconciler::new();
    let mut c = ctx(&mut conn);

    // First pass applies every step (datastore-setup, deployment, konnectivity,
    // kubeconfigs all converge) but is NOT steady state yet.
    let pass = r.reconcile(&mut t, &mut c).unwrap();
    assert!(!pass.steady_state);

    // The sub-conditions whose stages converged are reported True…
    assert_eq!(
        r.condition_status(ConditionType::DataStoreHealthy),
        Some(ConditionStatus::True)
    );
    assert_eq!(
        r.condition_status(ConditionType::ControlPlaneHealthy),
        Some(ConditionStatus::True)
    );
    assert_eq!(
        r.condition_status(ConditionType::KonnectivityHealthy),
        Some(ConditionStatus::True)
    );
    assert_eq!(
        r.condition_status(ConditionType::KubeconfigReady),
        Some(ConditionStatus::True)
    );
    // …but the aggregate Ready stays False until steady state.
    assert_eq!(
        r.condition_status(ConditionType::Ready),
        Some(ConditionStatus::False)
    );
}

#[test]
fn steady_state_aggregates_ready_true() {
    let mut t = tcp();
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let mut r = Reconciler::new();
    let mut c = ctx(&mut conn);

    r.reconcile(&mut t, &mut c).unwrap(); // converging pass
    r.reconcile(&mut t, &mut c).unwrap(); // steady state

    assert_eq!(
        r.condition_status(ConditionType::Ready),
        Some(ConditionStatus::True)
    );
    // The full condition set is reported.
    assert_eq!(r.conditions().len(), 5);
}

#[test]
fn delete_clears_reported_conditions() {
    let mut t = tcp();
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let mut r = Reconciler::new();
    let mut c = ctx(&mut conn);

    r.reconcile(&mut t, &mut c).unwrap();
    r.reconcile(&mut t, &mut c).unwrap();
    assert!(!r.conditions().is_empty());

    r.reconcile_delete(&mut t, &mut c).unwrap();
    assert!(
        r.conditions().is_empty(),
        "deprovisioning clears the reported status"
    );
}
