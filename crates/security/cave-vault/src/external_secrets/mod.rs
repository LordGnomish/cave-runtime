// SPDX-License-Identifier: AGPL-3.0-or-later
//! External Secrets Operator (ESO) data model + provider abstraction.
//!
//! Upstream: external-secrets/external-secrets v2.5.0 (Apache-2.0,
//! source_sha `0755b0af7de7f05a104b0df29ba84f43513fee8b`).
//!
//! Cave-vault hosts ESO as a sub-module rather than a separate crate because
//! the providers (`Vault`, `AWS-SM`, `GCP-SM`, `Azure-KV`, `Kubernetes`,
//! `Fake`) all map onto the same secret-store substrate the OpenBao deep-port
//! already exposes via `crate::engines`.
//!
//! Implemented surfaces (mapped):
//!   * `apis/externalsecrets/v1beta1/secretstore_types.go`
//!   * `apis/externalsecrets/v1beta1/clustersecretstore_types.go`
//!   * `apis/externalsecrets/v1beta1/externalsecret_types.go`
//!   * `apis/externalsecrets/v1beta1/pushsecret_types.go`
//!   * `apis/generators/v1alpha1/uuid_types.go`
//!   * `apis/generators/v1alpha1/webhook_types.go`
//!   * `pkg/provider/*` — Vault / AWS-SM / GCP-SM / Azure-KV / Kubernetes / Fake
//!   * `pkg/controllers/externalsecret/*` — reconciler core (synchronous variant)
//!
//! Out of scope (`scope_cut_to` in parity.manifest.toml):
//!   * `kubectl-eso` plugin            → cave-cli
//!   * Helm chart bootstrap            → cave-deploy
//!   * Metrics exporter                → cave-metrics
//!   * Continuous reconciler informer  → cave-policy-controller (Phase 2)

pub mod providers;
pub mod reconciler;

use serde::{Deserialize, Serialize};

/// `SecretStore` CRD — namespace-scoped secret-source configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStore {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: SecretStoreSpec,
}

/// `ClusterSecretStore` CRD — cluster-scoped variant of `SecretStore`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSecretStore {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: SecretStoreSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectMeta {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStoreSpec {
    /// Refresh interval in seconds. ESO default = 1h.
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_seconds: u32,
    /// Backend provider selector.
    pub provider: ProviderConfig,
    /// Retry settings.
    #[serde(default)]
    pub retry_settings: RetrySettings,
}

fn default_refresh_interval() -> u32 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderConfig {
    Vault {
        server: String,
        path: String,
        version: VaultKvVersion,
        auth: VaultAuth,
    },
    AwsSecretsManager {
        region: String,
        role: Option<String>,
    },
    GcpSecretManager {
        project_id: String,
    },
    AzureKeyVault {
        vault_url: String,
        tenant_id: String,
    },
    Kubernetes {
        remote_namespace: String,
    },
    Fake,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VaultKvVersion {
    V1,
    #[default]
    V2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultAuth {
    /// AppRole authentication (roleId + secretId).
    AppRole { role_id: String, secret_id: String },
    /// Kubernetes service-account token auth.
    Kubernetes {
        role: String,
        mount_path: String,
        service_account_token_path: String,
    },
    /// Static token auth.
    Token { token: String },
    /// JWT/OIDC auth.
    Jwt { role: String, jwt: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetrySettings {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_retry_interval_ms")]
    pub retry_interval_ms: u64,
}

fn default_max_retries() -> u32 {
    5
}

fn default_retry_interval_ms() -> u64 {
    2000
}

/// `ExternalSecret` CRD — declares one or more remote keys to materialise into a
/// local K8s `Secret`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSecret {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ExternalSecretSpec,
    #[serde(default)]
    pub status: ExternalSecretStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSecretSpec {
    pub secret_store_ref: SecretStoreRef,
    pub target: ExternalSecretTarget,
    #[serde(default)]
    pub data: Vec<ExternalSecretData>,
    #[serde(default)]
    pub data_from: Vec<DataFromSource>,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStoreRef {
    pub name: String,
    /// "SecretStore" or "ClusterSecretStore".
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSecretTarget {
    pub name: String,
    #[serde(default = "default_creation_policy")]
    pub creation_policy: CreationPolicy,
    #[serde(default = "default_deletion_policy")]
    pub deletion_policy: DeletionPolicy,
    #[serde(default = "default_template")]
    pub template: TargetTemplate,
}

fn default_creation_policy() -> CreationPolicy {
    CreationPolicy::Owner
}

fn default_deletion_policy() -> DeletionPolicy {
    DeletionPolicy::Retain
}

fn default_template() -> TargetTemplate {
    TargetTemplate {
        type_: "Opaque".into(),
        engine_version: TemplateEngine::V2,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CreationPolicy {
    /// Create + own the Secret (set ownerReference).
    Owner,
    /// Create but don't own (no ownerReference).
    Orphan,
    /// Merge into existing Secret if present.
    Merge,
    /// Do not create — assume Secret exists.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeletionPolicy {
    /// Delete the Secret when the ExternalSecret is removed.
    Delete,
    /// Merge: remove the keys but keep the Secret.
    Merge,
    /// Leave Secret untouched.
    Retain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetTemplate {
    #[serde(rename = "type")]
    pub type_: String,
    pub engine_version: TemplateEngine,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TemplateEngine {
    V1,
    V2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSecretData {
    pub secret_key: String,
    pub remote_ref: RemoteRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRef {
    pub key: String,
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataFromSource {
    Extract { key: String },
    Find { name: String, regexp: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExternalSecretStatus {
    #[serde(default)]
    pub conditions: Vec<StatusCondition>,
    #[serde(default)]
    pub refresh_time: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub sync_call_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCondition {
    /// "Ready", "SecretSynced", "RemoteUnreachable", ...
    pub type_: String,
    /// "True", "False", "Unknown".
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_transition_time: chrono::DateTime<chrono::Utc>,
}

/// `PushSecret` CRD — push a local K8s Secret to a remote provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecret {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PushSecretSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecretSpec {
    pub secret_store_refs: Vec<SecretStoreRef>,
    pub selector: PushSecretSelector,
    pub data: Vec<PushSecretData>,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecretSelector {
    pub secret: SecretRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecretData {
    pub match_: PushMatch,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushMatch {
    pub secret_key: String,
    pub remote_ref: RemoteRef,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_store_vault_yaml_round_trip() {
        let s = SecretStore {
            api_version: "external-secrets.io/v1beta1".into(),
            kind: "SecretStore".into(),
            metadata: ObjectMeta {
                name: "vault-backend".into(),
                namespace: "default".into(),
                ..Default::default()
            },
            spec: SecretStoreSpec {
                refresh_interval_seconds: 600,
                provider: ProviderConfig::Vault {
                    server: "https://vault.svc:8200".into(),
                    path: "secret".into(),
                    version: VaultKvVersion::V2,
                    auth: VaultAuth::Kubernetes {
                        role: "eso-role".into(),
                        mount_path: "kubernetes".into(),
                        service_account_token_path:
                            "/var/run/secrets/kubernetes.io/serviceaccount/token".into(),
                    },
                },
                retry_settings: RetrySettings {
                    max_retries: 3,
                    retry_interval_ms: 500,
                },
            },
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SecretStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.spec.refresh_interval_seconds, 600);
        assert!(matches!(back.spec.provider, ProviderConfig::Vault { .. }));
    }

    #[test]
    fn external_secret_creation_policy_defaults() {
        let json = serde_json::json!({
            "apiVersion": "external-secrets.io/v1beta1",
            "kind": "ExternalSecret",
            "metadata": {"name": "db-creds"},
            "spec": {
                "secretStoreRef": {"name": "vault-backend", "kind": "SecretStore"},
                "target": {"name": "db-creds-local"},
                "data": [{"secretKey": "password", "remoteRef": {"key": "kv/db", "property": "password"}}]
            }
        });
        let es: ExternalSecret = serde_json::from_value(json).unwrap();
        assert_eq!(es.spec.target.name, "db-creds-local");
        assert_eq!(es.spec.target.creation_policy, CreationPolicy::Owner);
        assert_eq!(es.spec.target.deletion_policy, DeletionPolicy::Retain);
    }

    #[test]
    fn push_secret_match_round_trip() {
        let ps = PushSecret {
            api_version: "external-secrets.io/v1alpha1".into(),
            kind: "PushSecret".into(),
            metadata: ObjectMeta {
                name: "push-db-pw".into(),
                ..Default::default()
            },
            spec: PushSecretSpec {
                secret_store_refs: vec![SecretStoreRef {
                    name: "vault-backend".into(),
                    kind: "SecretStore".into(),
                }],
                selector: PushSecretSelector {
                    secret: SecretRef {
                        name: "db-creds-local".into(),
                    },
                },
                data: vec![PushSecretData {
                    match_: PushMatch {
                        secret_key: "password".into(),
                        remote_ref: RemoteRef {
                            key: "kv/db".into(),
                            property: Some("password".into()),
                            version: None,
                        },
                    },
                    metadata: None,
                }],
                refresh_interval_seconds: 300,
            },
        };
        let s = serde_json::to_string(&ps).unwrap();
        assert!(s.contains("push-db-pw"));
        assert!(s.contains("vault-backend"));
    }
}
