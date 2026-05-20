// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster API integration — `internal/controllers/clusterapi`.
//!
//! Kamaji's CAPI bootstrap-provider shape: a TenantControlPlane participates
//! in a Cluster API `Cluster` by exposing a `ControlPlaneEndpoint` and
//! claiming an `infrastructureRef`. This module ports the wire-shape of
//! the CAPI bootstrap contract (host/port + ready bits + infrastructureRef
//! handle) and renders the CAPI-shaped status block that the CAPI reconciler
//! polls.
//!
//! Mapped surfaces:
//! * `internal/controllers/clusterapi/cluster_handler.go`     — bootstrap status
//! * `internal/controllers/clusterapi/control_plane.go`       — ControlPlaneEndpoint
//! * `internal/controllers/clusterapi/infrastructure_ref.go`  — infrastructureRef
//! * `api/v1alpha1/tenantcontrolplane_capi.go`                — CAPI-shaped fields

use crate::models::{TenantControlPlane, TenantPhase};
use serde::{Deserialize, Serialize};

/// Cluster API `ControlPlaneEndpoint` — `Host:Port` shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ControlPlaneEndpoint {
    pub host: String,
    pub port: u16,
}

impl ControlPlaneEndpoint {
    pub fn is_zero(&self) -> bool {
        self.host.is_empty() && self.port == 0
    }
    pub fn to_url(&self) -> String {
        format!("https://{}:{}", self.host, self.port)
    }
}

/// Cluster API `InfrastructureRef` — Kind+APIVersion+Name+Namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfrastructureRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
}

/// CAPI-shaped bootstrap status. Mirrors `cluster.x-k8s.io/v1beta1`'s
/// minimal contract for a bootstrap provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CapiBootstrapStatus {
    pub ready: bool,
    pub initialized: bool,
    pub data_secret_name: Option<String>,
    pub failure_reason: Option<String>,
    pub failure_message: Option<String>,
}

/// CAPI annotation shape — the CAPI reconciler poll target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapiTenantStatus {
    pub control_plane_endpoint: ControlPlaneEndpoint,
    pub bootstrap: CapiBootstrapStatus,
    pub infrastructure_ref: Option<InfrastructureRef>,
    pub ready_replicas: u32,
    pub replicas: u32,
    pub unavailable_replicas: u32,
}

/// Parse a Kamaji `tcp.status.api_server_endpoint` URL into the CAPI
/// `ControlPlaneEndpoint` host/port shape. Returns the zero value for
/// malformed inputs (the CAPI reconciler then waits).
pub fn parse_control_plane_endpoint(s: &str) -> ControlPlaneEndpoint {
    let trimmed = s
        .trim()
        .strip_prefix("https://")
        .or_else(|| s.trim().strip_prefix("http://"))
        .unwrap_or(s.trim());
    let (host, port) = trimmed
        .rsplit_once(':')
        .map(|(h, p)| (h, p.parse::<u16>().ok()))
        .unwrap_or((trimmed, None));
    let Some(port) = port else {
        return ControlPlaneEndpoint::default();
    };
    if host.is_empty() {
        return ControlPlaneEndpoint::default();
    }
    ControlPlaneEndpoint {
        host: host.to_string(),
        port,
    }
}

/// Build a CAPI-shaped status block for one Kamaji TenantControlPlane.
pub fn build_capi_status(tcp: &TenantControlPlane, infra: Option<InfrastructureRef>) -> CapiTenantStatus {
    let ep = tcp
        .status
        .api_server_endpoint
        .as_deref()
        .map(parse_control_plane_endpoint)
        .unwrap_or_default();
    let (ready, initialized, fail_reason, fail_msg) = match tcp.status.phase {
        TenantPhase::Running => (true, true, None, None),
        TenantPhase::Provisioning | TenantPhase::Upgrading => (false, false, None, None),
        TenantPhase::Deleting => (false, true, None, None),
        TenantPhase::Failed => (
            false,
            true,
            Some("ControlPlaneFailed".to_string()),
            tcp.status.message.clone(),
        ),
    };
    let ready_replicas = if matches!(tcp.status.phase, TenantPhase::Running) {
        tcp.spec.replicas
    } else {
        0
    };
    CapiTenantStatus {
        control_plane_endpoint: ep,
        bootstrap: CapiBootstrapStatus {
            ready,
            initialized,
            data_secret_name: Some(format!("{}-kubeconfig", tcp.name)),
            failure_reason: fail_reason,
            failure_message: fail_msg,
        },
        infrastructure_ref: infra,
        ready_replicas,
        replicas: tcp.spec.replicas,
        unavailable_replicas: tcp.spec.replicas.saturating_sub(ready_replicas),
    }
}

/// Predicate the CAPI Cluster reconciler uses: `Cluster.spec.controlPlaneRef` ready?
pub fn is_capi_ready(s: &CapiTenantStatus) -> bool {
    s.bootstrap.ready
        && s.bootstrap.initialized
        && !s.control_plane_endpoint.is_zero()
        && s.bootstrap.failure_reason.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TenantSpec, TenantStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn mk_tcp(phase: TenantPhase, endpoint: Option<&str>, replicas: u32) -> TenantControlPlane {
        let ready = matches!(phase, TenantPhase::Running);
        TenantControlPlane {
            id: Uuid::new_v4(),
            name: "demo".into(),
            namespace: "ns".into(),
            spec: TenantSpec {
                kubernetes_version: "v1.30.0".into(),
                data_store: "etcd".into(),
                replicas,
            },
            status: TenantStatus {
                phase,
                api_server_endpoint: endpoint.map(String::from),
                ready,
                message: None,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn parse_endpoint_https_host_port() {
        let ep = parse_control_plane_endpoint("https://api.example.com:6443");
        assert_eq!(ep.host, "api.example.com");
        assert_eq!(ep.port, 6443);
    }

    #[test]
    fn parse_endpoint_bare_host_port() {
        let ep = parse_control_plane_endpoint("api.example.com:6443");
        assert_eq!(ep.host, "api.example.com");
        assert_eq!(ep.port, 6443);
    }

    #[test]
    fn parse_endpoint_malformed_returns_zero() {
        assert!(parse_control_plane_endpoint("not-a-url").is_zero());
        assert!(parse_control_plane_endpoint("").is_zero());
        assert!(parse_control_plane_endpoint(":6443").is_zero());
    }

    #[test]
    fn build_capi_status_running_marks_ready() {
        let tcp = mk_tcp(TenantPhase::Running, Some("https://api:6443"), 3);
        let s = build_capi_status(&tcp, None);
        assert!(s.bootstrap.ready);
        assert!(s.bootstrap.initialized);
        assert_eq!(s.ready_replicas, 3);
        assert_eq!(s.replicas, 3);
        assert_eq!(s.unavailable_replicas, 0);
        assert_eq!(s.bootstrap.data_secret_name.as_deref(), Some("demo-kubeconfig"));
        assert!(is_capi_ready(&s));
    }

    #[test]
    fn build_capi_status_provisioning_not_ready() {
        let tcp = mk_tcp(TenantPhase::Provisioning, None, 3);
        let s = build_capi_status(&tcp, None);
        assert!(!s.bootstrap.ready);
        assert!(!s.bootstrap.initialized);
        assert_eq!(s.ready_replicas, 0);
        assert_eq!(s.unavailable_replicas, 3);
        assert!(!is_capi_ready(&s));
    }

    #[test]
    fn build_capi_status_failed_records_reason() {
        let mut tcp = mk_tcp(TenantPhase::Failed, Some("https://api:6443"), 1);
        tcp.status.message = Some("etcd unhealthy".into());
        let s = build_capi_status(&tcp, None);
        assert!(!s.bootstrap.ready);
        assert!(s.bootstrap.initialized);
        assert_eq!(s.bootstrap.failure_reason.as_deref(), Some("ControlPlaneFailed"));
        assert_eq!(s.bootstrap.failure_message.as_deref(), Some("etcd unhealthy"));
        assert!(!is_capi_ready(&s));
    }

    #[test]
    fn capi_status_includes_infrastructure_ref_when_supplied() {
        let tcp = mk_tcp(TenantPhase::Running, Some("https://api:6443"), 1);
        let infra = InfrastructureRef {
            api_version: "infrastructure.cluster.x-k8s.io/v1beta1".into(),
            kind: "AWSCluster".into(),
            name: "demo-infra".into(),
            namespace: "ns".into(),
        };
        let s = build_capi_status(&tcp, Some(infra.clone()));
        assert_eq!(s.infrastructure_ref.as_ref().unwrap().kind, "AWSCluster");
        assert_eq!(s.infrastructure_ref.unwrap().name, "demo-infra");
    }

    #[test]
    fn is_capi_ready_requires_endpoint_and_no_failure() {
        let mut s = CapiTenantStatus {
            control_plane_endpoint: ControlPlaneEndpoint {
                host: "api".into(),
                port: 6443,
            },
            bootstrap: CapiBootstrapStatus {
                ready: true,
                initialized: true,
                data_secret_name: None,
                failure_reason: None,
                failure_message: None,
            },
            infrastructure_ref: None,
            ready_replicas: 1,
            replicas: 1,
            unavailable_replicas: 0,
        };
        assert!(is_capi_ready(&s));
        s.bootstrap.failure_reason = Some("boom".into());
        assert!(!is_capi_ready(&s));
        s.bootstrap.failure_reason = None;
        s.control_plane_endpoint = ControlPlaneEndpoint::default();
        assert!(!is_capi_ready(&s));
    }

    #[test]
    fn endpoint_to_url_format() {
        let ep = ControlPlaneEndpoint {
            host: "api.x".into(),
            port: 6443,
        };
        assert_eq!(ep.to_url(), "https://api.x:6443");
    }
}
