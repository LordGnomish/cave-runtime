// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! kubeconfig generation for cluster access.

use crate::cluster::Cluster;
use crate::error::ClusterResult;
use serde::{Deserialize, Serialize};

/// A minimal kubeconfig structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kubeconfig {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub clusters: Vec<KubeconfigCluster>,
    pub users: Vec<KubeconfigUser>,
    pub contexts: Vec<KubeconfigContext>,
    #[serde(rename = "current-context")]
    pub current_context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigCluster {
    pub name: String,
    pub cluster: KubeconfigClusterData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigClusterData {
    pub server: String,
    #[serde(rename = "certificate-authority-data")]
    pub ca_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigUser {
    pub name: String,
    pub user: KubeconfigUserData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigUserData {
    pub token: Option<String>,
    #[serde(rename = "client-certificate-data", skip_serializing_if = "Option::is_none")]
    pub client_cert: Option<String>,
    #[serde(rename = "client-key-data", skip_serializing_if = "Option::is_none")]
    pub client_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigContext {
    pub name: String,
    pub context: KubeconfigContextData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigContextData {
    pub cluster: String,
    pub user: String,
    pub namespace: Option<String>,
}

// ── Token types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CredentialType {
    ServiceAccountToken(String),
    ClientCertificate { cert: String, key: String },
}

// ── Generator ─────────────────────────────────────────────────────────────────

/// Generate a kubeconfig for the given cluster and credentials.
pub fn generate(cluster: &Cluster, credential: CredentialType) -> ClusterResult<Kubeconfig> {
    let cluster_name = &cluster.spec.name;
    let user_name = format!("{cluster_name}-admin");
    let context_name = cluster_name.clone();

    let user_data = match credential {
        CredentialType::ServiceAccountToken(token) => KubeconfigUserData {
            token: Some(token),
            client_cert: None,
            client_key: None,
        },
        CredentialType::ClientCertificate { cert, key } => KubeconfigUserData {
            token: None,
            client_cert: Some(cert),
            client_key: Some(key),
        },
    };

    Ok(Kubeconfig {
        api_version: "v1".into(),
        kind: "Config".into(),
        clusters: vec![KubeconfigCluster {
            name: cluster_name.clone(),
            cluster: KubeconfigClusterData {
                server: cluster.api_endpoint.clone(),
                ca_data: cluster.ca_data.clone(),
            },
        }],
        users: vec![KubeconfigUser {
            name: user_name.clone(),
            user: user_data,
        }],
        contexts: vec![KubeconfigContext {
            name: context_name.clone(),
            context: KubeconfigContextData {
                cluster: cluster_name.clone(),
                user: user_name,
                namespace: Some("default".into()),
            },
        }],
        current_context: context_name,
    })
}

/// Serialise a kubeconfig to YAML string.
pub fn to_yaml(kc: &Kubeconfig) -> ClusterResult<String> {
    serde_yaml::to_string(kc)
        .map_err(|e| crate::error::ClusterError::KubeconfigFailed(e.to_string()))
}

/// Generate a service account token (placeholder — real impl would call K8s API).
pub fn generate_token(cluster: &Cluster, service_account: &str, namespace: &str) -> String {
    format!(
        "cave-token.{}.{}.{}.{}",
        cluster.spec.name,
        service_account,
        namespace,
        cluster.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterSpec, ClusterStatus, NetworkConfig};
    use std::collections::HashMap;

    fn test_cluster() -> Cluster {
        let spec = ClusterSpec {
            name: "test-cluster".into(),
            kubernetes_version: "1.30".into(),
            region: "eu-west-1".into(),
            network: NetworkConfig::default(),
            tags: HashMap::new(),
            enable_rbac: true,
            audit_logging: false,
        };
        Cluster::new(spec, "alice".into())
    }

    #[test]
    fn generate_with_token() {
        let cluster = test_cluster();
        let kc = generate(
            &cluster,
            CredentialType::ServiceAccountToken("my-token".into()),
        )
        .unwrap();
        assert_eq!(kc.clusters[0].cluster.server, cluster.api_endpoint);
        assert_eq!(kc.users[0].user.token, Some("my-token".into()));
        assert_eq!(kc.current_context, "test-cluster");
    }

    #[test]
    fn kubeconfig_yaml_serialisation() {
        let cluster = test_cluster();
        let kc = generate(&cluster, CredentialType::ServiceAccountToken("tok".into())).unwrap();
        let yaml = to_yaml(&kc).unwrap();
        assert!(yaml.contains("apiVersion: v1"));
        assert!(yaml.contains("kind: Config"));
        assert!(yaml.contains("test-cluster"));
    }
}
