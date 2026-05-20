// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant control-plane pod orchestration plan — Cave-side projection
//! of the upstream Kamaji `kubeapiserver` resource controller.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/resources/kubeapiserver/*.go
//!
//! Kamaji upstream materialises a Deployment for the tenant's
//! kube-apiserver with replicas + leader-election + datastore wiring.
//! The Cave port produces an [`ApiServerPodPlan`] — a manifest-shaped
//! struct cave-runtime / cave-kubelet consume to schedule the actual
//! container. We do not produce a Kubernetes Deployment YAML here:
//! that envelope is owned by cave-controller-manager.

use crate::models::TenantControlPlane;

/// Materialised plan for a tenant's kube-apiserver pod (or pod set).
#[derive(Debug, Clone)]
pub struct ApiServerPodPlan {
    pub replicas: u32,
    pub image: String,
    pub command: Vec<String>,
    pub args: Vec<String>,
    /// Mountpoints the pod needs (PKI bundle, kine socket, etc.).
    pub volume_mounts: Vec<String>,
    /// Environment variables — e.g. `KAMAJI_TENANT=foo`.
    pub env: Vec<(String, String)>,
}

/// Produce an [`ApiServerPodPlan`] from a TenantControlPlane spec.
pub fn plan_apiserver_pod(tcp: &TenantControlPlane) -> ApiServerPodPlan {
    let image = format!(
        "registry.k8s.io/kube-apiserver:{}",
        tcp.spec.kubernetes_version
    );
    let mut args = vec![
        "--allow-privileged=true".into(),
        format!("--advertise-address={}", "0.0.0.0"),
        format!("--etcd-servers={}", default_etcd_endpoint(&tcp.spec.data_store)),
        "--secure-port=6443".into(),
        "--service-cluster-ip-range=10.96.0.0/12".into(),
        format!("--service-account-issuer=https://kubernetes.default.svc.{}", tcp.name),
        "--authorization-mode=Node,RBAC".into(),
        format!("--tls-cert-file=/etc/kubernetes/pki/{}/apiserver.crt", tcp.name),
        format!("--tls-private-key-file=/etc/kubernetes/pki/{}/apiserver.key", tcp.name),
    ];
    args.sort();
    ApiServerPodPlan {
        replicas: tcp.spec.replicas,
        image,
        command: vec!["kube-apiserver".into()],
        args,
        volume_mounts: vec![
            "/etc/kubernetes/pki".into(),
            "/var/run/kamaji".into(),
        ],
        env: vec![
            ("KAMAJI_TENANT".to_string(), tcp.name.clone()),
            ("KUBERNETES_VERSION".to_string(), tcp.spec.kubernetes_version.clone()),
        ],
    }
}

fn default_etcd_endpoint(data_store: &str) -> &'static str {
    match data_store {
        "shared-etcd" | "etcd" => "https://etcd.cave-system.svc:2379",
        "postgres" | "postgresql" | "mysql" => "unix:///var/run/kine/kine.sock",
        _ => "https://etcd.cave-system.svc:2379",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TenantPhase, TenantSpec, TenantStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn base_tcp(data_store: &str) -> TenantControlPlane {
        let now = Utc::now();
        TenantControlPlane {
            id: Uuid::new_v4(),
            name: "t1".into(),
            namespace: "default".into(),
            spec: TenantSpec {
                kubernetes_version: "v1.31.0".into(),
                data_store: data_store.into(),
                replicas: 3,
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
    fn shared_etcd_yields_etcd_endpoint() {
        let plan = plan_apiserver_pod(&base_tcp("shared-etcd"));
        assert!(plan.args.iter().any(|a| a.contains("etcd.cave-system.svc")));
    }

    #[test]
    fn postgres_data_store_yields_kine_socket() {
        let plan = plan_apiserver_pod(&base_tcp("postgres"));
        assert!(plan.args.iter().any(|a| a.contains("kine.sock")));
    }

    #[test]
    fn env_carries_tenant_and_version() {
        let plan = plan_apiserver_pod(&base_tcp("shared-etcd"));
        assert!(plan.env.iter().any(|(k, v)| k == "KAMAJI_TENANT" && v == "t1"));
        assert!(plan
            .env
            .iter()
            .any(|(k, v)| k == "KUBERNETES_VERSION" && v == "v1.31.0"));
    }
}
