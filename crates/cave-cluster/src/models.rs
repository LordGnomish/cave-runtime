//! Data models for Kubernetes cluster lifecycle management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Cluster ──────────────────────────────────────────────────────────────────

/// A managed Kubernetes cluster owned by a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: Uuid,
    pub name: String,
    pub tenant_id: Uuid,
    pub provider: Provider,
    /// Kubernetes version, e.g. "1.31.0"
    pub version: String,
    pub status: ClusterStatus,
    /// Public API server endpoint, e.g. "https://api.k8s.example.com:6443"
    pub endpoint: Option<String>,
    /// Reference to the encrypted kubeconfig stored in cave-vault.
    pub kubeconfig_ref: Option<KubeconfigRef>,
    pub upgrade_policy: UpgradePolicy,
    pub network: NetworkPolicy,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Hetzner,
    Azure,
    Aws,
    BareMetal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClusterStatus {
    Provisioning,
    Running,
    Upgrading,
    Deleting,
    Error { message: String },
}

// ── Node Pool ─────────────────────────────────────────────────────────────────

/// A group of homogeneous worker nodes within a cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePool {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub name: String,
    /// Cloud instance type or bare metal profile, e.g. "cx31", "Standard_D4s_v3"
    pub instance_type: String,
    pub min_nodes: u32,
    pub max_nodes: u32,
    pub current_nodes: u32,
    pub labels: HashMap<String, String>,
    pub taints: Vec<Taint>,
    pub autoscaling_enabled: bool,
}

/// Kubernetes node taint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: String,
    pub effect: TaintEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

// ── Cluster Template ──────────────────────────────────────────────────────────

/// Predefined cluster configuration for rapid provisioning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterTemplate {
    pub id: Uuid,
    /// e.g. "small", "medium", "large"
    pub name: String,
    pub tier: TemplateTier,
    pub description: String,
    pub default_version: String,
    pub node_pools: Vec<NodePoolTemplate>,
    pub default_addons: Vec<ClusterAddonType>,
}

/// Node pool definition within a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePoolTemplate {
    pub name: String,
    pub instance_type: String,
    pub min_nodes: u32,
    pub max_nodes: u32,
    pub autoscaling_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateTier {
    Dev,
    Staging,
    Production,
}

// ── Tenant ────────────────────────────────────────────────────────────────────

/// A platform tenant that owns one or more clusters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub quota: TenantQuota,
    pub billing_info: BillingInfo,
    pub created_at: DateTime<Utc>,
}

/// Resource limits for a tenant across all their clusters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantQuota {
    pub max_clusters: u32,
    pub max_nodes: u32,
    /// Total CPU cores across all clusters
    pub max_cpu: u32,
    /// Total memory in GiB across all clusters
    pub max_memory_gib: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingInfo {
    pub billing_account_id: String,
    pub cost_center: String,
}

/// Snapshot of a tenant's current resource consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaUsage {
    pub clusters_used: u32,
    pub clusters_limit: u32,
    pub nodes_used: u32,
    pub nodes_limit: u32,
    pub cpu_used: u32,
    pub cpu_limit: u32,
    pub memory_used_gib: u64,
    pub memory_limit_gib: u64,
}

/// Aggregated view of a tenant's entire cluster fleet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantDashboard {
    pub tenant: Tenant,
    pub clusters: Vec<Cluster>,
    pub total_nodes: u32,
    pub quota_usage: QuotaUsage,
}

// ── Cluster Add-on ────────────────────────────────────────────────────────────

/// An installed add-on within a cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterAddon {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub addon_type: ClusterAddonType,
    pub version: String,
    pub status: AddonStatus,
    pub installed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterAddonType {
    IngressNginx,
    CertManager,
    MonitoringStack,
    CaveEbpfAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddonStatus {
    Installing,
    Running,
    Failed,
    Upgrading,
}

// ── Cluster Health ────────────────────────────────────────────────────────────

/// Point-in-time health snapshot of a cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterHealth {
    pub cluster_id: Uuid,
    pub api_server_status: HealthStatus,
    pub etcd_health: HealthStatus,
    pub node_readiness: NodeReadiness,
    pub component_statuses: Vec<ComponentStatus>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeReadiness {
    pub total: u32,
    pub ready: u32,
    pub not_ready: u32,
    pub cordoned: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub name: String,
    pub healthy: bool,
    pub message: Option<String>,
}

// ── Upgrade Policy ────────────────────────────────────────────────────────────

/// Controls how and when cluster upgrades are applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradePolicy {
    pub auto_upgrade: bool,
    pub maintenance_window: Option<MaintenanceWindow>,
    /// Extra nodes to provision during rolling upgrade (0 = in-place)
    pub max_surge: u32,
    pub drain_timeout_seconds: u64,
}

impl Default for UpgradePolicy {
    fn default() -> Self {
        Self {
            auto_upgrade: false,
            maintenance_window: None,
            max_surge: 1,
            drain_timeout_seconds: 300,
        }
    }
}

/// UTC maintenance window for automated operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    /// 0 = Sunday … 6 = Saturday
    pub day_of_week: u8,
    /// Start hour in UTC (0–23)
    pub start_hour: u8,
    pub duration_hours: u8,
}

// ── Cluster Event ─────────────────────────────────────────────────────────────

/// Immutable lifecycle event recorded against a cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterEvent {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub event_type: ClusterEventType,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterEventType {
    Created,
    Upgraded,
    Scaled,
    NodeAdded,
    NodeRemoved,
    AddonInstalled,
    CredentialsRotated,
    Error,
    Deleted,
}

// ── Kubeconfig Ref ────────────────────────────────────────────────────────────

/// Pointer to the encrypted kubeconfig stored in cave-vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeconfigRef {
    /// Vault path, e.g. "clusters/<cluster_id>/kubeconfig"
    pub vault_path: String,
    pub encrypted: bool,
    pub last_rotated: DateTime<Utc>,
}

// ── Network Policy ────────────────────────────────────────────────────────────

/// Per-cluster network configuration and tenant isolation settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Pod CIDR, e.g. "10.244.0.0/16"
    pub pod_cidr: String,
    /// Service CIDR, e.g. "10.96.0.0/12"
    pub service_cidr: String,
    pub cni_plugin: CniPlugin,
    /// Enforce NetworkPolicies that block cross-tenant pod traffic.
    pub tenant_isolation: bool,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            pod_cidr: "10.244.0.0/16".into(),
            service_cidr: "10.96.0.0/12".into(),
            cni_plugin: CniPlugin::Cilium,
            tenant_isolation: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CniPlugin {
    Cilium,
    Flannel,
    Calico,
    WeaveNet,
}

// ── Request / Response DTOs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateClusterRequest {
    pub name: String,
    pub tenant_id: Uuid,
    pub provider: Provider,
    pub version: String,
    pub template_id: Option<Uuid>,
    pub node_pools: Vec<CreateNodePoolRequest>,
    pub network: Option<NetworkPolicy>,
    pub upgrade_policy: Option<UpgradePolicy>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClusterRequest {
    pub name: Option<String>,
    pub upgrade_policy: Option<UpgradePolicy>,
}

#[derive(Debug, Deserialize)]
pub struct UpgradeClusterRequest {
    pub target_version: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateNodePoolRequest {
    pub name: String,
    pub instance_type: String,
    pub min_nodes: u32,
    pub max_nodes: u32,
    pub autoscaling_enabled: bool,
    pub labels: HashMap<String, String>,
    pub taints: Vec<Taint>,
}

#[derive(Debug, Deserialize)]
pub struct ScaleNodePoolRequest {
    pub desired_nodes: u32,
    pub min_nodes: Option<u32>,
    pub max_nodes: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct InstallAddonRequest {
    pub addon_type: ClusterAddonType,
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub quota: TenantQuota,
    pub billing_info: BillingInfo,
}
