// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! kubeadm init bootstrap renderer.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/utilities/kubeadm/*.go
//!
//! Kamaji uses a templated kubeadm config to bootstrap the tenant's
//! initial control-plane state. The Cave port renders the same
//! `InitConfiguration` + `ClusterConfiguration` YAML pair so the
//! kubeadm-init-as-Job pattern stays compatible with upstream tooling.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeadmConfig {
    pub cluster_name: String,
    pub kubernetes_version: String,
    pub pod_subnet: String,
    pub service_subnet: String,
    pub api_advertise_address: String,
    pub control_plane_endpoint: String,
}

/// Render the kubeadm init YAML — two documents (`InitConfiguration`
/// then `ClusterConfiguration`) separated by `---`. The text is stable
/// (sorted keys, no map-ordering wobble) so callers can diff for drift.
pub fn render_kubeadm_init_config(cfg: &KubeadmConfig) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("apiVersion: kubeadm.k8s.io/v1beta3\n");
    out.push_str("kind: InitConfiguration\n");
    out.push_str("localAPIEndpoint:\n");
    out.push_str(&format!(
        "  advertiseAddress: {}\n",
        cfg.api_advertise_address
    ));
    out.push_str("  bindPort: 6443\n");
    out.push_str("---\n");
    out.push_str("apiVersion: kubeadm.k8s.io/v1beta3\n");
    out.push_str("kind: ClusterConfiguration\n");
    out.push_str(&format!("clusterName: {}\n", cfg.cluster_name));
    out.push_str(&format!("kubernetesVersion: {}\n", cfg.kubernetes_version));
    out.push_str(&format!(
        "controlPlaneEndpoint: {}\n",
        cfg.control_plane_endpoint
    ));
    out.push_str("networking:\n");
    out.push_str(&format!("  podSubnet: {}\n", cfg.pod_subnet));
    out.push_str(&format!("  serviceSubnet: {}\n", cfg.service_subnet));
    out.push_str(&format!("  dnsDomain: cluster.local\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> KubeadmConfig {
        KubeadmConfig {
            cluster_name: "t1".into(),
            kubernetes_version: "v1.31.0".into(),
            pod_subnet: "10.244.0.0/16".into(),
            service_subnet: "10.96.0.0/12".into(),
            api_advertise_address: "10.0.0.1".into(),
            control_plane_endpoint: "10.0.0.1:6443".into(),
        }
    }

    #[test]
    fn render_emits_both_documents() {
        let s = render_kubeadm_init_config(&cfg());
        assert!(s.contains("kind: InitConfiguration"));
        assert!(s.contains("kind: ClusterConfiguration"));
        assert!(s.contains("---"));
    }

    #[test]
    fn render_is_deterministic() {
        let a = render_kubeadm_init_config(&cfg());
        let b = render_kubeadm_init_config(&cfg());
        assert_eq!(a, b);
    }
}
