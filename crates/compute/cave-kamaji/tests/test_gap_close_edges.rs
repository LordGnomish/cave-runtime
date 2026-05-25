// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-close edges for cave-kamaji.
//!
//! Focuses on areas not covered by `phase2_deep_port.rs` or inline `#[cfg(test)]`
//! modules: lifecycle state transitions, kubeconfig generation, model serde
//! round-trips, datastore validation edges, konnectivity builder + arg shape,
//! kubeadm field surface, pod-mgmt arg ordering / unknown-backend fallback,
//! webhook update immutability + recovery on revert, status aggregate
//! transitions.
//!
//! Hard constraint: pure consumer of the public API — does not touch
//! `crates/cave-kamaji/src/`.

use cave_kamaji::{
    datastore::{DataStore, DataStoreKind, DataStoreSpec, EtcdSnapshotConfig, KineDriver},
    konnectivity::{Konnectivity, KonnectivityMode},
    kubeadm::{KubeadmConfig, render_kubeadm_init_config},
    lifecycle::{deprovision, generate_kubeconfig, health_check, mark_running, provision},
    models::{
        CreateTenantRequest, TenantControlPlane, TenantPhase, TenantSpec, TenantStatus,
    },
    pod_mgmt::plan_apiserver_pod,
    status::{ConditionStatus, ConditionType, status_summary},
    webhook::{WebhookError, validate_create, validate_update},
};
use chrono::Utc;
use uuid::Uuid;

// ---- helpers -----------------------------------------------------------

fn tcp(name: &str, ns: &str, data_store: &str, replicas: u32) -> TenantControlPlane {
    let now = Utc::now();
    TenantControlPlane {
        id: Uuid::new_v4(),
        name: name.into(),
        namespace: ns.into(),
        spec: TenantSpec {
            kubernetes_version: "v1.31.0".into(),
            data_store: data_store.into(),
            replicas,
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

// ---- lifecycle state transitions ---------------------------------------

#[test]
fn provision_sets_phase_provisioning_and_message() {
    let mut t = tcp("a", "ns", "shared-etcd", 1);
    t.status.phase = TenantPhase::Failed;
    t.status.message = Some("stale".into());
    provision(&mut t);
    assert_eq!(t.status.phase, TenantPhase::Provisioning);
    assert!(
        t.status
            .message
            .as_deref()
            .unwrap_or("")
            .contains("provisioned")
    );
}

#[test]
fn mark_running_clears_message_and_sets_endpoint() {
    let mut t = tcp("a", "ns", "shared-etcd", 1);
    t.status.message = Some("initialising".into());
    mark_running(&mut t, "https://10.0.0.1:6443".into());
    assert_eq!(t.status.phase, TenantPhase::Running);
    assert!(t.status.ready);
    assert_eq!(
        t.status.api_server_endpoint.as_deref(),
        Some("https://10.0.0.1:6443")
    );
    assert!(t.status.message.is_none());
}

#[test]
fn deprovision_flips_ready_and_phase_after_running() {
    let mut t = tcp("a", "ns", "shared-etcd", 1);
    mark_running(&mut t, "https://x:6443".into());
    deprovision(&mut t);
    assert_eq!(t.status.phase, TenantPhase::Deleting);
    assert!(!t.status.ready);
    assert!(t.status.message.is_some());
}

#[test]
fn health_check_requires_both_running_and_ready() {
    let mut t = tcp("a", "ns", "shared-etcd", 1);
    assert!(!health_check(&t));
    t.status.phase = TenantPhase::Running;
    t.status.ready = false;
    assert!(!health_check(&t)); // running but not ready
    t.status.ready = true;
    assert!(health_check(&t));
    t.status.phase = TenantPhase::Upgrading;
    assert!(!health_check(&t)); // ready but not running
}

#[test]
fn generate_kubeconfig_none_when_endpoint_missing() {
    let t = tcp("a", "ns", "shared-etcd", 1);
    assert!(generate_kubeconfig(&t).is_none());
}

#[test]
fn generate_kubeconfig_populated_when_endpoint_present() {
    let mut t = tcp("alpha", "ns", "shared-etcd", 1);
    mark_running(&mut t, "https://api.alpha:6443".into());
    let kc = generate_kubeconfig(&t).expect("kubeconfig");
    assert_eq!(kc["kind"], "Config");
    assert_eq!(kc["current-context"], "alpha");
    assert_eq!(kc["clusters"][0]["cluster"]["server"], "https://api.alpha:6443");
}

// ---- models serde round-trip ------------------------------------------

#[test]
fn tenant_phase_serde_lowercase_round_trip() {
    let json = serde_json::to_string(&TenantPhase::Running).unwrap();
    assert_eq!(json, "\"running\"");
    let back: TenantPhase = serde_json::from_str("\"deleting\"").unwrap();
    assert_eq!(back, TenantPhase::Deleting);
}

#[test]
fn create_tenant_request_deserialises_from_minimal_json() {
    let raw = r#"{"name":"t","namespace":"n","spec":{"kubernetes_version":"v1.31.0","data_store":"shared-etcd","replicas":2}}"#;
    let req: CreateTenantRequest = serde_json::from_str(raw).unwrap();
    assert_eq!(req.name, "t");
    assert_eq!(req.spec.replicas, 2);
}

#[test]
fn tcp_round_trip_preserves_id_and_spec() {
    let t = tcp("rt", "ns", "shared-etcd", 3);
    let s = serde_json::to_string(&t).unwrap();
    let back: TenantControlPlane = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, t.id);
    assert_eq!(back.spec.replicas, 3);
    assert_eq!(back.namespace, "ns");
}

// ---- datastore edges ---------------------------------------------------

#[test]
fn datastore_etcd_requires_no_kine_driver() {
    let ds = DataStore {
        name: "e".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::Etcd,
            endpoints: vec!["https://etcd:2379".into()],
            kine_driver: None,
            tls_secret: None,
            etcd_snapshot: None,
        },
    };
    ds.validate().unwrap();
}

#[test]
fn datastore_validate_rejects_empty_endpoints() {
    let ds = DataStore {
        name: "empty".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::Etcd,
            endpoints: vec![],
            kine_driver: None,
            tls_secret: None,
            etcd_snapshot: None,
        },
    };
    let err = ds.validate().unwrap_err();
    assert!(err.contains("endpoints must not be empty"));
    assert!(err.contains("empty"));
}

#[test]
fn datastore_connection_string_single_endpoint_has_no_comma() {
    let ds = DataStore {
        name: "x".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::Etcd,
            endpoints: vec!["only-one".into()],
            kine_driver: None,
            tls_secret: None,
            etcd_snapshot: None,
        },
    };
    assert_eq!(ds.connection_string(), "only-one");
    assert!(!ds.connection_string().contains(','));
}

#[test]
fn datastore_etcd_snapshot_round_trip() {
    let snap = EtcdSnapshotConfig {
        s3_bucket: "kamaji-snaps".into(),
        s3_endpoint: "https://s3.cave.svc".into(),
        schedule_cron: "0 * * * *".into(),
        retention: 7,
    };
    let s = serde_json::to_string(&snap).unwrap();
    let back: EtcdSnapshotConfig = serde_json::from_str(&s).unwrap();
    assert_eq!(back.retention, 7);
    assert_eq!(back.schedule_cron, "0 * * * *");
}

#[test]
fn datastore_mysql_with_postgres_driver_rejected() {
    let ds = DataStore {
        name: "m".into(),
        spec: DataStoreSpec {
            kind: DataStoreKind::MySql,
            endpoints: vec!["mysql://x".into()],
            kine_driver: Some(KineDriver::Postgres),
            tls_secret: None,
            etcd_snapshot: None,
        },
    };
    assert!(ds.validate().is_err());
}

// ---- konnectivity builder + args --------------------------------------

#[test]
fn konnectivity_http_connect_args_contain_http_connect_mode() {
    let k = Konnectivity::new(KonnectivityMode::HttpConnect);
    let args = k.agent_manifest_args();
    assert!(args.iter().any(|a| a == "--mode=http-connect"));
    assert!(args.iter().any(|a| a.contains("--proxy-server-port=8133")));
}

#[test]
fn konnectivity_default_server_host_present() {
    let k = Konnectivity::new(KonnectivityMode::Grpc);
    let args = k.agent_manifest_args();
    assert!(
        args.iter()
            .any(|a| a == "--proxy-server-host=konnectivity.svc")
    );
}

#[test]
fn konnectivity_with_server_host_and_token_propagate() {
    let mut k = Konnectivity::new(KonnectivityMode::Grpc);
    k.with_server_host("kn.example.com");
    k.with_agent_token("agent-token-file");
    let args = k.agent_manifest_args();
    assert!(
        args.iter()
            .any(|a| a == "--proxy-server-host=kn.example.com")
    );
    assert!(
        args.iter()
            .any(|a| a.contains("agent-token-file") && a.starts_with("--service-account-token-path="))
    );
}

#[test]
fn konnectivity_empty_agent_id_omits_identifier_flag() {
    let k = Konnectivity::new(KonnectivityMode::Grpc);
    assert!(k.agent_id.is_empty());
    let args = k.agent_manifest_args();
    assert!(!args.iter().any(|a| a.contains("--agent-identifiers")));
}

// ---- kubeadm field coverage -------------------------------------------

#[test]
fn kubeadm_render_carries_subnets_and_endpoint() {
    let cfg = KubeadmConfig {
        cluster_name: "k1".into(),
        kubernetes_version: "v1.31.0".into(),
        pod_subnet: "10.10.0.0/16".into(),
        service_subnet: "10.20.0.0/12".into(),
        api_advertise_address: "10.0.0.5".into(),
        control_plane_endpoint: "10.0.0.5:6443".into(),
    };
    let s = render_kubeadm_init_config(&cfg);
    assert!(s.contains("podSubnet: 10.10.0.0/16"));
    assert!(s.contains("serviceSubnet: 10.20.0.0/12"));
    assert!(s.contains("controlPlaneEndpoint: 10.0.0.5:6443"));
    assert!(s.contains("advertiseAddress: 10.0.0.5"));
    assert!(s.contains("dnsDomain: cluster.local"));
    assert!(s.contains("apiVersion: kubeadm.k8s.io/v1beta3"));
}

// ---- pod_mgmt args / fallback -----------------------------------------

#[test]
fn pod_mgmt_args_are_sorted_lexicographically() {
    let t = tcp("t", "ns", "shared-etcd", 2);
    let plan = plan_apiserver_pod(&t);
    let mut sorted = plan.args.clone();
    sorted.sort();
    assert_eq!(plan.args, sorted, "args must be deterministically sorted");
}

#[test]
fn pod_mgmt_unknown_backend_falls_back_to_etcd_endpoint() {
    let t = tcp("t", "ns", "neo4j-not-real", 1);
    let plan = plan_apiserver_pod(&t);
    assert!(
        plan.args
            .iter()
            .any(|a| a.contains("etcd.cave-system.svc:2379")),
        "unknown backend should fall back to default etcd endpoint"
    );
}

#[test]
fn pod_mgmt_image_pin_matches_kubernetes_version() {
    let mut t = tcp("t", "ns", "shared-etcd", 1);
    t.spec.kubernetes_version = "v1.30.4".into();
    let plan = plan_apiserver_pod(&t);
    assert_eq!(plan.image, "registry.k8s.io/kube-apiserver:v1.30.4");
}

#[test]
fn pod_mgmt_tls_paths_include_tenant_name() {
    let t = tcp("tenant-x", "ns", "shared-etcd", 1);
    let plan = plan_apiserver_pod(&t);
    assert!(
        plan.args
            .iter()
            .any(|a| a.contains("/etc/kubernetes/pki/tenant-x/apiserver.crt"))
    );
    assert!(
        plan.args
            .iter()
            .any(|a| a.contains("/etc/kubernetes/pki/tenant-x/apiserver.key"))
    );
}

#[test]
fn pod_mgmt_command_is_kube_apiserver_singleton() {
    let t = tcp("t", "ns", "shared-etcd", 1);
    let plan = plan_apiserver_pod(&t);
    assert_eq!(plan.command, vec!["kube-apiserver".to_string()]);
}

// ---- webhook edges -----------------------------------------------------

#[test]
fn webhook_rejects_whitespace_only_name() {
    let mut t = tcp("   ", "ns", "shared-etcd", 1);
    t.namespace = "ns".into();
    match validate_create(&t).unwrap_err() {
        WebhookError::EmptyField { field } => assert_eq!(field, "name"),
        other => panic!("expected EmptyField name, got {other:?}"),
    }
}

#[test]
fn webhook_rejects_whitespace_only_namespace() {
    let t = tcp("ok", "\t \n", "shared-etcd", 1);
    match validate_create(&t).unwrap_err() {
        WebhookError::EmptyField { field } => assert_eq!(field, "namespace"),
        other => panic!("expected EmptyField namespace, got {other:?}"),
    }
}

#[test]
fn webhook_update_passes_when_only_replicas_change() {
    let old = tcp("ok", "ns", "shared-etcd", 2);
    let mut new = old.clone();
    new.spec.replicas = 5;
    validate_update(&old, &new).unwrap();
}

#[test]
fn webhook_update_recovers_after_revert_of_immutable_field() {
    let old = tcp("ok", "ns", "shared-etcd", 1);
    let mut new = old.clone();
    new.spec.kubernetes_version = "v1.32.0".into();
    assert!(validate_update(&old, &new).is_err());
    new.spec.kubernetes_version = old.spec.kubernetes_version.clone();
    validate_update(&old, &new).unwrap();
}

#[test]
fn webhook_accepts_all_known_data_store_aliases() {
    for ds in ["shared-etcd", "etcd", "postgres", "postgresql", "mysql"] {
        let t = tcp("ok", "ns", ds, 1);
        validate_create(&t).unwrap_or_else(|e| panic!("kind {ds} rejected: {e:?}"));
    }
}

// ---- status aggregation -----------------------------------------------

#[test]
fn status_summary_provisioning_yields_not_ready_aggregate() {
    let t = tcp("t", "ns", "shared-etcd", 1);
    let conds = status_summary(&t);
    let ready = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::Ready)
        .expect("Ready condition present");
    assert_eq!(ready.status, ConditionStatus::False);
    assert_eq!(ready.reason, "NotReady");
}

#[test]
fn status_summary_running_with_endpoint_yields_full_true() {
    let mut t = tcp("t", "ns", "shared-etcd", 1);
    mark_running(&mut t, "https://api.t:6443".into());
    let conds = status_summary(&t);
    for ct in [
        ConditionType::ControlPlaneHealthy,
        ConditionType::KubeconfigReady,
        ConditionType::DataStoreHealthy,
        ConditionType::KonnectivityHealthy,
        ConditionType::Ready,
    ] {
        let c = conds.iter().find(|c| c.cond_type == ct).unwrap_or_else(|| {
            panic!("missing condition {ct:?}")
        });
        assert_eq!(c.status, ConditionStatus::True, "{ct:?} should be True");
    }
}

#[test]
fn status_summary_running_without_endpoint_marks_kubeconfig_false() {
    // Force phase=Running + ready=true but leave api_server_endpoint=None
    // (an inconsistent state — exercises the endpoint-gate in KubeconfigReady).
    let mut t = tcp("t", "ns", "shared-etcd", 1);
    t.status.phase = TenantPhase::Running;
    t.status.ready = true;
    // api_server_endpoint stays None
    let conds = status_summary(&t);
    let kc = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::KubeconfigReady)
        .unwrap();
    assert_eq!(kc.status, ConditionStatus::False);
    let ready = conds
        .iter()
        .find(|c| c.cond_type == ConditionType::Ready)
        .unwrap();
    assert_eq!(ready.status, ConditionStatus::False);
}
