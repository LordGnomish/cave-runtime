// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! End-to-end multi-tenant lifecycle spec — provisions several tenant control
//! planes against one shared datastore through the [`TenantManager`], asserts
//! their datastores and control planes are isolated, enforces the cross-tenant
//! boundary, then decommissions one tenant without disturbing the others.
//!
//! Faithful port target (Kamaji v1.0.0):
//!   controllers/tenantcontrolplane_controller.go — per-tenant Reconcile loop
//!   internal/utilities/utilities.go               — isolation labelling
//!   api/v1alpha1/datastore_types.go               — DataStore.status.usedBy

use cave_kamaji::components::{ControlPlaneInput, DatastoreBinding, NetworkProfile};
use cave_kamaji::connection::{Connection, Driver, FakeConnection};
use cave_kamaji::ds_setup::{SetupResource, tenant_schema};
use cave_kamaji::isolation::kamaji_labels;
use cave_kamaji::manager::{IsolationError, TenantManager};
use cave_kamaji::models::{TenantControlPlane, TenantPhase, TenantSpec, TenantStatus};
use chrono::Utc;
use uuid::Uuid;

fn tcp(name: &str) -> TenantControlPlane {
    let now = Utc::now();
    TenantControlPlane {
        id: Uuid::new_v4(),
        name: name.into(),
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

fn control_plane(tcp: &TenantControlPlane) -> ControlPlaneInput {
    ControlPlaneInput {
        name: tcp.name.clone(),
        version: tcp.spec.kubernetes_version.clone(),
        advertise_address: "0.0.0.0".into(),
        network: NetworkProfile {
            service_cidr: "10.96.0.0/12".into(),
            pod_cidr: "10.244.0.0/16".into(),
            port: 6443,
        },
        datastore: DatastoreBinding {
            driver: Driver::PostgreSql,
            endpoints: vec![],
            schema: tenant_schema(&tcp.namespace, &tcp.name),
        },
        admission_plugins: vec!["NodeRestriction".into()],
        preferred_address_types: vec!["InternalIP".into()],
    }
}

fn setup(tcp: &TenantControlPlane) -> SetupResource {
    let s = tenant_schema(&tcp.namespace, &tcp.name);
    SetupResource {
        schema: s.clone(),
        user: s,
        password: "pw".into(),
    }
}

/// Provision three tenants on one shared back-end, then drive a full lifecycle.
#[test]
fn three_tenants_share_a_backend_with_isolated_datastores() {
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let mut mgr = TenantManager::new();

    let mut alpha = tcp("alpha");
    let mut beta = tcp("beta");
    let mut gamma = tcp("gamma");

    for t in [&mut alpha, &mut beta, &mut gamma] {
        let cp = control_plane(t);
        let su = setup(t);
        let endpoint = format!("https://{}.tenants.svc:6443", t.name);
        mgr.provision(t, &mut conn, cp, su, endpoint).unwrap();
    }

    // All three converged to Ready.
    assert_eq!(mgr.tenant_count(), 3);
    assert!(mgr.is_ready(alpha.id));
    assert!(mgr.is_ready(beta.id));
    assert!(mgr.is_ready(gamma.id));
    assert_eq!(alpha.status.phase, TenantPhase::Running);

    // Each tenant carved its own isolated schema + user out of the shared DB.
    for schema in ["tenants_alpha", "tenants_beta", "tenants_gamma"] {
        assert!(conn.db_exists(schema).unwrap(), "{schema} db exists");
        assert!(conn.user_exists(schema).unwrap(), "{schema} user exists");
    }

    // The Kine schema was migrated against the shared back-end.
    assert!(
        conn.statement_log()
            .iter()
            .any(|s| s.starts_with("CREATE TABLE IF NOT EXISTS kine"))
    );

    // Each tenant has its own dedicated 3-container control plane.
    assert_eq!(mgr.control_plane(alpha.id).unwrap().len(), 3);
    assert_eq!(mgr.control_plane(gamma.id).unwrap().len(), 3);

    // The shared DataStore usedBy registry tracks all three, sorted.
    assert_eq!(
        mgr.used_by(),
        &["tenants/alpha", "tenants/beta", "tenants/gamma"]
    );
}

#[test]
fn cross_tenant_resource_access_is_refused() {
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let mut mgr = TenantManager::new();

    let mut alpha = tcp("alpha");
    let mut beta = tcp("beta");
    for t in [&mut alpha, &mut beta] {
        let cp = control_plane(t);
        let su = setup(t);
        mgr.provision(t, &mut conn, cp, su, "https://x:6443").unwrap();
    }

    // A resource stamped for alpha is owned by alpha…
    let alpha_apiserver = kamaji_labels("alpha", "kube-apiserver");
    assert!(mgr.authorize("alpha", &alpha_apiserver).is_ok());

    // …and beta may not touch it.
    let err = mgr.authorize("beta", &alpha_apiserver).unwrap_err();
    assert_eq!(
        err,
        IsolationError::CrossTenant {
            requested_by: "beta".into(),
            owned_by: Some("alpha".into()),
        }
    );
}

#[test]
fn deleting_one_tenant_leaves_the_others_running() {
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let mut mgr = TenantManager::new();

    let mut alpha = tcp("alpha");
    let mut beta = tcp("beta");
    let mut gamma = tcp("gamma");
    for t in [&mut alpha, &mut beta, &mut gamma] {
        let cp = control_plane(t);
        let su = setup(t);
        mgr.provision(t, &mut conn, cp, su, "https://x:6443").unwrap();
    }

    // Decommission beta only.
    mgr.delete(&mut beta, &mut conn).unwrap();

    // beta's isolated datastore is gone…
    assert!(!conn.db_exists("tenants_beta").unwrap());
    assert!(!conn.user_exists("tenants_beta").unwrap());
    assert_eq!(beta.status.phase, TenantPhase::Deleting);
    assert_eq!(mgr.tenant_count(), 2);
    assert!(!mgr.is_ready(beta.id));

    // …while alpha and gamma keep their datastores and stay Ready.
    assert!(conn.db_exists("tenants_alpha").unwrap());
    assert!(conn.db_exists("tenants_gamma").unwrap());
    assert!(mgr.is_ready(alpha.id));
    assert!(mgr.is_ready(gamma.id));

    // beta released its claim on the shared back-end.
    assert_eq!(mgr.used_by(), &["tenants/alpha", "tenants/gamma"]);
}
