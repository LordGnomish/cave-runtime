// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for the TenantControlPlane reconcile flow — the ordered
//! pipeline that provisions a dedicated control plane for each tenant and
//! drives its lifecycle phase to Running.
//!
//! Faithful port target (Kamaji v1.0.0):
//!   controllers/resources.go  — getDefaultResources ordering, GetDeletableResources

use cave_kamaji::components::{ControlPlaneInput, DatastoreBinding, NetworkProfile};
use cave_kamaji::connection::{Driver, FakeConnection};
use cave_kamaji::ds_setup::SetupResource;
use cave_kamaji::models::{TenantControlPlane, TenantPhase, TenantSpec, TenantStatus};
use cave_kamaji::reconcile::{ReconcileContext, Reconciler, default_pipeline};
use chrono::Utc;
use uuid::Uuid;

fn control_plane(driver: Driver, schema: &str) -> ControlPlaneInput {
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
            driver,
            endpoints: vec!["etcd.cave-system.svc:2379".into()],
            schema: schema.into(),
        },
        admission_plugins: vec!["NodeRestriction".into()],
        preferred_address_types: vec!["InternalIP".into(), "Hostname".into()],
    }
}

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

fn ctx_parts() -> (FakeConnection, SetupResource) {
    (
        FakeConnection::new(Driver::PostgreSql),
        SetupResource {
            schema: "tenants_alpha".into(),
            user: "tenants_alpha".into(),
            password: "pw".into(),
        },
    )
}

// ── Pipeline ordering (getDefaultResources) ─────────────────────────────────

#[test]
fn pipeline_starts_with_migration_and_ends_with_ingress() {
    let p = default_pipeline();
    assert_eq!(p.first().copied(), Some("datastore-migrate"));
    assert_eq!(p.last().copied(), Some("ingress"));
}

fn idx(step: &str) -> usize {
    default_pipeline().iter().position(|s| *s == step).expect(step)
}

#[test]
fn certificates_precede_kubeconfigs() {
    assert!(idx("ca") < idx("admin-kubeconfig"));
    assert!(idx("apiserver-certificate") < idx("scheduler-kubeconfig"));
}

#[test]
fn datastore_setup_precedes_deployment() {
    // the control-plane containers can only start once the tenant DB exists
    assert!(idx("datastore-setup") < idx("deployment"));
    assert!(idx("ds.multitenancy") < idx("datastore-config"));
    assert!(idx("datastore-config") < idx("datastore-setup"));
}

#[test]
fn konnectivity_requirements_wrap_the_deployment() {
    assert!(idx("konnectivity-certificate") < idx("deployment"));
    assert!(idx("deployment") < idx("konnectivity-service"));
}

// ── Reconcile convergence + phase machine ───────────────────────────────────

#[test]
fn first_pass_provisions_and_creates_tenant_datastore() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };

    let pass = r.reconcile(&mut t, &mut c).unwrap();
    assert!(!pass.steady_state, "first pass still converging");
    assert_eq!(t.status.phase, TenantPhase::Provisioning);
    assert!(!t.status.ready);
    // the datastore-setup step ran for real: tenant schema/user now exist
    assert!(c.connection.db_exists("tenants_alpha").unwrap());
    assert!(c.connection.user_exists("tenants_alpha").unwrap());
}

#[test]
fn second_pass_reaches_steady_state_and_marks_running() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };

    r.reconcile(&mut t, &mut c).unwrap();
    let pass = r.reconcile(&mut t, &mut c).unwrap();
    assert!(pass.steady_state, "converged on the second pass");
    assert!(pass.applied.is_empty());
    assert_eq!(t.status.phase, TenantPhase::Running);
    assert!(t.status.ready);
    assert_eq!(
        t.status.api_server_endpoint.as_deref(),
        Some("https://alpha.tenants.svc:6443")
    );
    assert_eq!(r.converged_steps(), default_pipeline().len());
}

#[test]
fn reconcile_is_idempotent_once_converged() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };
    r.reconcile(&mut t, &mut c).unwrap(); // provisions everything
    r.reconcile(&mut t, &mut c).unwrap(); // converged -> Running
    // every subsequent pass is a no-op: the datastore-setup step stays
    // converged so no further datastore work is performed.
    let pass = r.reconcile(&mut t, &mut c).unwrap();
    assert!(pass.applied.is_empty(), "converged pipeline applies nothing");
    assert_eq!(t.status.phase, TenantPhase::Running);
    assert!(c.connection.db_exists("tenants_alpha").unwrap());
}

// ── Deletion (GetDeletableResources) ────────────────────────────────────────

#[test]
fn delete_tears_down_datastore_and_enters_deleting() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };
    r.reconcile(&mut t, &mut c).unwrap();
    r.reconcile(&mut t, &mut c).unwrap();

    r.reconcile_delete(&mut t, &mut c).unwrap();
    assert_eq!(t.status.phase, TenantPhase::Deleting);
    assert!(!t.status.ready);
    assert!(!c.connection.db_exists("tenants_alpha").unwrap());
    assert!(!c.connection.user_exists("tenants_alpha").unwrap());
}

// ── Dedicated control-plane materialisation (deployment step) ────────────────

#[test]
fn deployment_step_builds_the_three_dedicated_components() {
    // Before reconcile the control plane has not been materialised.
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    assert!(r.control_plane().is_none(), "no components before reconcile");

    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };
    r.reconcile(&mut t, &mut c).unwrap();

    // Once the `deployment` step converges, the dedicated apiserver +
    // controller-manager + scheduler container plan exists, in that order.
    let cp = r.control_plane().expect("deployment step materialised the CP");
    let names: Vec<&str> = cp.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["kube-apiserver", "kube-controller-manager", "kube-scheduler"]
    );
}

#[test]
fn kine_backed_tenant_apiserver_points_at_local_kine_sidecar() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };
    r.reconcile(&mut t, &mut c).unwrap();

    let cp = r.control_plane().unwrap();
    let apiserver = cp.iter().find(|c| c.name == "kube-apiserver").unwrap();
    assert!(
        apiserver
            .args
            .iter()
            .any(|a| a == "--etcd-servers=http://127.0.0.1:2379"),
        "SQL/kine driver routes the apiserver at the local Kine etcd shim"
    );
    assert!(
        !apiserver.args.iter().any(|a| a.starts_with("--etcd-prefix")),
        "kine driver does not set an etcd key prefix"
    );
}

#[test]
fn etcd_backed_tenant_apiserver_sets_per_tenant_key_prefix() {
    let mut t = tcp();
    let mut conn = FakeConnection::new(Driver::Etcd);
    let setup = SetupResource {
        schema: "tenants_alpha".into(),
        user: "tenants_alpha".into(),
        password: "pw".into(),
    };
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::Etcd, "tenants_alpha"),
    };
    r.reconcile(&mut t, &mut c).unwrap();

    let cp = r.control_plane().unwrap();
    let apiserver = cp.iter().find(|c| c.name == "kube-apiserver").unwrap();
    assert!(
        apiserver
            .args
            .iter()
            .any(|a| a == "--etcd-prefix=/tenants_alpha"),
        "etcd driver isolates the tenant under its own key prefix"
    );
}

#[test]
fn reconcile_migrates_the_kine_schema_for_sql_tenants() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    {
        let mut c = ReconcileContext {
            connection: &mut conn,
            setup,
            api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
            control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
        };
        r.reconcile(&mut t, &mut c).unwrap();
    }
    // The datastore-migrate step ran the Kine table DDL before datastore-setup.
    assert!(
        conn.statement_log()
            .iter()
            .any(|s| s.starts_with("CREATE TABLE IF NOT EXISTS kine")),
        "reconcile materialised the Kine schema"
    );
}

#[test]
fn deleting_a_tenant_tears_down_its_materialised_control_plane() {
    let mut t = tcp();
    let (mut conn, setup) = ctx_parts();
    let mut r = Reconciler::new();
    let mut c = ReconcileContext {
        connection: &mut conn,
        setup,
        api_server_endpoint: "https://alpha.tenants.svc:6443".into(),
        control_plane: control_plane(Driver::PostgreSql, "tenants_alpha"),
    };
    r.reconcile(&mut t, &mut c).unwrap();
    assert!(r.control_plane().is_some());

    r.reconcile_delete(&mut t, &mut c).unwrap();
    assert!(
        r.control_plane().is_none(),
        "deleting the tenant releases its control-plane plan"
    );
}
