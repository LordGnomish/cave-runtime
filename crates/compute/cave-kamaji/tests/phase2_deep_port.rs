// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-kamaji Phase 2 deep-port — tests for datastore controllers,
//! Konnectivity tunneling, validating webhook, control-plane pod
//! orchestration sketch, and status conditions.

use cave_kamaji::datastore::{
    DataStore, DataStoreKind, DataStoreSpec, EtcdSnapshotConfig, KineDriver,
};
use cave_kamaji::konnectivity::{Konnectivity, KonnectivityMode};
use cave_kamaji::kubeadm::{KubeadmConfig, render_kubeadm_init_config};
use cave_kamaji::models::{TenantControlPlane, TenantPhase, TenantSpec, TenantStatus};
use cave_kamaji::pod_mgmt::{ApiServerPodPlan, plan_apiserver_pod};
use cave_kamaji::status::{
    Condition, ConditionStatus, ConditionType, set_condition, status_summary,
};
use cave_kamaji::webhook::{WebhookError, validate_create, validate_update};
use chrono::Utc;
use uuid::Uuid;

fn base_tcp() -> TenantControlPlane {
    let now = Utc::now();
    TenantControlPlane {
        id: Uuid::new_v4(),
        name: "t1".into(),
        namespace: "default".into(),
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

// ─── Datastore controllers ──────────────────────────────────────────────────

#[test]
fn datastore_postgres_spec_carries_dsn_and_kine_driver() {
    let ds = DataStore {
        name: "postgres-primary".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::Postgres,
            endpoints: vec!["postgres://kine@10.0.0.5:5432/k8s".into()],
            kine_driver: Some(KineDriver::Postgres),
            tls_secret: Some("kine-tls".into()),
            etcd_snapshot: None,
        },
    };
    assert_eq!(ds.spec.kind, DataStoreKind::Postgres);
    assert!(ds.connection_string().contains("postgres://"));
}

#[test]
fn datastore_mysql_uses_kine_mysql_driver() {
    let ds = DataStore {
        name: "mysql".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::MySql,
            endpoints: vec!["mysql://kine@10.0.0.6:3306/k8s".into()],
            kine_driver: Some(KineDriver::MySql),
            tls_secret: None,
            etcd_snapshot: None,
        },
    };
    assert_eq!(ds.spec.kind, DataStoreKind::MySql);
}

#[test]
fn datastore_etcd_supports_s3_snapshot() {
    let ds = DataStore {
        name: "etcd-shared".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::Etcd,
            endpoints: vec!["https://10.0.0.10:2379".into()],
            kine_driver: None,
            tls_secret: Some("etcd-tls".into()),
            etcd_snapshot: Some(EtcdSnapshotConfig {
                s3_bucket: "cave-snapshots".into(),
                s3_endpoint: "https://s3.example.com".into(),
                schedule_cron: "0 */6 * * *".into(),
                retention: 14,
            }),
        },
    };
    assert!(ds.spec.etcd_snapshot.is_some());
    assert_eq!(ds.spec.etcd_snapshot.as_ref().unwrap().retention, 14);
}

#[test]
fn datastore_validates_endpoints_non_empty() {
    let ds = DataStore {
        name: "bad".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::Etcd,
            endpoints: vec![],
            kine_driver: None,
            tls_secret: None,
            etcd_snapshot: None,
        },
    };
    let err = ds.validate().unwrap_err();
    assert!(err.contains("endpoints"));
}

// ─── Konnectivity controller ────────────────────────────────────────────────

#[test]
fn konnectivity_grpc_mode_has_default_port() {
    let k = Konnectivity::new(KonnectivityMode::Grpc);
    assert_eq!(k.server_port, 8132);
    assert_eq!(k.mode, KonnectivityMode::Grpc);
}

#[test]
fn konnectivity_http_connect_mode_uses_8133() {
    let k = Konnectivity::new(KonnectivityMode::HttpConnect);
    assert_eq!(k.server_port, 8133);
}

#[test]
fn konnectivity_agent_token_can_be_set() {
    let mut k = Konnectivity::new(KonnectivityMode::Grpc);
    k.with_agent_token("secret-token");
    assert_eq!(k.agent_token.as_deref(), Some("secret-token"));
}

// ─── Validating webhook ─────────────────────────────────────────────────────

#[test]
fn webhook_rejects_empty_name() {
    let mut tcp = base_tcp();
    tcp.name = "".into();
    let err = validate_create(&tcp).unwrap_err();
    assert!(matches!(err, WebhookError::EmptyField { .. }));
}

#[test]
fn webhook_rejects_zero_replicas() {
    let mut tcp = base_tcp();
    tcp.spec.replicas = 0;
    let err = validate_create(&tcp).unwrap_err();
    assert!(matches!(err, WebhookError::InvalidReplicas { .. }));
}

#[test]
fn webhook_rejects_unknown_data_store_kind() {
    let mut tcp = base_tcp();
    tcp.spec.data_store = "noSQL-magic".into();
    let err = validate_create(&tcp).unwrap_err();
    assert!(matches!(err, WebhookError::UnknownDataStore { .. }));
}

#[test]
fn webhook_rejects_kubernetes_version_change_on_update() {
    let old = base_tcp();
    let mut new = old.clone();
    new.spec.kubernetes_version = "v1.32.0".into();
    let err = validate_update(&old, &new).unwrap_err();
    assert!(matches!(err, WebhookError::ImmutableField { .. }));
}

#[test]
fn webhook_accepts_replica_change_on_update() {
    let old = base_tcp();
    let mut new = old.clone();
    new.spec.replicas = 3;
    validate_update(&old, &new).unwrap();
}

// ─── Control-plane pod orchestration ────────────────────────────────────────

#[test]
fn plan_apiserver_pod_emits_one_pod_per_replica() {
    let tcp = base_tcp();
    let plan = plan_apiserver_pod(&tcp);
    assert_eq!(plan.replicas, 2);
    assert!(plan.image.contains("kube-apiserver:v1.31.0"));
    assert!(plan.command.iter().any(|c| c == "kube-apiserver"));
    assert!(plan.args.iter().any(|a| a.starts_with("--etcd-servers=")));
}

#[test]
fn plan_apiserver_pod_carries_static_args() {
    let tcp = base_tcp();
    let plan = plan_apiserver_pod(&tcp);
    let joined = plan.args.join(" ");
    assert!(joined.contains("--secure-port"));
    assert!(joined.contains("--service-cluster-ip-range"));
}

// ─── kubeadm init bootstrap ────────────────────────────────────────────────

#[test]
fn kubeadm_init_config_renders_cluster_section() {
    let cfg = KubeadmConfig {
        cluster_name: "t1".into(),
        kubernetes_version: "v1.31.0".into(),
        pod_subnet: "10.244.0.0/16".into(),
        service_subnet: "10.96.0.0/12".into(),
        api_advertise_address: "10.0.0.1".into(),
        control_plane_endpoint: "10.0.0.1:6443".into(),
    };
    let yaml = render_kubeadm_init_config(&cfg);
    assert!(yaml.contains("kind: InitConfiguration"));
    assert!(yaml.contains("kind: ClusterConfiguration"));
    assert!(yaml.contains("kubernetesVersion: v1.31.0"));
    assert!(yaml.contains("podSubnet: 10.244.0.0/16"));
    assert!(yaml.contains("controlPlaneEndpoint: 10.0.0.1:6443"));
}

// ─── Status conditions ──────────────────────────────────────────────────────

#[test]
fn set_condition_appends_when_unknown_type() {
    let mut conds: Vec<Condition> = vec![];
    set_condition(
        &mut conds,
        ConditionType::Ready,
        ConditionStatus::True,
        "Ready",
        "all up",
    );
    assert_eq!(conds.len(), 1);
    assert_eq!(conds[0].cond_type, ConditionType::Ready);
}

#[test]
fn set_condition_updates_in_place_when_type_present() {
    let mut conds: Vec<Condition> = vec![];
    set_condition(
        &mut conds,
        ConditionType::Ready,
        ConditionStatus::False,
        "NotReady",
        "warming up",
    );
    set_condition(
        &mut conds,
        ConditionType::Ready,
        ConditionStatus::True,
        "Ready",
        "all up",
    );
    assert_eq!(conds.len(), 1);
    assert_eq!(conds[0].status, ConditionStatus::True);
}

#[test]
fn status_summary_reflects_running_tcp() {
    let mut tcp = base_tcp();
    tcp.status.phase = TenantPhase::Running;
    tcp.status.api_server_endpoint = Some("https://10.0.0.1:6443".into());
    tcp.status.ready = true;
    let conds = status_summary(&tcp);
    assert!(
        conds
            .iter()
            .any(|c| c.cond_type == ConditionType::Ready && c.status == ConditionStatus::True)
    );
    assert!(
        conds
            .iter()
            .any(|c| c.cond_type == ConditionType::ControlPlaneHealthy
                && c.status == ConditionStatus::True)
    );
}

#[test]
fn status_summary_reflects_provisioning_tcp() {
    let tcp = base_tcp();
    let conds = status_summary(&tcp);
    assert!(
        conds
            .iter()
            .any(|c| c.cond_type == ConditionType::Ready && c.status == ConditionStatus::False)
    );
}
