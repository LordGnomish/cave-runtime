// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for the REST plan projections that surface the new
//! control-plane / datastore / reconcile ports over the Kamaji HTTP API.

use cave_kamaji::models::{TenantControlPlane, TenantPhase, TenantSpec, TenantStatus};
use cave_kamaji::routes::{
    component_plan_json, driver_from_data_store, reconcile_plan_json, status_plan_json,
};
use cave_kamaji::connection::Driver;
use chrono::Utc;
use uuid::Uuid;

fn tcp(data_store: &str) -> TenantControlPlane {
    let now = Utc::now();
    TenantControlPlane {
        id: Uuid::new_v4(),
        name: "alpha".into(),
        namespace: "tenants".into(),
        spec: TenantSpec {
            kubernetes_version: "v1.31.0".into(),
            data_store: data_store.into(),
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

#[test]
fn driver_mapping_covers_aliases() {
    assert_eq!(driver_from_data_store("etcd"), Driver::Etcd);
    assert_eq!(driver_from_data_store("shared-etcd"), Driver::Etcd);
    assert_eq!(driver_from_data_store("postgres"), Driver::PostgreSql);
    assert_eq!(driver_from_data_store("postgresql"), Driver::PostgreSql);
    assert_eq!(driver_from_data_store("mysql"), Driver::MySql);
    assert_eq!(driver_from_data_store("nats"), Driver::Nats);
}

#[test]
fn component_plan_lists_three_components_with_args() {
    let v = component_plan_json(&tcp("postgres"));
    let comps = v["components"].as_array().unwrap();
    assert_eq!(comps.len(), 3);
    assert_eq!(comps[0]["name"], "kube-apiserver");
    assert_eq!(comps[1]["name"], "kube-controller-manager");
    assert_eq!(comps[2]["name"], "kube-scheduler");
    // postgres -> Kine local etcd shim
    let api_args = comps[0]["args"].as_array().unwrap();
    assert!(
        api_args
            .iter()
            .any(|a| a.as_str() == Some("--etcd-servers=http://127.0.0.1:2379"))
    );
}

#[test]
fn reconcile_plan_exposes_pipeline_and_isolated_datastore() {
    let v = reconcile_plan_json(&tcp("postgres"));
    assert_eq!(v["phase"], "provisioning");
    let pipe = v["pipeline"].as_array().unwrap();
    assert_eq!(pipe.first().and_then(|s| s.as_str()), Some("datastore-migrate"));
    assert_eq!(pipe.last().and_then(|s| s.as_str()), Some("ingress"));
    // datastore isolation: per-tenant schema/user derived from ns_name
    assert_eq!(v["datastore"]["schema"], "tenants_alpha");
    assert_eq!(v["datastore"]["user"], "tenants_alpha");
    assert_eq!(v["datastore"]["driver"], "PostgreSQL");
    assert_eq!(v["datastore"]["supports_multitenancy"], true);
}

#[test]
fn reconcile_plan_flags_nats_single_tenant() {
    let v = reconcile_plan_json(&tcp("nats"));
    assert_eq!(v["datastore"]["driver"], "NATS");
    assert_eq!(v["datastore"]["supports_multitenancy"], false);
}

#[test]
fn status_plan_reports_five_conditions_not_ready_while_provisioning() {
    let v = status_plan_json(&tcp("postgres"));
    assert_eq!(v["phase"], "provisioning");
    assert_eq!(v["ready"], false);
    let conds = v["conditions"].as_array().unwrap();
    assert_eq!(conds.len(), 5);
    let ready = conds
        .iter()
        .find(|c| c["type"] == "Ready")
        .expect("Ready condition present");
    assert_eq!(ready["status"], "False");
}

#[test]
fn status_plan_reports_ready_true_when_running() {
    let mut t = tcp("postgres");
    t.status.phase = TenantPhase::Running;
    t.status.ready = true;
    t.status.api_server_endpoint = Some("https://alpha.tenants.svc:6443".into());
    let v = status_plan_json(&t);
    assert_eq!(v["ready"], true);
    let conds = v["conditions"].as_array().unwrap();
    let ready = conds.iter().find(|c| c["type"] == "Ready").unwrap();
    assert_eq!(ready["status"], "True");
}
