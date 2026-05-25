// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Selector emission rules
// line-ported from pkg/agent/plugin/workloadattestor/k8s/k8s.go +
// pkg/agent/plugin/nodeattestor/k8s_psat/psat.go.
//
//! K8s workload + node (PSAT) attestor.

use crate::attestor::WorkloadAttestor;
use crate::error::{IdentityError, Result};
use crate::models::{Selector, SpiffeId};
use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Snapshot of a k8s pod — comes from kubelet `/pods` summary endpoint or
/// the API-server in real deployments.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct K8sPodInfo {
    pub namespace: String,
    pub pod_name: String,
    pub pod_uid: String,
    pub service_account: String,
    pub node_name: String,
    pub pod_labels: BTreeMap<String, String>,
    pub pod_annotations: BTreeMap<String, String>,
    pub container_id: String,
    pub container_name: String,
    pub container_image: String,
    pub container_image_id: String,
}

/// K8s workload attestor — driven by an in-memory pod table keyed by pid.
pub struct K8sWorkloadAttestor {
    pub by_pid: DashMap<i32, K8sPodInfo>,
}

impl Default for K8sWorkloadAttestor {
    fn default() -> Self {
        Self {
            by_pid: DashMap::new(),
        }
    }
}

impl K8sWorkloadAttestor {
    pub fn register(&self, pid: i32, info: K8sPodInfo) {
        self.by_pid.insert(pid, info);
    }
    pub fn forget(&self, pid: i32) {
        self.by_pid.remove(&pid);
    }
}

/// Generates the SPIRE-compatible selector list for a pod (the set is
/// stable, matching `pkg/agent/plugin/workloadattestor/k8s/k8s.go::Attest`).
pub fn pod_selectors(info: &K8sPodInfo) -> Vec<Selector> {
    let mut out = Vec::new();
    out.push(Selector::new("k8s", format!("ns:{}", info.namespace)));
    out.push(Selector::new("k8s", format!("pod-name:{}", info.pod_name)));
    out.push(Selector::new("k8s", format!("pod-uid:{}", info.pod_uid)));
    out.push(Selector::new("k8s", format!("sa:{}", info.service_account)));
    out.push(Selector::new("k8s", format!("node-name:{}", info.node_name)));
    out.push(Selector::new(
        "k8s",
        format!("container-name:{}", info.container_name),
    ));
    out.push(Selector::new(
        "k8s",
        format!("container-image:{}", info.container_image),
    ));
    out.push(Selector::new(
        "k8s",
        format!("container-image-id:{}", info.container_image_id),
    ));
    out.push(Selector::new(
        "k8s",
        format!("container-id:{}", info.container_id),
    ));
    for (k, v) in &info.pod_labels {
        out.push(Selector::new("k8s", format!("pod-label:{}={}", k, v)));
    }
    for (k, v) in &info.pod_annotations {
        out.push(Selector::new("k8s", format!("pod-annotation:{}={}", k, v)));
    }
    out
}

#[async_trait]
impl WorkloadAttestor for K8sWorkloadAttestor {
    fn name(&self) -> &str {
        "k8s"
    }
    async fn attest(&self, pid: i32) -> Result<Vec<Selector>> {
        let info = self
            .by_pid
            .get(&pid)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::AttestationFailed(format!("k8s pid {} unknown", pid)))?;
        Ok(pod_selectors(&info))
    }
}

/// `k8s_psat` node attestor — relies on a projected service-account token
/// (PSAT). The token + audience are validated and the corresponding agent
/// SPIFFE ID is `spiffe://<td>/spire/agent/k8s_psat/<cluster>/<node-uid>`.
pub struct K8sPsatNodeAttestor {
    pub trust_domain: String,
    pub cluster: String,
    /// Synthetic SA-token table: `token -> (namespace, sa, node-uid)`.
    pub tokens: DashMap<String, K8sTokenClaims>,
}

#[derive(Debug, Clone)]
pub struct K8sTokenClaims {
    pub namespace: String,
    pub service_account: String,
    pub pod_uid: String,
    pub node_uid: String,
    pub audience: Vec<String>,
}

impl K8sPsatNodeAttestor {
    pub fn new(trust_domain: impl Into<String>, cluster: impl Into<String>) -> Self {
        Self {
            trust_domain: trust_domain.into(),
            cluster: cluster.into(),
            tokens: DashMap::new(),
        }
    }

    pub fn attest_token(&self, token: &str, expected_audience: &str) -> Result<(SpiffeId, Vec<Selector>)> {
        let claims = self
            .tokens
            .get(token)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::AttestationFailed("psat: unknown token".into()))?;
        if !claims.audience.iter().any(|a| a == expected_audience) {
            return Err(IdentityError::AttestationFailed(format!(
                "psat: audience mismatch (want {}; got {:?})",
                expected_audience, claims.audience
            )));
        }
        let id = SpiffeId::new(format!(
            "spiffe://{}/spire/agent/k8s_psat/{}/{}",
            self.trust_domain, self.cluster, claims.node_uid
        ));
        let selectors = vec![
            Selector::new("k8s", format!("ns:{}", claims.namespace)),
            Selector::new("k8s", format!("sa:{}", claims.service_account)),
            Selector::new("k8s", format!("pod-uid:{}", claims.pod_uid)),
            Selector::new("k8s", format!("node-uid:{}", claims.node_uid)),
            Selector::new("k8s", format!("cluster:{}", self.cluster)),
        ];
        Ok((id, selectors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pod() -> K8sPodInfo {
        let mut labels = BTreeMap::new();
        labels.insert("app".into(), "frontend".into());
        labels.insert("tier".into(), "web".into());
        let mut anno = BTreeMap::new();
        anno.insert("svc.example/owner".into(), "team-a".into());
        K8sPodInfo {
            namespace: "prod".into(),
            pod_name: "frontend-abc".into(),
            pod_uid: "uid-1".into(),
            service_account: "default".into(),
            node_name: "node-1".into(),
            pod_labels: labels,
            pod_annotations: anno,
            container_id: "containerd://aaa".into(),
            container_name: "main".into(),
            container_image: "frontend:1.0".into(),
            container_image_id: "sha256:1234".into(),
        }
    }

    #[test]
    fn pod_selectors_complete() {
        let s = pod_selectors(&sample_pod());
        assert!(s.iter().any(|x| x.value == "ns:prod"));
        assert!(s.iter().any(|x| x.value == "sa:default"));
        assert!(s.iter().any(|x| x.value == "pod-label:app=frontend"));
        assert!(s
            .iter()
            .any(|x| x.value == "pod-annotation:svc.example/owner=team-a"));
        assert!(s.iter().any(|x| x.value == "container-name:main"));
    }

    #[tokio::test]
    async fn attestor_returns_selectors_by_pid() {
        let a = K8sWorkloadAttestor::default();
        a.register(99, sample_pod());
        let s = a.attest(99).await.unwrap();
        assert!(s.iter().any(|x| x.value == "ns:prod"));
    }

    #[tokio::test]
    async fn attestor_missing_pid_errors() {
        let a = K8sWorkloadAttestor::default();
        assert!(matches!(
            a.attest(0).await,
            Err(IdentityError::AttestationFailed(_))
        ));
    }

    #[test]
    fn psat_validates_audience() {
        let n = K8sPsatNodeAttestor::new("example.org", "prod-cluster");
        n.tokens.insert(
            "tok-1".into(),
            K8sTokenClaims {
                namespace: "spire".into(),
                service_account: "spire-agent".into(),
                pod_uid: "po".into(),
                node_uid: "no".into(),
                audience: vec!["spire-server".into()],
            },
        );
        let (id, selectors) = n.attest_token("tok-1", "spire-server").unwrap();
        assert_eq!(
            id.as_str(),
            "spiffe://example.org/spire/agent/k8s_psat/prod-cluster/no"
        );
        assert!(selectors
            .iter()
            .any(|s| s.value == "cluster:prod-cluster"));
    }

    #[test]
    fn psat_rejects_wrong_audience() {
        let n = K8sPsatNodeAttestor::new("example.org", "prod-cluster");
        n.tokens.insert(
            "tok-1".into(),
            K8sTokenClaims {
                namespace: "spire".into(),
                service_account: "spire-agent".into(),
                pod_uid: "po".into(),
                node_uid: "no".into(),
                audience: vec!["spire-server".into()],
            },
        );
        assert!(n.attest_token("tok-1", "other").is_err());
    }

    #[test]
    fn psat_rejects_unknown_token() {
        let n = K8sPsatNodeAttestor::new("example.org", "c");
        assert!(n.attest_token("missing", "spire-server").is_err());
    }
}
