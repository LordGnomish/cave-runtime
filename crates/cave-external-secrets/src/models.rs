use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    AwsSecretsManager,
    AwsParameterStore,
    GcpSecretManager,
    AzureKeyVault,
    HashicorpVault,
    Kubernetes,
    Fake,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SecretStoreScope {
    Namespaced,
    Cluster,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SecretStoreStatus {
    Valid,
    Invalid,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStore {
    pub id: Uuid,
    pub name: String,
    pub namespace: Option<String>,
    pub scope: SecretStoreScope,
    pub provider: ProviderType,
    pub provider_config: serde_json::Value,
    pub refresh_interval_secs: u64,
    pub status: SecretStoreStatus,
    pub status_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSecret {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub secret_store_ref: SecretStoreRef,
    pub target: ExternalSecretTarget,
    pub data: Vec<ExternalSecretData>,
    pub data_from: Vec<ExternalSecretDataFrom>,
    pub refresh_interval_secs: u64,
    pub status: SyncStatus,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub synced_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStoreRef {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSecretTarget {
    pub name: String,
    pub creation_policy: CreationPolicy,
    pub deletion_policy: DeletionPolicy,
    pub template: Option<SecretTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum CreationPolicy {
    Owner,
    Merge,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum DeletionPolicy {
    Delete,
    Merge,
    Retain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretTemplate {
    pub metadata: Option<HashMap<String, String>>,
    pub secret_type: Option<String>,
    pub data: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSecretData {
    pub secret_key: String,
    pub remote_ref: RemoteRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSecretDataFrom {
    pub extract: Option<RemoteRef>,
    pub find: Option<FindSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRef {
    pub key: String,
    pub version: Option<String>,
    pub property: Option<String>,
    pub conversion_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindSpec {
    pub name: Option<FindByName>,
    pub path: Option<String>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindByName {
    pub regexp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecret {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub secret_store_refs: Vec<SecretStoreRef>,
    pub selector: PushSecretSelector,
    pub data: Vec<PushSecretData>,
    pub status: SyncStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecretSelector {
    pub secret: SecretName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretName {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecretData {
    pub match_: PushSecretMatch,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSecretMatch {
    pub secret_key: String,
    pub remote_ref: PushRemoteRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRemoteRef {
    pub remote_key: String,
    pub property: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub secret_name: String,
    pub namespace: String,
    pub keys_synced: Vec<String>,
    pub synced_at: DateTime<Utc>,
    pub version: String,
}

// Request types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSecretStoreRequest {
    pub name: String,
    pub namespace: Option<String>,
    pub scope: Option<SecretStoreScope>,
    pub provider: ProviderType,
    pub provider_config: serde_json::Value,
    pub refresh_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExternalSecretRequest {
    pub name: String,
    pub namespace: String,
    pub secret_store_ref: SecretStoreRef,
    pub target: ExternalSecretTarget,
    pub data: Vec<ExternalSecretData>,
    pub data_from: Option<Vec<ExternalSecretDataFrom>>,
    pub refresh_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePushSecretRequest {
    pub name: String,
    pub namespace: String,
    pub secret_store_refs: Vec<SecretStoreRef>,
    pub selector: PushSecretSelector,
    pub data: Vec<PushSecretData>,
}
