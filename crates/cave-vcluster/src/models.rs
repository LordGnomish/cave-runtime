use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub const DEFAULT_CPU_LIMIT: &str = "2000m";
pub const DEFAULT_MEMORY_LIMIT: &str = "4Gi";
pub const DEFAULT_TTL_SECS: u64 = 4 * 3600; // 4 hours
pub const MAX_CLUSTERS_PER_NAMESPACE: u32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VCluster {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub pr_number: Option<u32>,
    pub branch: Option<String>,
    pub spec: VClusterSpec,
    pub status: VClusterStatus,
    pub kubeconfig: Option<String>,
    pub api_server_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VClusterSpec {
    pub cpu_limit: String,
    pub memory_limit: String,
    pub ttl_secs: u64,
    pub k8s_version: Option<String>,
    pub tenant_id: String,
    pub synced_resources: Vec<String>,
}

impl Default for VClusterSpec {
    fn default() -> Self {
        Self {
            cpu_limit: DEFAULT_CPU_LIMIT.into(),
            memory_limit: DEFAULT_MEMORY_LIMIT.into(),
            ttl_secs: DEFAULT_TTL_SECS,
            k8s_version: None,
            tenant_id: "default".into(),
            synced_resources: vec!["ConfigMap".into(), "Secret".into(), "ServiceAccount".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VClusterStatus {
    Pending,
    Provisioning,
    Running,
    Suspended,
    Terminating,
    Failed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncedResource {
    pub id: Uuid,
    pub cluster_name: String,
    pub namespace: String,
    pub resource_kind: String,
    pub resource_name: String,
    pub synced_at: DateTime<Utc>,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantIsolation {
    pub tenant_id: String,
    pub namespace_prefix: String,
    pub network_policy: NetworkPolicyMode,
    pub rbac_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicyMode {
    Strict,
    Permissive,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateClusterRequest {
    pub name: String,
    pub namespace: String,
    pub pr_number: Option<u32>,
    pub branch: Option<String>,
    pub spec: Option<VClusterSpec>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaStatus {
    pub namespace: String,
    pub current_count: u32,
    pub max_count: u32,
    pub available: u32,
}
