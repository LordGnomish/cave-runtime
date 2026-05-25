// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sealed Secrets — bitnami-labs/sealed-secrets v0.37.0 deep-port (Apache-2.0).
//!
//! source_sha = `8e4ed463552a6a6462648a9ff090a1f42abbda30`.
//!
//! Implemented surfaces:
//!   * `pkg/apis/sealedsecrets/v1alpha1/sealedsecret_types.go`
//!   * `pkg/crypto/crypto.go`            (HybridEncrypt / HybridDecrypt)
//!   * `pkg/controller/keys.go`          (key rotation state)
//!   * `pkg/kubeseal/kubeseal.go`        (raw / strict / namespace-wide scope)
//!
//! Out of scope (`scope_cut_to`):
//!   * `cmd/controller`       → cave-policy-controller (k8s controller-runtime)
//!   * `cmd/kubeseal`         → cave-cli
//!   * `helm-chart-bootstrap` → cave-deploy
//!   * `prometheus-metrics`   → cave-metrics

pub mod controller;
pub mod crypto;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    /// Strict (default): name + namespace + data all sealed together.
    Strict,
    /// Namespace-wide: any Secret name within the namespace can be decrypted.
    Namespace,
    /// Cluster-wide: any Secret name in any namespace can be decrypted.
    Cluster,
}

impl Default for Scope {
    fn default() -> Self {
        Self::Strict
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecret {
    pub api_version: String,
    pub kind: String,
    pub metadata: SealedSecretMeta,
    pub spec: SealedSecretSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecretMeta {
    pub name: String,
    pub namespace: String,
    #[serde(default)]
    pub annotations: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecretSpec {
    /// Map of key → base64(ciphertext).
    pub encrypted_data: std::collections::BTreeMap<String, String>,
    /// `template` is the Secret template (metadata + type) that the controller
    /// will produce on successful unseal.
    pub template: SecretTemplate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretTemplate {
    #[serde(default)]
    pub metadata: TemplateMeta,
    #[serde(default = "default_secret_type")]
    pub r#type: String,
}

fn default_secret_type() -> String {
    "Opaque".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateMeta {
    #[serde(default)]
    pub annotations: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

/// Scope is encoded in annotations on the SealedSecret manifest.
pub fn scope_from_annotations(
    annotations: &std::collections::BTreeMap<String, String>,
) -> Scope {
    if annotations
        .get("sealedsecrets.bitnami.com/cluster-wide")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        return Scope::Cluster;
    }
    if annotations
        .get("sealedsecrets.bitnami.com/namespace-wide")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        return Scope::Namespace;
    }
    Scope::Strict
}

/// `pkg/crypto/crypto.go` — assembled per-secret HKDF binding label.
///
/// label = namespace || "/" || name      (Strict)
///       = namespace                     (Namespace)
///       = ""                            (Cluster)
pub fn binding_label(scope: Scope, namespace: &str, name: &str) -> Vec<u8> {
    match scope {
        Scope::Strict => format!("{namespace}/{name}").into_bytes(),
        Scope::Namespace => namespace.as_bytes().to_vec(),
        Scope::Cluster => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_default_strict() {
        let s: Scope = Default::default();
        assert_eq!(s, Scope::Strict);
    }

    #[test]
    fn scope_from_annotations_cluster() {
        let mut a = std::collections::BTreeMap::new();
        a.insert(
            "sealedsecrets.bitnami.com/cluster-wide".into(),
            "true".into(),
        );
        assert_eq!(scope_from_annotations(&a), Scope::Cluster);
    }

    #[test]
    fn scope_from_annotations_namespace() {
        let mut a = std::collections::BTreeMap::new();
        a.insert(
            "sealedsecrets.bitnami.com/namespace-wide".into(),
            "true".into(),
        );
        assert_eq!(scope_from_annotations(&a), Scope::Namespace);
    }

    #[test]
    fn binding_label_strict() {
        let l = binding_label(Scope::Strict, "default", "my-secret");
        assert_eq!(l, b"default/my-secret");
    }

    #[test]
    fn binding_label_namespace() {
        let l = binding_label(Scope::Namespace, "default", "my-secret");
        assert_eq!(l, b"default");
    }

    #[test]
    fn binding_label_cluster() {
        let l = binding_label(Scope::Cluster, "default", "my-secret");
        assert!(l.is_empty());
    }

    #[test]
    fn sealed_secret_yaml_round_trip() {
        let ss = SealedSecret {
            api_version: "bitnami.com/v1alpha1".into(),
            kind: "SealedSecret".into(),
            metadata: SealedSecretMeta {
                name: "mysecret".into(),
                namespace: "default".into(),
                annotations: Default::default(),
            },
            spec: SealedSecretSpec {
                encrypted_data: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("password".into(), "AwBkAGYAZQ==".into());
                    m
                },
                template: SecretTemplate {
                    metadata: Default::default(),
                    r#type: "Opaque".into(),
                },
            },
        };
        let s = serde_json::to_string(&ss).unwrap();
        let back: SealedSecret = serde_json::from_str(&s).unwrap();
        assert_eq!(back.metadata.name, "mysecret");
    }
}
