//! In-memory data sources backing the admin views.
//!
//! Real cave-runtime instances hand the views *connected* clients (an
//! `etcd::Client`, a `cri::Sandbox`, `kube::Api<DynamicObject>`, …). For
//! the parity scaffold we model the same shape with deterministic in-memory
//! seeded fixtures so the views can be unit-tested without a cluster.
//!
//! Multi-tenancy is enforced by filtering every collection on the request
//! context's tenant before returning anything. The fixtures intentionally
//! contain rows for *more than one* tenant so the cross-tenant tests can
//! observe that filter actually firing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use crate::admin::runtime_client::{RuntimeError, SharedRuntime};
use crate::admin::types::TenantId;

// ── etcd ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtcdKv {
    pub tenant: TenantId,
    pub key: String,
    pub value: String,
    pub revision: u64,
    pub lease_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtcdLease {
    pub tenant: TenantId,
    pub lease_id: u64,
    pub ttl_seconds: u64,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EtcdEvent {
    Put { key: String, value: String, revision: u64 },
    Delete { key: String, revision: u64 },
}

// ── cri ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriSandbox {
    pub tenant: TenantId,
    pub sandbox_id: String,
    pub pod_name: String,
    pub state: &'static str, // "Ready" | "NotReady"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriContainer {
    pub tenant: TenantId,
    pub sandbox_id: String,
    pub container_id: String,
    pub image: String,
    pub state: &'static str, // "Running" | "Exited" | "Created"
}

// ── apiserver ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct K8sResource {
    pub tenant: TenantId,
    pub kind: String,
    pub name: String,
    pub namespace: String,
}

// ── iam ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamUser {
    pub tenant: TenantId,
    pub username: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamRoleAssignment {
    pub tenant: TenantId,
    pub username: String,
    pub role: String,
}

// ── mesh ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshAuthzPolicy {
    pub tenant: TenantId,
    pub name: String,
    /// `Allow` or `Deny`.
    pub action: &'static str,
    pub principal_glob: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshFlow {
    pub tenant: TenantId,
    pub source: String,
    pub destination: String,
    pub verdict: &'static str, // "Forwarded" | "Dropped"
    pub bytes: u64,
}

// ── pg ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PgTable {
    pub tenant: TenantId,
    pub schema: String,
    pub name: String,
    pub row_count: u64,
}

// ── vault ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultSecretMeta {
    pub tenant: TenantId,
    pub path: String,
    pub version: u32,
    pub created_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultAuditEntry {
    pub tenant: TenantId,
    pub time_unix: i64,
    pub principal: String,
    pub op: &'static str, // "read-meta" | "read-value" | "write" | "delete"
    pub path: String,
}

/// A mounted secrets engine — mirrors `GET /v1/sys/mounts` in the
/// Vault HTTP API. Engines are the Vault concept that determines
/// HOW a path's secrets are stored (kv-v2, transit, pki, database, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultSecretsEngine {
    pub tenant: TenantId,
    /// Mount path (e.g. `kv/` or `pki-int/`). Includes trailing slash
    /// per Vault convention.
    pub path: String,
    /// Engine type identifier (`kv`, `pki`, `transit`, `database`, …).
    pub engine_type: String,
    /// Optional engine version (KV mounts carry `version=2` etc.).
    pub version: u32,
    /// Default TTL applied to leases issued by this mount, in seconds.
    /// `0` means inherit from the Vault default.
    pub default_lease_ttl_s: u64,
    /// Sealed-status indicator: `true` means the mount is currently
    /// usable, `false` means it was disabled or seal-wrapped.
    pub enabled: bool,
}

/// A mounted auth method — mirrors `GET /v1/sys/auth`. Auth methods
/// determine HOW a caller authenticates (token, userpass, kubernetes,
/// approle, oidc, …). The `vault_auth_methods` collection lists
/// every mount in the tenant; the `accessor` field is the unique
/// stable identifier Vault uses in audit logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultAuthMethod {
    pub tenant: TenantId,
    /// Mount path (e.g. `userpass/`, `kubernetes/`).
    pub path: String,
    /// Method type identifier (`token`, `userpass`, `kubernetes`,
    /// `approle`, `oidc`, …).
    pub method_type: String,
    /// Vault-issued stable identifier (used in audit logs to identify
    /// the mount even across rename).
    pub accessor: String,
    /// Default TTL for tokens issued by this method, in seconds.
    pub default_lease_ttl_s: u64,
    /// `true` if the mount is currently active.
    pub enabled: bool,
}

/// A named policy — mirrors `GET /v1/sys/policies/acl/<name>`. Vault
/// policies are HCL documents; the `rules` field carries the rendered
/// text so the UI can show the same content `vault policy read` would.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultPolicy {
    pub tenant: TenantId,
    pub name: String,
    /// HCL policy body. May be multi-line; the UI escapes it before
    /// rendering.
    pub rules: String,
    /// Number of tokens currently bound to this policy. Surfaced so
    /// an operator can see at a glance whether deleting the policy
    /// would orphan access.
    pub bound_token_count: u32,
}

// ── keda ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaledObject {
    pub tenant: TenantId,
    pub name: String,
    /// Target Deployment / StatefulSet / Custom resource.
    pub target_ref: String,
    pub min_replicas: u32,
    pub max_replicas: u32,
    pub current_replicas: u32,
    pub paused: bool,
    /// Trigger types attached: `cpu`, `memory`, `kafka`, `prometheus`, `redis`, `cron`, `http`.
    pub triggers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScalerEvent {
    pub tenant: TenantId,
    pub when_unix: i64,
    pub scaled_object: String,
    /// e.g. `kafka:lag=120`, `cpu:75`, `prometheus:queue_depth=900`.
    pub trigger: String,
    pub from_replicas: u32,
    pub to_replicas: u32,
    /// `Scaled` | `NoChange` | `FallbackActive`.
    pub verdict: &'static str,
}

// ── scheduler ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerNode {
    pub tenant: TenantId,
    pub name: String,
    pub ready: bool,
    pub allocatable_cpu_milli: u64,
    pub allocatable_mem_mib: u64,
    pub taints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerPolicy {
    pub tenant: TenantId,
    pub name: String,
    pub predicate: String,
    pub weight: u32,
}

// ── controller-manager ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerLease {
    pub tenant: TenantId,
    pub controller: String,
    pub leader_id: String,
    pub renewals: u64,
    pub expires_unix: i64,
}

// ── kubelet ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubeletPod {
    pub tenant: TenantId,
    pub node: String,
    pub pod_name: String,
    pub status: &'static str, // "Running" | "Pending" | "Failed"
    pub restart_count: u32,
}

// ── cloud-controller-manager ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudVolume {
    pub tenant: TenantId,
    pub id: String,
    pub region: String,
    pub size_gb: u64,
    pub attached_node: Option<String>,
}

// ── kamaji ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KamajiTcp {
    pub tenant: TenantId,
    pub name: String,
    pub k8s_version: String,
    pub ready_replicas: u32,
    pub desired_replicas: u32,
}

// ── net (Cilium) ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetEndpoint {
    pub tenant: TenantId,
    pub identity: u64,
    pub namespace: String,
    pub ip: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetPolicy {
    pub tenant: TenantId,
    pub name: String,
    pub direction: &'static str, // "Ingress" | "Egress" | "Both"
    pub selector: String,
}

// ── rdbms (postgres operator) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdbmsCluster {
    pub tenant: TenantId,
    pub name: String,
    pub version: String,
    pub replicas: u32,
    pub primary_node: String,
}

// ── docdb (mongo / yugabyte / etc.) ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocdbCollection {
    pub tenant: TenantId,
    pub database: String,
    pub collection: String,
    pub document_count: u64,
}

// ── cache (dragonfly / valkey) ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub tenant: TenantId,
    pub namespace: String,
    pub key: String,
    pub ttl_seconds: u64,
    pub size_bytes: u64,
}

// ── rdbms-operator (Postgres / CNPG) ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdbmsOperatorCluster {
    pub tenant: TenantId,
    pub name: String,
    pub upstream_kind: &'static str, // "CNPG" | "PgBouncer"
    pub version: String,
    pub instances: u32,
    pub primary_pod: String,
    pub replication_lag_bytes: u64,
    pub replication_state: &'static str, // "InSync" | "Catchup" | "Stale" | "Disconnected"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdbmsOperatorBackup {
    pub tenant: TenantId,
    pub cluster: String,
    pub backup_id: String,
    pub started_unix: i64,
    pub finished_unix: Option<i64>,
    pub size_mib: u64,
    pub state: &'static str, // "Completed" | "Running" | "Failed"
}

// ── lakehouse (Iceberg + DataFusion) ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LakehouseTable {
    pub tenant: TenantId,
    pub namespace: String,
    pub name: String,
    pub format_version: u32,    // Iceberg v1/v2/v3
    pub partition_count: u64,
    pub file_count: u64,
    pub size_bytes: u64,
    pub current_snapshot_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LakehouseSnapshot {
    pub tenant: TenantId,
    pub namespace: String,
    pub table: String,
    pub snapshot_id: u64,
    pub committed_unix: i64,
    pub op: &'static str, // "Append" | "Overwrite" | "Delete" | "Replace"
    pub added_files: u64,
}

// ── streams (Kafka + Pulsar) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamsTopic {
    pub tenant: TenantId,
    pub name: String,
    pub partitions: u32,
    pub replication_factor: u32,
    pub retention_ms: u64,
    pub compaction: &'static str, // "Delete" | "Compact" | "DeleteAndCompact"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamsConsumerGroup {
    pub tenant: TenantId,
    pub group_id: String,
    pub topic: String,
    pub members: u32,
    pub current_offset: u64,
    pub log_end_offset: u64,
    pub state: &'static str, // "Stable" | "Rebalancing" | "Empty" | "Dead"
}

impl StreamsConsumerGroup {
    pub fn lag(&self) -> u64 {
        self.log_end_offset.saturating_sub(self.current_offset)
    }
}

// ── Kafka Connect ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamsConnector {
    pub tenant: TenantId,
    pub name: String,
    pub kind: &'static str,    // "Source" | "Sink"
    pub state: &'static str,   // "Running" | "Paused" | "Failed" | "Stopped"
    pub tasks_max: u32,
    pub class: String,
    pub topics: String, // CSV
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamsConnectWorker {
    pub tenant: TenantId,
    pub id: String,
    pub state: &'static str, // "Running" | "Dead"
    pub host: String,
    pub connectors_owned: u32,
    pub tasks_owned: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamsConnectTask {
    pub tenant: TenantId,
    pub connector: String,
    pub task: u32,
    pub state: &'static str, // "Running" | "Failed" | "Paused" | "Stopped"
    pub failure_trace: Option<String>,
}

// ── policy ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub tenant: TenantId,
    pub name: String,
    pub action: &'static str, // "Allow" | "Deny" | "Audit"
    pub subject: String,      // SPIFFE / role glob
    pub resource: String,     // resource glob
    pub enabled: bool,
}

// ── artifacts ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub tenant: TenantId,
    pub registry: String,
    pub name: String,
    pub digest: String,    // sha256:...
    pub size_bytes: u64,
    pub pushed_unix: i64,
}

// ── alerts ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRule {
    pub tenant: TenantId,
    pub name: String,
    pub severity: &'static str, // "critical" | "warning" | "info"
    pub expr: String,
    pub for_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveAlert {
    pub tenant: TenantId,
    pub rule: String,
    pub state: &'static str, // "firing" | "pending"
    pub fired_unix: i64,
}

// ── backup ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupJob {
    pub tenant: TenantId,
    pub name: String,
    pub source: String,
    pub destination: String,
    pub schedule_cron: String,
    pub last_run_unix: Option<i64>,
    pub state: &'static str, // "Scheduled" | "Running" | "Completed" | "Failed"
}

// ── incidents ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentRecord {
    pub tenant: TenantId,
    pub id: String,
    pub title: String,
    pub severity: &'static str, // "SEV1" | "SEV2" | "SEV3" | "SEV4"
    pub state: &'static str,    // "Open" | "Investigating" | "Resolved"
    pub opened_unix: i64,
}

// ── vulns ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulnRecord {
    pub tenant: TenantId,
    pub cve_id: String,
    pub package: String,
    pub installed_version: String,
    pub fixed_version: Option<String>,
    pub severity: &'static str, // "Critical" | "High" | "Medium" | "Low"
}

// ── workflows ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub tenant: TenantId,
    pub name: String,
    pub run_id: String,
    pub status: &'static str, // "Pending" | "Running" | "Succeeded" | "Failed"
    pub started_unix: i64,
    pub finished_unix: Option<i64>,
}

// ── chaos ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChaosExperiment {
    pub tenant: TenantId,
    pub name: String,
    pub kind: String,       // "pod-kill", "network-delay", "cpu-stress", ...
    pub target_selector: String,
    pub schedule: &'static str, // "Once" | "Cron" | "Continuous"
    pub last_run_unix: Option<i64>,
}

// ── slo ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slo {
    pub tenant: TenantId,
    pub name: String,
    pub service: String,
    pub objective_pct: f32, // 99.9 etc.
    pub window_days: u32,
    pub current_pct: f32,
    pub error_budget_remaining_pct: f32,
}

// ── cave-cdc ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdcPipeline {
    pub tenant: TenantId,
    pub name: String,
    pub source: String,
    pub sink: String,
    pub state: &'static str,
}

// ── cave-certs ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertRecord {
    pub tenant: TenantId,
    pub subject: String,
    pub issuer: String,
    pub not_after_unix: i64,
    pub serial: String,
}

// ── cave-crm ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrmAccount {
    pub tenant: TenantId,
    pub id: String,
    pub name: String,
    pub plan: &'static str,
    pub mrr_cents: u64,
}

// ── cave-crossplane ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossplaneClaim {
    pub tenant: TenantId,
    pub name: String,
    pub kind: String,
    pub composition: String,
    pub state: &'static str,
}

// ── cave-gitops-config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitopsApp {
    pub tenant: TenantId,
    pub name: String,
    pub repo: String,
    pub path: String,
    pub synced_at_unix: i64,
}

// ── cave-karpenter ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePool {
    pub tenant: TenantId,
    pub name: String,
    pub instance_class: String,
    pub max_nodes: u32,
    pub active_nodes: u32,
}

// ── cave-kubevirt ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualMachine {
    pub tenant: TenantId,
    pub name: String,
    pub phase: &'static str,
    pub cpu: u32,
    pub memory_mib: u64,
}

// ── cave-ledger ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub tenant: TenantId,
    pub id: String,
    pub actor: String,
    pub action: String,
    pub at_unix: i64,
}

// ── cave-oncall ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OncallShift {
    pub tenant: TenantId,
    pub rotation: String,
    pub oncaller: String,
    pub start_unix: i64,
    pub end_unix: i64,
}

// ── cave-search ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndex {
    pub tenant: TenantId,
    pub name: String,
    pub doc_count: u64,
    pub size_bytes: u64,
    pub status: &'static str,
}

// ── cave-deploy ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployActivity {
    pub tenant: TenantId,
    pub id: String,
    pub service: String,
    pub version: String,
    pub status: &'static str,
}

// ── cave-pipelines ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineRun {
    pub tenant: TenantId,
    pub pipeline: String,
    pub run_id: String,
    pub status: &'static str,
    pub duration_seconds: u32,
}

// ── cave-rollouts ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutStatus {
    pub tenant: TenantId,
    pub name: String,
    pub strategy: &'static str,
    pub traffic_pct: u32,
    pub state: &'static str,
}

// ── cave-knative ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnativeService {
    pub tenant: TenantId,
    pub name: String,
    pub image: String,
    pub replicas: u32,
    pub min_scale: u32,
    pub max_scale: u32,
}

// ── cave-llm-gateway ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmRoute {
    pub tenant: TenantId,
    pub name: String,
    pub upstream: String,
    pub rpm_limit: u32,
    pub daily_tokens: u64,
}

// ── cave-local-llm ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalLlmModel {
    pub tenant: TenantId,
    pub tag: String,
    pub size_bytes: u64,
    pub quant: String,
    pub loaded: bool,
}

// ── cave-tracker ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssue {
    pub tenant: TenantId,
    pub id: String,
    pub title: String,
    pub state: &'static str,
    pub assignee: Option<String>,
}

// ── cave-upstream ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamProject {
    pub tenant: TenantId,
    pub name: String,
    pub repo: String,
    pub pinned_version: String,
    pub last_check_unix: i64,
}

// ── cave-container-scan ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerScanResult {
    pub tenant: TenantId,
    pub image: String,
    pub digest: String,
    pub critical_cves: u32,
    pub scanned_at_unix: i64,
}

// ── cave-admission ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    pub tenant: TenantId,
    pub id: String,
    pub resource_kind: String,
    pub decision: &'static str,
    pub reason: String,
}

// ── cave-store ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreBucket {
    pub tenant: TenantId,
    pub name: String,
    pub backend: String,
    pub object_count: u64,
    pub size_bytes: u64,
}

// ── cave-metrics ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricSeries {
    pub tenant: TenantId,
    pub name: String,
    pub scraper: String,
    pub sample_count: u64,
    pub retention_days: u32,
}

// ── cave-trace ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceService {
    pub tenant: TenantId,
    pub service: String,
    pub span_count_per_sec: u32,
    pub error_rate_per_thousand: u32,
}

// ── cave-auth ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub tenant: TenantId,
    pub session_id: String,
    pub principal: String,
    pub realm: String,
    pub expires_unix: i64,
}

// ── cave-dashboard ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardCatalog {
    pub tenant: TenantId,
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub panels: u32,
}

// ── cave-dns ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsZone {
    pub tenant: TenantId,
    pub zone: String,
    pub record_count: u32,
    pub serial: u64,
}

// ── cave-logs ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogStream {
    pub tenant: TenantId,
    pub name: String,
    pub sink: String,
    pub ingest_rate_per_sec: u32,
    pub retention_days: u32,
}

// ── cave-security ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityEvent {
    pub tenant: TenantId,
    pub id: String,
    pub kind: String,
    pub severity: &'static str,
    pub at_unix: i64,
}

// ── cave-ha ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaFailoverEvent {
    pub tenant: TenantId,
    pub id: String,
    pub subject: String,
    pub old_primary: String,
    pub new_primary: String,
    pub at_unix: i64,
}

// ── cave-erp ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErpInvoice {
    pub tenant: TenantId,
    pub invoice_id: String,
    pub customer: String,
    pub amount_cents: u64,
    pub status: &'static str,
}

// ── cave-ai-obs ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModelMetric {
    pub tenant: TenantId,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub latency_p99_ms: u32,
}

// ── cave-chat ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatThread {
    pub tenant: TenantId,
    pub id: String,
    pub topic: String,
    pub members: u32,
    pub last_message_unix: i64,
}

// ── cave-cost ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostReport {
    pub tenant: TenantId,
    pub period: String,
    pub service: String,
    pub amount_cents: u64,
}

// ── cave-dast ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DastScan {
    pub tenant: TenantId,
    pub target: String,
    pub scan_id: String,
    pub findings: u32,
    pub severity: &'static str,
}

// ── cave-devlake ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevlakeMetric {
    pub tenant: TenantId,
    pub project: String,
    pub metric: String,
    pub value_thousandths: u64,
}

// ── cave-forensics ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForensicsEvidence {
    pub tenant: TenantId,
    pub case_id: String,
    pub artifact: String,
    pub collected_unix: i64,
    pub digest: String,
}

// ── cave-gateway ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayRoute {
    pub tenant: TenantId,
    pub name: String,
    pub listener: String,
    pub hostname: String,
    pub backend: String,
}

// ── cave-infra ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfraStack {
    pub tenant: TenantId,
    pub name: String,
    pub provider: String,
    pub region: String,
    pub resources: u32,
    pub state: &'static str,
}

// ── cave-pam ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PamSession {
    pub tenant: TenantId,
    pub id: String,
    pub principal: String,
    pub target: String,
    pub started_unix: i64,
    pub ended_unix: Option<i64>,
}

// ── cave-sbom ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SbomComponent {
    pub tenant: TenantId,
    pub image: String,
    pub package: String,
    pub version: String,
    pub license: String,
}

// ── cave-scan ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanResult {
    pub tenant: TenantId,
    pub scan_id: String,
    pub scanner: String,
    pub findings: u32,
    pub worst_severity: &'static str,
}

// ── cave-secrets ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub tenant: TenantId,
    pub path: String,
    pub backend: String,
    pub version: u32,
    pub created_unix: i64,
}

// ── cave-uptime ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UptimeProbe {
    pub tenant: TenantId,
    pub name: String,
    pub url: String,
    pub interval_seconds: u32,
    pub last_status: &'static str,
}

// ── cave-cluster ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubeCluster {
    pub tenant: TenantId,
    pub name: String,
    pub k8s_version: String,
    pub nodes: u32,
    pub state: &'static str,
}

// ── cave-kube-proxy ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubeProxyService {
    pub tenant: TenantId,
    pub name: String,
    pub namespace: String,
    pub cluster_ip: String,
    pub backend_count: u32,
}

// ── tenant dashboard recent activity ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub tenant: TenantId,
    pub when_unix: i64,
    pub kind: &'static str,
    pub summary: String,
}

// ── aggregate state ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct AdminState {
    /// Real-runtime data source set at startup (one-shot via
    /// `set_runtime_client`). When `None`, admin views read directly
    /// from the seeded `RwLock<Vec<T>>` fixtures below — the legacy
    /// development workflow is unchanged. When set, the `materialise_*`
    /// methods refresh the corresponding collection from the live
    /// cave-apiserver before each render.
    pub runtime_client: OnceLock<SharedRuntime>,
    pub etcd_kv: RwLock<Vec<EtcdKv>>,
    pub etcd_leases: RwLock<Vec<EtcdLease>>,
    pub etcd_event_log: RwLock<Vec<EtcdEvent>>,
    pub cri_sandboxes: RwLock<Vec<CriSandbox>>,
    pub cri_containers: RwLock<Vec<CriContainer>>,
    pub k8s_resources: RwLock<Vec<K8sResource>>,
    pub iam_users: RwLock<Vec<IamUser>>,
    pub iam_assignments: RwLock<Vec<IamRoleAssignment>>,
    pub mesh_authz: RwLock<Vec<MeshAuthzPolicy>>,
    pub mesh_flows: RwLock<Vec<MeshFlow>>,
    pub pg_tables: RwLock<Vec<PgTable>>,
    pub vault_secrets: RwLock<Vec<VaultSecretMeta>>,
    pub vault_audit: RwLock<Vec<VaultAuditEntry>>,
    // 2026-05-12 batch: rich Vault UI surface (engines + auth + policies).
    pub vault_engines: RwLock<Vec<VaultSecretsEngine>>,
    pub vault_auth_methods: RwLock<Vec<VaultAuthMethod>>,
    pub vault_policies: RwLock<Vec<VaultPolicy>>,
    pub keda_scaled_objects: RwLock<Vec<KedaScaledObject>>,
    pub keda_scaler_events: RwLock<Vec<KedaScalerEvent>>,
    // 2026-05-12 batch: rich CRD-shaped KEDA detail surface.
    pub keda_scaled_object_details:
        RwLock<Vec<crate::admin::keda::types::KedaScaledObjectDetail>>,
    pub keda_scaled_jobs: RwLock<Vec<crate::admin::keda::types::KedaScaledJob>>,
    pub keda_trigger_authentications:
        RwLock<Vec<crate::admin::keda::types::KedaTriggerAuthentication>>,
    pub recent_activity: RwLock<Vec<ActivityEntry>>,
    // 2026-05-10 batch.
    pub scheduler_nodes: RwLock<Vec<SchedulerNode>>,
    pub scheduler_policies: RwLock<Vec<SchedulerPolicy>>,
    pub controller_leases: RwLock<Vec<ControllerLease>>,
    pub kubelet_pods: RwLock<Vec<KubeletPod>>,
    pub cloud_volumes: RwLock<Vec<CloudVolume>>,
    pub kamaji_tcps: RwLock<Vec<KamajiTcp>>,
    pub net_endpoints: RwLock<Vec<NetEndpoint>>,
    pub net_policies: RwLock<Vec<NetPolicy>>,
    pub rdbms_clusters: RwLock<Vec<RdbmsCluster>>,
    pub docdb_collections: RwLock<Vec<DocdbCollection>>,
    pub cache_entries: RwLock<Vec<CacheEntry>>,
    pub rdbms_operator_clusters: RwLock<Vec<RdbmsOperatorCluster>>,
    pub rdbms_operator_backups: RwLock<Vec<RdbmsOperatorBackup>>,
    pub lakehouse_tables: RwLock<Vec<LakehouseTable>>,
    pub lakehouse_snapshots: RwLock<Vec<LakehouseSnapshot>>,
    pub streams_topics: RwLock<Vec<StreamsTopic>>,
    pub streams_consumer_groups: RwLock<Vec<StreamsConsumerGroup>>,
    pub streams_connectors: RwLock<Vec<StreamsConnector>>,
    pub streams_connect_workers: RwLock<Vec<StreamsConnectWorker>>,
    pub streams_connect_tasks: RwLock<Vec<StreamsConnectTask>>,
    pub policy_rules: RwLock<Vec<PolicyRule>>,
    pub artifact_records: RwLock<Vec<ArtifactRecord>>,
    pub alert_rules: RwLock<Vec<AlertRule>>,
    pub active_alerts: RwLock<Vec<ActiveAlert>>,
    pub backup_jobs: RwLock<Vec<BackupJob>>,
    pub incident_records: RwLock<Vec<IncidentRecord>>,
    pub vuln_records: RwLock<Vec<VulnRecord>>,
    pub workflow_runs: RwLock<Vec<WorkflowRun>>,
    pub chaos_experiments: RwLock<Vec<ChaosExperiment>>,
    pub slos: RwLock<Vec<Slo>>,
    pub ai_model_metrics: RwLock<Vec<AiModelMetric>>,
    pub chat_threads: RwLock<Vec<ChatThread>>,
    pub cost_reports: RwLock<Vec<CostReport>>,
    pub dast_scans: RwLock<Vec<DastScan>>,
    pub devlake_metrics: RwLock<Vec<DevlakeMetric>>,
    pub forensics_evidence: RwLock<Vec<ForensicsEvidence>>,
    pub gateway_routes: RwLock<Vec<GatewayRoute>>,
    pub infra_stacks: RwLock<Vec<InfraStack>>,
    pub pam_sessions: RwLock<Vec<PamSession>>,
    pub sbom_components: RwLock<Vec<SbomComponent>>,
    pub scan_results: RwLock<Vec<ScanResult>>,
    pub secret_metadatas: RwLock<Vec<SecretMetadata>>,
    pub uptime_probes: RwLock<Vec<UptimeProbe>>,
    pub kube_clusters: RwLock<Vec<KubeCluster>>,
    pub kube_proxy_services: RwLock<Vec<KubeProxyService>>,
    pub store_buckets: RwLock<Vec<StoreBucket>>,
    pub metric_series: RwLock<Vec<MetricSeries>>,
    pub trace_services: RwLock<Vec<TraceService>>,
    pub auth_sessions: RwLock<Vec<AuthSession>>,
    pub dashboard_catalog: RwLock<Vec<DashboardCatalog>>,
    pub dns_zones: RwLock<Vec<DnsZone>>,
    pub log_streams: RwLock<Vec<LogStream>>,
    pub security_events: RwLock<Vec<SecurityEvent>>,
    pub ha_failover_events: RwLock<Vec<HaFailoverEvent>>,
    pub erp_invoices: RwLock<Vec<ErpInvoice>>,
    pub deploy_activities: RwLock<Vec<DeployActivity>>,
    pub pipeline_runs: RwLock<Vec<PipelineRun>>,
    pub rollout_statuses: RwLock<Vec<RolloutStatus>>,
    pub knative_services: RwLock<Vec<KnativeService>>,
    pub llm_routes: RwLock<Vec<LlmRoute>>,
    pub local_llm_models: RwLock<Vec<LocalLlmModel>>,
    pub tracker_issues: RwLock<Vec<TrackerIssue>>,
    pub upstream_projects: RwLock<Vec<UpstreamProject>>,
    pub container_scan_results: RwLock<Vec<ContainerScanResult>>,
    pub admission_decisions: RwLock<Vec<AdmissionDecision>>,
    pub cdc_pipelines: RwLock<Vec<CdcPipeline>>,
    pub cert_records: RwLock<Vec<CertRecord>>,
    pub crm_accounts: RwLock<Vec<CrmAccount>>,
    pub crossplane_claims: RwLock<Vec<CrossplaneClaim>>,
    pub gitops_apps: RwLock<Vec<GitopsApp>>,
    pub node_pools: RwLock<Vec<NodePool>>,
    pub virtual_machines: RwLock<Vec<VirtualMachine>>,
    pub ledger_entries: RwLock<Vec<LedgerEntry>>,
    pub oncall_shifts: RwLock<Vec<OncallShift>>,
    pub search_indexes: RwLock<Vec<SearchIndex>>,

    // ── 2026-05-13 realtime + power-user batch ────────────────────
    pub event_bus: std::sync::Arc<crate::admin::events::EventBus>,
    pub audit_store: std::sync::Arc<crate::admin::audit::AuditStore>,
    pub cluster_live: std::sync::Arc<crate::admin::cluster_live::ClusterLiveState>,
    pub onboarding: std::sync::Arc<crate::admin::onboarding::OnboardingState>,
    pub global_search: std::sync::Arc<crate::admin::global_search::GlobalSearchIndex>,

    // ── 2026-05-13 P1 scratch pages ───────────────────────────────
    pub iceberg_tables: RwLock<Vec<crate::admin::iceberg::IcebergTable>>,
    pub mlflow_experiments: RwLock<Vec<crate::admin::mlflow::MlflowExperiment>>,
    pub mlflow_runs: RwLock<Vec<crate::admin::mlflow::MlflowRun>>,
    pub mlflow_models: RwLock<Vec<crate::admin::mlflow::RegisteredModel>>,
    pub mlflow_model_versions: RwLock<Vec<crate::admin::mlflow::ModelVersion>>,
    pub mlflow_deployments: RwLock<Vec<crate::admin::mlflow::ModelDeployment>>,
    pub litellm_models: RwLock<Vec<crate::admin::litellm::LiteLlmModel>>,
    pub litellm_routes: RwLock<Vec<crate::admin::litellm::LiteLlmRoute>>,
    pub litellm_api_keys: RwLock<Vec<crate::admin::litellm::LiteLlmApiKey>>,
    pub litellm_budgets: RwLock<Vec<crate::admin::litellm::LiteLlmBudget>>,
    pub litellm_traffic: RwLock<Vec<crate::admin::litellm::LiteLlmTraffic>>,
}

impl Default for AdminState {
    fn default() -> Self {
        Self::seeded()
    }
}

impl AdminState {
    /// Install a real-runtime data source for this state. Idempotent —
    /// the first call wins; subsequent calls silently no-op so concurrent
    /// startup hooks can't double-install.
    pub fn set_runtime_client(&self, client: SharedRuntime) {
        let _ = self.runtime_client.set(client);
    }

    /// `Some(client)` when a real apiserver client has been installed.
    pub fn runtime(&self) -> Option<&SharedRuntime> {
        self.runtime_client.get()
    }

    /// If a real runtime is installed, fetch the live kubelet pods for
    /// the given tenant and overwrite `self.kubelet_pods` for that
    /// tenant in place. Returns `Ok(())` whether the materialiser fired
    /// or not (no runtime = no-op).
    pub async fn materialise_kubelet_pods(&self, tenant: &TenantId) -> Result<(), RuntimeError> {
        let Some(rt) = self.runtime() else {
            return Ok(());
        };
        let fresh = rt.list_kubelet_pods(tenant).await?;
        replace_tenant_rows(&self.kubelet_pods, tenant, fresh, |r| &r.tenant);
        Ok(())
    }

    pub async fn materialise_scheduler_nodes(
        &self,
        tenant: &TenantId,
    ) -> Result<(), RuntimeError> {
        let Some(rt) = self.runtime() else {
            return Ok(());
        };
        let fresh = rt.list_scheduler_nodes(tenant).await?;
        replace_tenant_rows(&self.scheduler_nodes, tenant, fresh, |r| &r.tenant);
        Ok(())
    }

    pub async fn materialise_net_endpoints(
        &self,
        tenant: &TenantId,
    ) -> Result<(), RuntimeError> {
        let Some(rt) = self.runtime() else {
            return Ok(());
        };
        let fresh = rt.list_net_endpoints(tenant).await?;
        replace_tenant_rows(&self.net_endpoints, tenant, fresh, |r| &r.tenant);
        Ok(())
    }

    pub async fn materialise_keda_scaled_objects(
        &self,
        tenant: &TenantId,
    ) -> Result<(), RuntimeError> {
        let Some(rt) = self.runtime() else {
            return Ok(());
        };
        let fresh = rt.list_keda_scaled_objects(tenant).await?;
        replace_tenant_rows(&self.keda_scaled_objects, tenant, fresh, |r| &r.tenant);
        Ok(())
    }

    /// Empty state — used by integration tests that want to push their
    /// own fixtures.
    pub fn empty() -> Self {
        Self {
            runtime_client: OnceLock::new(),
            etcd_kv: RwLock::new(Vec::new()),
            etcd_leases: RwLock::new(Vec::new()),
            etcd_event_log: RwLock::new(Vec::new()),
            cri_sandboxes: RwLock::new(Vec::new()),
            cri_containers: RwLock::new(Vec::new()),
            k8s_resources: RwLock::new(Vec::new()),
            iam_users: RwLock::new(Vec::new()),
            iam_assignments: RwLock::new(Vec::new()),
            mesh_authz: RwLock::new(Vec::new()),
            mesh_flows: RwLock::new(Vec::new()),
            pg_tables: RwLock::new(Vec::new()),
            vault_secrets: RwLock::new(Vec::new()),
            vault_audit: RwLock::new(Vec::new()),
            vault_engines: RwLock::new(Vec::new()),
            vault_auth_methods: RwLock::new(Vec::new()),
            vault_policies: RwLock::new(Vec::new()),
            keda_scaled_objects: RwLock::new(Vec::new()),
            keda_scaler_events: RwLock::new(Vec::new()),
            keda_scaled_object_details: RwLock::new(Vec::new()),
            keda_scaled_jobs: RwLock::new(Vec::new()),
            keda_trigger_authentications: RwLock::new(Vec::new()),
            recent_activity: RwLock::new(Vec::new()),
            scheduler_nodes: RwLock::new(Vec::new()),
            scheduler_policies: RwLock::new(Vec::new()),
            controller_leases: RwLock::new(Vec::new()),
            kubelet_pods: RwLock::new(Vec::new()),
            cloud_volumes: RwLock::new(Vec::new()),
            kamaji_tcps: RwLock::new(Vec::new()),
            net_endpoints: RwLock::new(Vec::new()),
            net_policies: RwLock::new(Vec::new()),
            rdbms_clusters: RwLock::new(Vec::new()),
            docdb_collections: RwLock::new(Vec::new()),
            cache_entries: RwLock::new(Vec::new()),
            rdbms_operator_clusters: RwLock::new(Vec::new()),
            rdbms_operator_backups: RwLock::new(Vec::new()),
            lakehouse_tables: RwLock::new(Vec::new()),
            lakehouse_snapshots: RwLock::new(Vec::new()),
            streams_topics: RwLock::new(Vec::new()),
            streams_consumer_groups: RwLock::new(Vec::new()),
            streams_connectors: RwLock::new(Vec::new()),
            streams_connect_workers: RwLock::new(Vec::new()),
            streams_connect_tasks: RwLock::new(Vec::new()),
            policy_rules: RwLock::new(Vec::new()),
            artifact_records: RwLock::new(Vec::new()),
            alert_rules: RwLock::new(Vec::new()),
            active_alerts: RwLock::new(Vec::new()),
            backup_jobs: RwLock::new(Vec::new()),
            incident_records: RwLock::new(Vec::new()),
            vuln_records: RwLock::new(Vec::new()),
            workflow_runs: RwLock::new(Vec::new()),
            chaos_experiments: RwLock::new(Vec::new()),
            slos: RwLock::new(Vec::new()),
            ai_model_metrics: RwLock::new(Vec::new()),
            chat_threads: RwLock::new(Vec::new()),
            cost_reports: RwLock::new(Vec::new()),
            dast_scans: RwLock::new(Vec::new()),
            devlake_metrics: RwLock::new(Vec::new()),
            forensics_evidence: RwLock::new(Vec::new()),
            gateway_routes: RwLock::new(Vec::new()),
            infra_stacks: RwLock::new(Vec::new()),
            pam_sessions: RwLock::new(Vec::new()),
            sbom_components: RwLock::new(Vec::new()),
            scan_results: RwLock::new(Vec::new()),
            secret_metadatas: RwLock::new(Vec::new()),
            uptime_probes: RwLock::new(Vec::new()),
            kube_clusters: RwLock::new(Vec::new()),
            kube_proxy_services: RwLock::new(Vec::new()),
            store_buckets: RwLock::new(Vec::new()),
            metric_series: RwLock::new(Vec::new()),
            trace_services: RwLock::new(Vec::new()),
            auth_sessions: RwLock::new(Vec::new()),
            dashboard_catalog: RwLock::new(Vec::new()),
            dns_zones: RwLock::new(Vec::new()),
            log_streams: RwLock::new(Vec::new()),
            security_events: RwLock::new(Vec::new()),
            ha_failover_events: RwLock::new(Vec::new()),
            erp_invoices: RwLock::new(Vec::new()),
            deploy_activities: RwLock::new(Vec::new()),
            pipeline_runs: RwLock::new(Vec::new()),
            rollout_statuses: RwLock::new(Vec::new()),
            knative_services: RwLock::new(Vec::new()),
            llm_routes: RwLock::new(Vec::new()),
            local_llm_models: RwLock::new(Vec::new()),
            tracker_issues: RwLock::new(Vec::new()),
            upstream_projects: RwLock::new(Vec::new()),
            container_scan_results: RwLock::new(Vec::new()),
            admission_decisions: RwLock::new(Vec::new()),
            cdc_pipelines: RwLock::new(Vec::new()),
            cert_records: RwLock::new(Vec::new()),
            crm_accounts: RwLock::new(Vec::new()),
            crossplane_claims: RwLock::new(Vec::new()),
            gitops_apps: RwLock::new(Vec::new()),
            node_pools: RwLock::new(Vec::new()),
            virtual_machines: RwLock::new(Vec::new()),
            ledger_entries: RwLock::new(Vec::new()),
            oncall_shifts: RwLock::new(Vec::new()),
            search_indexes: RwLock::new(Vec::new()),
            event_bus: std::sync::Arc::new(crate::admin::events::EventBus::new()),
            audit_store: std::sync::Arc::new(crate::admin::audit::AuditStore::default()),
            cluster_live: std::sync::Arc::new(
                crate::admin::cluster_live::ClusterLiveState::new(),
            ),
            onboarding: std::sync::Arc::new(crate::admin::onboarding::OnboardingState::new()),
            global_search: std::sync::Arc::new(
                crate::admin::global_search::GlobalSearchIndex::new(),
            ),
            iceberg_tables: RwLock::new(Vec::new()),
            mlflow_experiments: RwLock::new(Vec::new()),
            mlflow_runs: RwLock::new(Vec::new()),
            mlflow_models: RwLock::new(Vec::new()),
            mlflow_model_versions: RwLock::new(Vec::new()),
            mlflow_deployments: RwLock::new(Vec::new()),
            litellm_models: RwLock::new(Vec::new()),
            litellm_routes: RwLock::new(Vec::new()),
            litellm_api_keys: RwLock::new(Vec::new()),
            litellm_budgets: RwLock::new(Vec::new()),
            litellm_traffic: RwLock::new(Vec::new()),
        }
    }

    /// Seeded state — every collection has rows for `acme` *and* a foreign
    /// `evil` tenant so cross-tenant tests can verify the filter.
    pub fn seeded() -> Self {
        let s = Self::empty();
        let acme = TenantId::new("acme").expect("test fixture");
        let evil = TenantId::new("evil").expect("test fixture");
        s.etcd_kv.write().unwrap().extend([
            EtcdKv { tenant: acme.clone(), key: "/cfg/feature_x".into(), value: "on".into(), revision: 7, lease_id: None },
            EtcdKv { tenant: acme.clone(), key: "/state/leader".into(), value: "node-a".into(), revision: 8, lease_id: Some(1001) },
            EtcdKv { tenant: evil.clone(), key: "/cfg/feature_y".into(), value: "secret".into(), revision: 1, lease_id: None },
        ]);
        s.etcd_leases.write().unwrap().extend([
            EtcdLease { tenant: acme.clone(), lease_id: 1001, ttl_seconds: 30, keys: vec!["/state/leader".into()] },
            EtcdLease { tenant: evil.clone(), lease_id: 9999, ttl_seconds: 60, keys: vec!["/secret/x".into()] },
        ]);
        s.etcd_event_log.write().unwrap().extend([
            EtcdEvent::Put { key: "/cfg/feature_x".into(), value: "on".into(), revision: 7 },
            EtcdEvent::Put { key: "/state/leader".into(), value: "node-a".into(), revision: 8 },
        ]);
        s.cri_sandboxes.write().unwrap().extend([
            CriSandbox { tenant: acme.clone(), sandbox_id: "sb-1".into(), pod_name: "web-0".into(), state: "Ready" },
            CriSandbox { tenant: acme.clone(), sandbox_id: "sb-2".into(), pod_name: "api-0".into(), state: "Ready" },
            CriSandbox { tenant: evil.clone(), sandbox_id: "sb-evil".into(), pod_name: "x-0".into(), state: "Ready" },
        ]);
        s.cri_containers.write().unwrap().extend([
            CriContainer { tenant: acme.clone(), sandbox_id: "sb-1".into(), container_id: "c-1".into(), image: "nginx:1.27".into(), state: "Running" },
            CriContainer { tenant: acme.clone(), sandbox_id: "sb-2".into(), container_id: "c-2".into(), image: "api:v3".into(), state: "Running" },
        ]);
        s.k8s_resources.write().unwrap().extend([
            K8sResource { tenant: acme.clone(), kind: "Deployment".into(), name: "web".into(), namespace: "default".into() },
            K8sResource { tenant: acme.clone(), kind: "Service".into(), name: "web".into(), namespace: "default".into() },
            K8sResource { tenant: evil.clone(), kind: "Deployment".into(), name: "evil-web".into(), namespace: "default".into() },
        ]);
        s.iam_users.write().unwrap().extend([
            IamUser { tenant: acme.clone(), username: "alice".into(), email: "alice@acme".into() },
            IamUser { tenant: acme.clone(), username: "bob".into(), email: "bob@acme".into() },
            IamUser { tenant: evil.clone(), username: "mallory".into(), email: "m@evil".into() },
        ]);
        s.iam_assignments.write().unwrap().extend([
            IamRoleAssignment { tenant: acme.clone(), username: "alice".into(), role: "admin".into() },
            IamRoleAssignment { tenant: acme.clone(), username: "bob".into(), role: "viewer".into() },
        ]);
        s.mesh_authz.write().unwrap().extend([
            MeshAuthzPolicy { tenant: acme.clone(), name: "allow-web".into(), action: "Allow", principal_glob: "spiffe://*/ns/acme/sa/*".into() },
            MeshAuthzPolicy { tenant: evil.clone(), name: "evil-allow".into(), action: "Allow", principal_glob: "*".into() },
        ]);
        s.mesh_flows.write().unwrap().extend([
            MeshFlow { tenant: acme.clone(), source: "web".into(), destination: "api".into(), verdict: "Forwarded", bytes: 100 },
            MeshFlow { tenant: acme.clone(), source: "web".into(), destination: "api".into(), verdict: "Dropped", bytes: 0 },
        ]);
        s.pg_tables.write().unwrap().extend([
            PgTable { tenant: acme.clone(), schema: "public".into(), name: "users".into(), row_count: 1234 },
            PgTable { tenant: acme.clone(), schema: "public".into(), name: "orders".into(), row_count: 99 },
            PgTable { tenant: evil.clone(), schema: "public".into(), name: "secret".into(), row_count: 0 },
        ]);
        s.vault_secrets.write().unwrap().extend([
            VaultSecretMeta { tenant: acme.clone(), path: "kv/db".into(), version: 3, created_unix: 1_000_000 },
            VaultSecretMeta { tenant: acme.clone(), path: "kv/api".into(), version: 1, created_unix: 1_000_500 },
            VaultSecretMeta { tenant: evil.clone(), path: "kv/secret".into(), version: 7, created_unix: 999_999 },
        ]);
        s.vault_audit.write().unwrap().extend([
            VaultAuditEntry { tenant: acme.clone(), time_unix: 1_000_001, principal: "alice".into(), op: "read-meta", path: "kv/db".into() },
            VaultAuditEntry { tenant: acme.clone(), time_unix: 1_000_010, principal: "bob".into(), op: "read-meta", path: "kv/api".into() },
        ]);
        s.vault_engines.write().unwrap().extend([
            VaultSecretsEngine { tenant: acme.clone(), path: "kv/".into(), engine_type: "kv".into(), version: 2, default_lease_ttl_s: 0, enabled: true },
            VaultSecretsEngine { tenant: acme.clone(), path: "transit/".into(), engine_type: "transit".into(), version: 1, default_lease_ttl_s: 0, enabled: true },
            VaultSecretsEngine { tenant: acme.clone(), path: "pki-int/".into(), engine_type: "pki".into(), version: 1, default_lease_ttl_s: 86400, enabled: true },
            VaultSecretsEngine { tenant: acme.clone(), path: "database/".into(), engine_type: "database".into(), version: 1, default_lease_ttl_s: 3600, enabled: true },
            VaultSecretsEngine { tenant: acme.clone(), path: "legacy-kv/".into(), engine_type: "kv".into(), version: 1, default_lease_ttl_s: 0, enabled: false },
            // Foreign tenant — must be filtered out by tenant scoping.
            VaultSecretsEngine { tenant: evil.clone(), path: "kv/".into(), engine_type: "kv".into(), version: 2, default_lease_ttl_s: 0, enabled: true },
        ]);
        s.vault_auth_methods.write().unwrap().extend([
            VaultAuthMethod { tenant: acme.clone(), path: "token/".into(), method_type: "token".into(), accessor: "auth_token_a1".into(), default_lease_ttl_s: 0, enabled: true },
            VaultAuthMethod { tenant: acme.clone(), path: "userpass/".into(), method_type: "userpass".into(), accessor: "auth_userpass_b2".into(), default_lease_ttl_s: 3600, enabled: true },
            VaultAuthMethod { tenant: acme.clone(), path: "kubernetes/".into(), method_type: "kubernetes".into(), accessor: "auth_kubernetes_c3".into(), default_lease_ttl_s: 1800, enabled: true },
            VaultAuthMethod { tenant: acme.clone(), path: "approle/".into(), method_type: "approle".into(), accessor: "auth_approle_d4".into(), default_lease_ttl_s: 1800, enabled: true },
            VaultAuthMethod { tenant: acme.clone(), path: "oidc/".into(), method_type: "oidc".into(), accessor: "auth_oidc_e5".into(), default_lease_ttl_s: 3600, enabled: false },
            VaultAuthMethod { tenant: evil.clone(), path: "token/".into(), method_type: "token".into(), accessor: "auth_token_evil".into(), default_lease_ttl_s: 0, enabled: true },
        ]);
        s.vault_policies.write().unwrap().extend([
            VaultPolicy {
                tenant: acme.clone(),
                name: "default".into(),
                rules: r#"path "kv/data/*" { capabilities = ["read", "list"] }"#.into(),
                bound_token_count: 12,
            },
            VaultPolicy {
                tenant: acme.clone(),
                name: "db-admin".into(),
                rules: r#"path "database/creds/*" { capabilities = ["read"] }"#.into(),
                bound_token_count: 3,
            },
            VaultPolicy {
                tenant: acme.clone(),
                name: "pki-ca".into(),
                rules: r#"path "pki-int/issue/*" { capabilities = ["create", "update"] }"#.into(),
                bound_token_count: 1,
            },
            VaultPolicy {
                tenant: evil.clone(),
                name: "evil-default".into(),
                rules: r#"path "kv/*" { capabilities = ["read", "write"] }"#.into(),
                bound_token_count: 1,
            },
        ]);
        s.keda_scaled_objects.write().unwrap().extend([
            KedaScaledObject {
                tenant: acme.clone(),
                name: "ingest-worker".into(),
                target_ref: "Deployment/ingest-worker".into(),
                min_replicas: 1,
                max_replicas: 50,
                current_replicas: 8,
                paused: false,
                triggers: vec!["kafka".into(), "prometheus".into()],
            },
            KedaScaledObject {
                tenant: acme.clone(),
                name: "report-runner".into(),
                target_ref: "Deployment/report-runner".into(),
                min_replicas: 0,
                max_replicas: 10,
                current_replicas: 0,
                paused: true,
                triggers: vec!["cron".into()],
            },
            KedaScaledObject {
                tenant: evil.clone(),
                name: "evil-worker".into(),
                target_ref: "Deployment/evil-worker".into(),
                min_replicas: 1,
                max_replicas: 5,
                current_replicas: 1,
                paused: false,
                triggers: vec!["cpu".into()],
            },
        ]);
        s.keda_scaler_events.write().unwrap().extend([
            KedaScalerEvent {
                tenant: acme.clone(),
                when_unix: 1_000_400,
                scaled_object: "ingest-worker".into(),
                trigger: "kafka:lag=2400".into(),
                from_replicas: 4,
                to_replicas: 8,
                verdict: "Scaled",
            },
            KedaScalerEvent {
                tenant: acme.clone(),
                when_unix: 1_000_450,
                scaled_object: "ingest-worker".into(),
                trigger: "kafka:lag=2300".into(),
                from_replicas: 8,
                to_replicas: 8,
                verdict: "NoChange",
            },
            KedaScalerEvent {
                tenant: evil.clone(),
                when_unix: 1_000_460,
                scaled_object: "evil-worker".into(),
                trigger: "cpu:99".into(),
                from_replicas: 1,
                to_replicas: 5,
                verdict: "Scaled",
            },
        ]);
        s.recent_activity.write().unwrap().extend([
            ActivityEntry { tenant: acme.clone(), when_unix: 1_000_100, kind: "deploy", summary: "deployed web v17".into() },
            ActivityEntry { tenant: acme.clone(), when_unix: 1_000_200, kind: "policy", summary: "updated AuthorizationPolicy allow-web".into() },
            ActivityEntry { tenant: evil.clone(), when_unix: 1_000_300, kind: "deploy", summary: "deployed evil-web v1".into() },
        ]);
        seed_keda_rich(&s, &acme, &evil);
        s.scheduler_nodes.write().unwrap().extend([
            SchedulerNode { tenant: acme.clone(), name: "node-a".into(), ready: true, allocatable_cpu_milli: 8000, allocatable_mem_mib: 16384, taints: vec![] },
            SchedulerNode { tenant: acme.clone(), name: "node-b".into(), ready: false, allocatable_cpu_milli: 4000, allocatable_mem_mib: 8192, taints: vec!["NoSchedule=cordoned".into()] },
            SchedulerNode { tenant: evil.clone(), name: "evil-node".into(), ready: true, allocatable_cpu_milli: 1000, allocatable_mem_mib: 2048, taints: vec![] },
        ]);
        s.scheduler_policies.write().unwrap().extend([
            SchedulerPolicy { tenant: acme.clone(), name: "least-utilised".into(), predicate: "cpu<70".into(), weight: 5 },
            SchedulerPolicy { tenant: evil.clone(), name: "evil-pin".into(), predicate: "host==evil-node".into(), weight: 10 },
        ]);
        s.controller_leases.write().unwrap().extend([
            ControllerLease { tenant: acme.clone(), controller: "deployment".into(), leader_id: "ctl-1".into(), renewals: 17, expires_unix: 1_001_000 },
            ControllerLease { tenant: acme.clone(), controller: "replicaset".into(), leader_id: "ctl-1".into(), renewals: 18, expires_unix: 1_001_010 },
            ControllerLease { tenant: evil.clone(), controller: "evil-loop".into(), leader_id: "evil-1".into(), renewals: 1, expires_unix: 1_001_020 },
        ]);
        s.kubelet_pods.write().unwrap().extend([
            KubeletPod { tenant: acme.clone(), node: "node-a".into(), pod_name: "web-0".into(), status: "Running", restart_count: 0 },
            KubeletPod { tenant: acme.clone(), node: "node-a".into(), pod_name: "api-0".into(), status: "Running", restart_count: 1 },
            KubeletPod { tenant: acme.clone(), node: "node-b".into(), pod_name: "worker-0".into(), status: "Pending", restart_count: 0 },
            KubeletPod { tenant: evil.clone(), node: "evil-node".into(), pod_name: "x-0".into(), status: "Running", restart_count: 99 },
        ]);
        s.cloud_volumes.write().unwrap().extend([
            CloudVolume { tenant: acme.clone(), id: "vol-1".into(), region: "eu-central-1".into(), size_gb: 50, attached_node: Some("node-a".into()) },
            CloudVolume { tenant: acme.clone(), id: "vol-2".into(), region: "eu-central-1".into(), size_gb: 100, attached_node: None },
            CloudVolume { tenant: evil.clone(), id: "evil-vol".into(), region: "us-east-1".into(), size_gb: 1024, attached_node: None },
        ]);
        s.kamaji_tcps.write().unwrap().extend([
            KamajiTcp { tenant: acme.clone(), name: "tcp-prod".into(), k8s_version: "1.31.2".into(), ready_replicas: 3, desired_replicas: 3 },
            KamajiTcp { tenant: acme.clone(), name: "tcp-staging".into(), k8s_version: "1.31.0".into(), ready_replicas: 2, desired_replicas: 3 },
            KamajiTcp { tenant: evil.clone(), name: "evil-tcp".into(), k8s_version: "1.27.0".into(), ready_replicas: 1, desired_replicas: 1 },
        ]);
        s.net_endpoints.write().unwrap().extend([
            NetEndpoint { tenant: acme.clone(), identity: 1001, namespace: "default".into(), ip: "10.0.0.5".into(), ready: true },
            NetEndpoint { tenant: acme.clone(), identity: 1002, namespace: "default".into(), ip: "10.0.0.6".into(), ready: true },
            NetEndpoint { tenant: evil.clone(), identity: 9001, namespace: "default".into(), ip: "10.0.99.99".into(), ready: true },
        ]);
        s.net_policies.write().unwrap().extend([
            NetPolicy { tenant: acme.clone(), name: "allow-web".into(), direction: "Ingress", selector: "app=web".into() },
            NetPolicy { tenant: evil.clone(), name: "evil-allow-all".into(), direction: "Both", selector: "*".into() },
        ]);
        s.rdbms_clusters.write().unwrap().extend([
            RdbmsCluster { tenant: acme.clone(), name: "pg-prod".into(), version: "16.2".into(), replicas: 3, primary_node: "node-a".into() },
            RdbmsCluster { tenant: evil.clone(), name: "evil-pg".into(), version: "13.0".into(), replicas: 1, primary_node: "evil-node".into() },
        ]);
        s.docdb_collections.write().unwrap().extend([
            DocdbCollection { tenant: acme.clone(), database: "orders".into(), collection: "items".into(), document_count: 10_000 },
            DocdbCollection { tenant: acme.clone(), database: "orders".into(), collection: "ledger".into(), document_count: 250 },
            DocdbCollection { tenant: evil.clone(), database: "secrets".into(), collection: "tokens".into(), document_count: 5 },
        ]);
        s.cache_entries.write().unwrap().extend([
            CacheEntry { tenant: acme.clone(), namespace: "session".into(), key: "u-1".into(), ttl_seconds: 3600, size_bytes: 256 },
            CacheEntry { tenant: acme.clone(), namespace: "session".into(), key: "u-2".into(), ttl_seconds: 1800, size_bytes: 256 },
            CacheEntry { tenant: evil.clone(), namespace: "session".into(), key: "evil-1".into(), ttl_seconds: 60, size_bytes: 999 },
        ]);
        s.rdbms_operator_clusters.write().unwrap().extend([
            RdbmsOperatorCluster { tenant: acme.clone(), name: "primary-prod".into(), upstream_kind: "CNPG", version: "1.24.0".into(), instances: 3, primary_pod: "primary-prod-1".into(), replication_lag_bytes: 8192, replication_state: "InSync" },
            RdbmsOperatorCluster { tenant: acme.clone(), name: "analytics".into(), upstream_kind: "CNPG", version: "1.24.0".into(), instances: 2, primary_pod: "analytics-1".into(), replication_lag_bytes: 4_194_304, replication_state: "Catchup" },
            RdbmsOperatorCluster { tenant: evil.clone(), name: "evil-cluster".into(), upstream_kind: "CNPG", version: "1.10.0".into(), instances: 1, primary_pod: "evil-1".into(), replication_lag_bytes: 0, replication_state: "InSync" },
        ]);
        s.rdbms_operator_backups.write().unwrap().extend([
            RdbmsOperatorBackup { tenant: acme.clone(), cluster: "primary-prod".into(), backup_id: "bk-2026-05-10-01".into(), started_unix: 1_001_500, finished_unix: Some(1_001_700), size_mib: 4096, state: "Completed" },
            RdbmsOperatorBackup { tenant: acme.clone(), cluster: "primary-prod".into(), backup_id: "bk-2026-05-10-02".into(), started_unix: 1_002_000, finished_unix: None, size_mib: 0, state: "Running" },
            RdbmsOperatorBackup { tenant: evil.clone(), cluster: "evil-cluster".into(), backup_id: "evil-bk-1".into(), started_unix: 1_000_000, finished_unix: Some(1_000_100), size_mib: 16, state: "Completed" },
        ]);
        s.lakehouse_tables.write().unwrap().extend([
            LakehouseTable { tenant: acme.clone(), namespace: "warehouse".into(), name: "orders".into(), format_version: 2, partition_count: 365, file_count: 4_320, size_bytes: 1_073_741_824, current_snapshot_id: 1001 },
            LakehouseTable { tenant: acme.clone(), namespace: "warehouse".into(), name: "events".into(), format_version: 2, partition_count: 90, file_count: 1_120, size_bytes: 268_435_456, current_snapshot_id: 1002 },
            LakehouseTable { tenant: evil.clone(), namespace: "secrets".into(), name: "tokens".into(), format_version: 2, partition_count: 1, file_count: 4, size_bytes: 4096, current_snapshot_id: 9001 },
        ]);
        s.lakehouse_snapshots.write().unwrap().extend([
            LakehouseSnapshot { tenant: acme.clone(), namespace: "warehouse".into(), table: "orders".into(), snapshot_id: 1001, committed_unix: 1_002_500, op: "Append", added_files: 12 },
            LakehouseSnapshot { tenant: acme.clone(), namespace: "warehouse".into(), table: "orders".into(), snapshot_id: 1000, committed_unix: 1_002_300, op: "Overwrite", added_files: 4_320 },
            LakehouseSnapshot { tenant: evil.clone(), namespace: "secrets".into(), table: "tokens".into(), snapshot_id: 9001, committed_unix: 999_999, op: "Append", added_files: 4 },
        ]);
        s.streams_topics.write().unwrap().extend([
            StreamsTopic { tenant: acme.clone(), name: "orders".into(), partitions: 12, replication_factor: 3, retention_ms: 604_800_000, compaction: "Delete" },
            StreamsTopic { tenant: acme.clone(), name: "events".into(), partitions: 24, replication_factor: 3, retention_ms: 86_400_000, compaction: "Compact" },
            StreamsTopic { tenant: evil.clone(), name: "evil-topic".into(), partitions: 1, replication_factor: 1, retention_ms: 3_600_000, compaction: "Delete" },
        ]);
        s.streams_consumer_groups.write().unwrap().extend([
            StreamsConsumerGroup { tenant: acme.clone(), group_id: "orders-consumer".into(), topic: "orders".into(), members: 4, current_offset: 9_500, log_end_offset: 10_000, state: "Stable" },
            StreamsConsumerGroup { tenant: acme.clone(), group_id: "events-consumer".into(), topic: "events".into(), members: 2, current_offset: 5_000, log_end_offset: 50_000, state: "Rebalancing" },
            StreamsConsumerGroup { tenant: evil.clone(), group_id: "evil-consumer".into(), topic: "evil-topic".into(), members: 1, current_offset: 0, log_end_offset: 0, state: "Empty" },
        ]);
        s.streams_connectors.write().unwrap().extend([
            StreamsConnector { tenant: acme.clone(), name: "jdbc-source".into(), kind: "Source", state: "Running", tasks_max: 2, class: "io.confluent.connect.jdbc.JdbcSourceConnector".into(), topics: "orders,refunds".into() },
            StreamsConnector { tenant: acme.clone(), name: "hdfs-sink".into(), kind: "Sink", state: "Failed", tasks_max: 1, class: "io.confluent.connect.hdfs.HdfsSinkConnector".into(), topics: "events".into() },
            StreamsConnector { tenant: evil.clone(), name: "evil-connector".into(), kind: "Source", state: "Running", tasks_max: 1, class: "evil.SourceConnector".into(), topics: "evil-topic".into() },
        ]);
        s.streams_connect_workers.write().unwrap().extend([
            StreamsConnectWorker { tenant: acme.clone(), id: "worker-1".into(), state: "Running", host: "connect-1.acme.local:8083".into(), connectors_owned: 1, tasks_owned: 2 },
            StreamsConnectWorker { tenant: acme.clone(), id: "worker-2".into(), state: "Running", host: "connect-2.acme.local:8083".into(), connectors_owned: 1, tasks_owned: 1 },
            StreamsConnectWorker { tenant: evil.clone(), id: "evil-worker".into(), state: "Running", host: "evil.local:8083".into(), connectors_owned: 1, tasks_owned: 1 },
        ]);
        s.streams_connect_tasks.write().unwrap().extend([
            StreamsConnectTask { tenant: acme.clone(), connector: "jdbc-source".into(), task: 0, state: "Running", failure_trace: None },
            StreamsConnectTask { tenant: acme.clone(), connector: "jdbc-source".into(), task: 1, state: "Running", failure_trace: None },
            StreamsConnectTask { tenant: acme.clone(), connector: "hdfs-sink".into(), task: 0, state: "Failed", failure_trace: Some("java.io.IOException: connection refused".into()) },
            StreamsConnectTask { tenant: evil.clone(), connector: "evil-connector".into(), task: 0, state: "Running", failure_trace: None },
        ]);
        s.policy_rules.write().unwrap().extend([
            PolicyRule { tenant: acme.clone(), name: "deny-internet-prod".into(), action: "Deny", subject: "spiffe://*/ns/prod/sa/*".into(), resource: "egress:0.0.0.0/0".into(), enabled: true },
            PolicyRule { tenant: acme.clone(), name: "allow-monitoring".into(), action: "Allow", subject: "spiffe://*/ns/monitoring/sa/prom".into(), resource: "/metrics".into(), enabled: true },
            PolicyRule { tenant: evil.clone(), name: "evil-allow-all".into(), action: "Allow", subject: "*".into(), resource: "*".into(), enabled: true },
        ]);
        s.artifact_records.write().unwrap().extend([
            ArtifactRecord { tenant: acme.clone(), registry: "registry.acme/web".into(), name: "web:v17".into(), digest: "sha256:aaaa1111".into(), size_bytes: 67_108_864, pushed_unix: 1_001_000 },
            ArtifactRecord { tenant: acme.clone(), registry: "registry.acme/api".into(), name: "api:v3".into(), digest: "sha256:bbbb2222".into(), size_bytes: 134_217_728, pushed_unix: 1_001_500 },
            ArtifactRecord { tenant: evil.clone(), registry: "registry.evil/x".into(), name: "x:latest".into(), digest: "sha256:cccc3333".into(), size_bytes: 1024, pushed_unix: 999_000 },
        ]);
        s.alert_rules.write().unwrap().extend([
            AlertRule { tenant: acme.clone(), name: "HighErrorRate".into(), severity: "critical", expr: "rate(errors[5m]) > 0.05".into(), for_seconds: 300 },
            AlertRule { tenant: acme.clone(), name: "DiskPressure".into(), severity: "warning", expr: "disk_free < 0.10".into(), for_seconds: 600 },
            AlertRule { tenant: evil.clone(), name: "EvilNoiseAlert".into(), severity: "info", expr: "evil > 0".into(), for_seconds: 30 },
        ]);
        s.active_alerts.write().unwrap().extend([
            ActiveAlert { tenant: acme.clone(), rule: "HighErrorRate".into(), state: "firing", fired_unix: 1_002_100 },
            ActiveAlert { tenant: evil.clone(), rule: "EvilNoiseAlert".into(), state: "pending", fired_unix: 1_002_200 },
        ]);
        s.backup_jobs.write().unwrap().extend([
            BackupJob { tenant: acme.clone(), name: "pg-prod-daily".into(), source: "rdbms://pg-prod".into(), destination: "s3://backups/pg-prod/".into(), schedule_cron: "0 2 * * *".into(), last_run_unix: Some(1_002_000), state: "Completed" },
            BackupJob { tenant: acme.clone(), name: "etcd-hourly".into(), source: "etcd://cluster-a".into(), destination: "s3://backups/etcd/".into(), schedule_cron: "0 * * * *".into(), last_run_unix: Some(1_002_500), state: "Running" },
            BackupJob { tenant: evil.clone(), name: "evil-backup".into(), source: "evil".into(), destination: "evil".into(), schedule_cron: "* * * * *".into(), last_run_unix: None, state: "Scheduled" },
        ]);
        s.incident_records.write().unwrap().extend([
            IncidentRecord { tenant: acme.clone(), id: "INC-2026-001".into(), title: "API latency spike".into(), severity: "SEV2", state: "Investigating", opened_unix: 1_002_300 },
            IncidentRecord { tenant: acme.clone(), id: "INC-2026-002".into(), title: "DB failover".into(), severity: "SEV1", state: "Resolved", opened_unix: 1_001_900 },
            IncidentRecord { tenant: evil.clone(), id: "EVIL-001".into(), title: "evil disruption".into(), severity: "SEV4", state: "Open", opened_unix: 999_000 },
        ]);
        s.vuln_records.write().unwrap().extend([
            VulnRecord { tenant: acme.clone(), cve_id: "CVE-2025-0001".into(), package: "openssl".into(), installed_version: "3.0.10".into(), fixed_version: Some("3.0.14".into()), severity: "Critical" },
            VulnRecord { tenant: acme.clone(), cve_id: "CVE-2025-0042".into(), package: "tokio".into(), installed_version: "1.40.0".into(), fixed_version: None, severity: "Medium" },
            VulnRecord { tenant: evil.clone(), cve_id: "CVE-2025-9999".into(), package: "evil-lib".into(), installed_version: "0.1.0".into(), fixed_version: None, severity: "Low" },
        ]);
        s.workflow_runs.write().unwrap().extend([
            WorkflowRun { tenant: acme.clone(), name: "etl-orders".into(), run_id: "wf-1001".into(), status: "Succeeded", started_unix: 1_001_800, finished_unix: Some(1_001_900) },
            WorkflowRun { tenant: acme.clone(), name: "etl-orders".into(), run_id: "wf-1002".into(), status: "Running", started_unix: 1_002_400, finished_unix: None },
            WorkflowRun { tenant: evil.clone(), name: "evil-wf".into(), run_id: "evil-1".into(), status: "Failed", started_unix: 999_000, finished_unix: Some(999_100) },
        ]);
        s.chaos_experiments.write().unwrap().extend([
            ChaosExperiment { tenant: acme.clone(), name: "kill-web-pod".into(), kind: "pod-kill".into(), target_selector: "app=web".into(), schedule: "Cron", last_run_unix: Some(1_002_000) },
            ChaosExperiment { tenant: acme.clone(), name: "delay-api-egress".into(), kind: "network-delay".into(), target_selector: "app=api".into(), schedule: "Once", last_run_unix: None },
            ChaosExperiment { tenant: evil.clone(), name: "evil-chaos".into(), kind: "full-cluster".into(), target_selector: "*".into(), schedule: "Continuous", last_run_unix: Some(1_000_000) },
        ]);
        s.slos.write().unwrap().extend([
            Slo { tenant: acme.clone(), name: "web-availability".into(), service: "web".into(), objective_pct: 99.9, window_days: 30, current_pct: 99.94, error_budget_remaining_pct: 60.0 },
            Slo { tenant: acme.clone(), name: "api-latency-p99".into(), service: "api".into(), objective_pct: 99.0, window_days: 30, current_pct: 98.7, error_budget_remaining_pct: -30.0 },
            Slo { tenant: evil.clone(), name: "evil-slo".into(), service: "evil".into(), objective_pct: 50.0, window_days: 7, current_pct: 100.0, error_budget_remaining_pct: 100.0 },
        ]);
        s.ai_model_metrics.write().unwrap().extend([
            AiModelMetric { tenant: acme.clone(), model: "gpt-4".into(), tokens_in: 1000000, tokens_out: 500000, latency_p99_ms: 250 },
            AiModelMetric { tenant: acme.clone(), model: "claude-3".into(), tokens_in: 2000000, tokens_out: 800000, latency_p99_ms: 180 },
            AiModelMetric { tenant: evil.clone(), model: "evil-model".into(), tokens_in: 1, tokens_out: 1, latency_p99_ms: 9999 },
        ]);
        s.chat_threads.write().unwrap().extend([
            ChatThread { tenant: acme.clone(), id: "thr-1".into(), topic: "deploy-coord".into(), members: 8, last_message_unix: 1001000 },
            ChatThread { tenant: acme.clone(), id: "thr-2".into(), topic: "incident-response".into(), members: 12, last_message_unix: 1001500 },
            ChatThread { tenant: evil.clone(), id: "evil-thr".into(), topic: "evil".into(), members: 1, last_message_unix: 999000 },
        ]);
        s.cost_reports.write().unwrap().extend([
            CostReport { tenant: acme.clone(), period: "2026-05".into(), service: "compute".into(), amount_cents: 1250000 },
            CostReport { tenant: acme.clone(), period: "2026-05".into(), service: "storage".into(), amount_cents: 320000 },
            CostReport { tenant: evil.clone(), period: "2026-05".into(), service: "evil".into(), amount_cents: 999 },
        ]);
        s.dast_scans.write().unwrap().extend([
            DastScan { tenant: acme.clone(), target: "https://api.acme/".into(), scan_id: "dast-001".into(), findings: 3, severity: "medium" },
            DastScan { tenant: acme.clone(), target: "https://web.acme/".into(), scan_id: "dast-002".into(), findings: 12, severity: "high" },
            DastScan { tenant: evil.clone(), target: "https://evil/".into(), scan_id: "evil-1".into(), findings: 0, severity: "info" },
        ]);
        s.devlake_metrics.write().unwrap().extend([
            DevlakeMetric { tenant: acme.clone(), project: "acme-web".into(), metric: "deploy_freq".into(), value_thousandths: 1500 },
            DevlakeMetric { tenant: acme.clone(), project: "acme-api".into(), metric: "lead_time_hours".into(), value_thousandths: 24000 },
            DevlakeMetric { tenant: evil.clone(), project: "evil".into(), metric: "evil_metric".into(), value_thousandths: 1 },
        ]);
        s.forensics_evidence.write().unwrap().extend([
            ForensicsEvidence { tenant: acme.clone(), case_id: "CASE-001".into(), artifact: "memdump-pod-web-0".into(), collected_unix: 1001000, digest: "sha256:aaa1".into() },
            ForensicsEvidence { tenant: acme.clone(), case_id: "CASE-002".into(), artifact: "pcap-2026-05-11".into(), collected_unix: 1001500, digest: "sha256:bbb2".into() },
            ForensicsEvidence { tenant: evil.clone(), case_id: "EVIL-001".into(), artifact: "evil-evidence".into(), collected_unix: 999000, digest: "sha256:evil".into() },
        ]);
        s.gateway_routes.write().unwrap().extend([
            GatewayRoute { tenant: acme.clone(), name: "web-route".into(), listener: "https-443".into(), hostname: "acme.com".into(), backend: "svc/web:80".into() },
            GatewayRoute { tenant: acme.clone(), name: "api-route".into(), listener: "https-443".into(), hostname: "api.acme.com".into(), backend: "svc/api:8080".into() },
            GatewayRoute { tenant: evil.clone(), name: "evil-route".into(), listener: "http-80".into(), hostname: "evil.com".into(), backend: "svc/evil:1".into() },
        ]);
        s.infra_stacks.write().unwrap().extend([
            InfraStack { tenant: acme.clone(), name: "prod-vpc".into(), provider: "aws".into(), region: "eu-central-1".into(), resources: 24, state: "Ok" },
            InfraStack { tenant: acme.clone(), name: "staging-vpc".into(), provider: "hetzner".into(), region: "hel1".into(), resources: 12, state: "Drift" },
            InfraStack { tenant: evil.clone(), name: "evil-vpc".into(), provider: "evil".into(), region: "nowhere".into(), resources: 1, state: "Failed" },
        ]);
        s.pam_sessions.write().unwrap().extend([
            PamSession { tenant: acme.clone(), id: "sess-1".into(), principal: "alice@acme".into(), target: "pg-prod-1".into(), started_unix: 1001000, ended_unix: Some(1_001_300) },
            PamSession { tenant: acme.clone(), id: "sess-2".into(), principal: "bob@acme".into(), target: "etcd-cluster".into(), started_unix: 1001500, ended_unix: None },
            PamSession { tenant: evil.clone(), id: "evil-sess".into(), principal: "mallory@evil".into(), target: "any".into(), started_unix: 999000, ended_unix: None },
        ]);
        s.sbom_components.write().unwrap().extend([
            SbomComponent { tenant: acme.clone(), image: "web:v17".into(), package: "openssl".into(), version: "3.0.14".into(), license: "Apache-2.0".into() },
            SbomComponent { tenant: acme.clone(), image: "web:v17".into(), package: "tokio".into(), version: "1.40.0".into(), license: "MIT".into() },
            SbomComponent { tenant: evil.clone(), image: "evil:x".into(), package: "evil-pkg".into(), version: "0.1.0".into(), license: "Unknown".into() },
        ]);
        s.scan_results.write().unwrap().extend([
            ScanResult { tenant: acme.clone(), scan_id: "scan-1".into(), scanner: "trivy".into(), findings: 5, worst_severity: "High" },
            ScanResult { tenant: acme.clone(), scan_id: "scan-2".into(), scanner: "grype".into(), findings: 1, worst_severity: "Low" },
            ScanResult { tenant: evil.clone(), scan_id: "evil-scan".into(), scanner: "evil-scanner".into(), findings: 99, worst_severity: "Critical" },
        ]);
        s.secret_metadatas.write().unwrap().extend([
            SecretMetadata { tenant: acme.clone(), path: "app/db-password".into(), backend: "vault-kv".into(), version: 3, created_unix: 1001000 },
            SecretMetadata { tenant: acme.clone(), path: "app/api-token".into(), backend: "vault-kv".into(), version: 1, created_unix: 1001500 },
            SecretMetadata { tenant: evil.clone(), path: "evil/secret".into(), backend: "evil-kv".into(), version: 1, created_unix: 999000 },
        ]);
        s.uptime_probes.write().unwrap().extend([
            UptimeProbe { tenant: acme.clone(), name: "web-prod".into(), url: "https://web.acme/health".into(), interval_seconds: 60, last_status: "Up" },
            UptimeProbe { tenant: acme.clone(), name: "api-prod".into(), url: "https://api.acme/health".into(), interval_seconds: 30, last_status: "Up" },
            UptimeProbe { tenant: evil.clone(), name: "evil-probe".into(), url: "https://evil/".into(), interval_seconds: 30, last_status: "Down" },
        ]);
        s.kube_clusters.write().unwrap().extend([
            KubeCluster { tenant: acme.clone(), name: "prod".into(), k8s_version: "1.31.2".into(), nodes: 12, state: "Ready" },
            KubeCluster { tenant: acme.clone(), name: "staging".into(), k8s_version: "1.31.0".into(), nodes: 4, state: "Upgrading" },
            KubeCluster { tenant: evil.clone(), name: "evil-k8s".into(), k8s_version: "1.27.0".into(), nodes: 1, state: "Unknown" },
        ]);
        s.kube_proxy_services.write().unwrap().extend([
            KubeProxyService { tenant: acme.clone(), name: "web".into(), namespace: "default".into(), cluster_ip: "10.96.10.5".into(), backend_count: 3 },
            KubeProxyService { tenant: acme.clone(), name: "api".into(), namespace: "default".into(), cluster_ip: "10.96.10.6".into(), backend_count: 5 },
            KubeProxyService { tenant: evil.clone(), name: "evil-svc".into(), namespace: "default".into(), cluster_ip: "10.96.99.99".into(), backend_count: 1 },
        ]);
        s.store_buckets.write().unwrap().extend([
            StoreBucket { tenant: acme.clone(), name: "prod-images".into(), backend: "s3".into(), object_count: 12345, size_bytes: 5368709120 },
            StoreBucket { tenant: acme.clone(), name: "prod-logs".into(), backend: "s3".into(), object_count: 1000000, size_bytes: 21474836480 },
            StoreBucket { tenant: evil.clone(), name: "evil-bucket".into(), backend: "s3".into(), object_count: 1, size_bytes: 1 },
        ]);
        s.metric_series.write().unwrap().extend([
            MetricSeries { tenant: acme.clone(), name: "http_requests_total".into(), scraper: "prometheus-prod".into(), sample_count: 1000000000, retention_days: 30 },
            MetricSeries { tenant: acme.clone(), name: "cpu_seconds_total".into(), scraper: "prometheus-prod".into(), sample_count: 500000000, retention_days: 30 },
            MetricSeries { tenant: evil.clone(), name: "evil_metric".into(), scraper: "evil-scraper".into(), sample_count: 1, retention_days: 1 },
        ]);
        s.trace_services.write().unwrap().extend([
            TraceService { tenant: acme.clone(), service: "web".into(), span_count_per_sec: 1500, error_rate_per_thousand: 5 },
            TraceService { tenant: acme.clone(), service: "api".into(), span_count_per_sec: 800, error_rate_per_thousand: 12 },
            TraceService { tenant: evil.clone(), service: "evil-svc".into(), span_count_per_sec: 1, error_rate_per_thousand: 999 },
        ]);
        s.auth_sessions.write().unwrap().extend([
            AuthSession { tenant: acme.clone(), session_id: "sess-aaa".into(), principal: "alice@acme".into(), realm: "acme-realm".into(), expires_unix: 1010000 },
            AuthSession { tenant: acme.clone(), session_id: "sess-bbb".into(), principal: "bob@acme".into(), realm: "acme-realm".into(), expires_unix: 1020000 },
            AuthSession { tenant: evil.clone(), session_id: "sess-evil".into(), principal: "mallory@evil".into(), realm: "evil-realm".into(), expires_unix: 999999 },
        ]);
        s.dashboard_catalog.write().unwrap().extend([
            DashboardCatalog { tenant: acme.clone(), uid: "web-dash".into(), title: "Web Service".into(), folder: "prod".into(), panels: 12 },
            DashboardCatalog { tenant: acme.clone(), uid: "api-dash".into(), title: "API Service".into(), folder: "prod".into(), panels: 16 },
            DashboardCatalog { tenant: evil.clone(), uid: "evil-dash".into(), title: "Evil".into(), folder: "evil".into(), panels: 1 },
        ]);
        s.dns_zones.write().unwrap().extend([
            DnsZone { tenant: acme.clone(), zone: "acme.com".into(), record_count: 24, serial: 2026051101 },
            DnsZone { tenant: acme.clone(), zone: "acme.io".into(), record_count: 12, serial: 2026051102 },
            DnsZone { tenant: evil.clone(), zone: "evil.com".into(), record_count: 1, serial: 1 },
        ]);
        s.log_streams.write().unwrap().extend([
            LogStream { tenant: acme.clone(), name: "web-stdout".into(), sink: "loki".into(), ingest_rate_per_sec: 5000, retention_days: 14 },
            LogStream { tenant: acme.clone(), name: "api-stdout".into(), sink: "loki".into(), ingest_rate_per_sec: 8000, retention_days: 14 },
            LogStream { tenant: evil.clone(), name: "evil-stream".into(), sink: "evil-sink".into(), ingest_rate_per_sec: 1, retention_days: 1 },
        ]);
        s.security_events.write().unwrap().extend([
            SecurityEvent { tenant: acme.clone(), id: "sec-1".into(), kind: "brute-force-detected".into(), severity: "high", at_unix: 1002000 },
            SecurityEvent { tenant: acme.clone(), id: "sec-2".into(), kind: "anomalous-egress".into(), severity: "medium", at_unix: 1002500 },
            SecurityEvent { tenant: evil.clone(), id: "sec-evil".into(), kind: "evil".into(), severity: "info", at_unix: 999000 },
        ]);
        s.ha_failover_events.write().unwrap().extend([
            HaFailoverEvent { tenant: acme.clone(), id: "fo-1".into(), subject: "pg-prod".into(), old_primary: "pg-prod-1".into(), new_primary: "pg-prod-2".into(), at_unix: 1003000 },
            HaFailoverEvent { tenant: evil.clone(), id: "fo-evil".into(), subject: "evil".into(), old_primary: "e-1".into(), new_primary: "e-2".into(), at_unix: 999000 },
        ]);
        s.erp_invoices.write().unwrap().extend([
            ErpInvoice { tenant: acme.clone(), invoice_id: "INV-001".into(), customer: "ACME-CUST-1".into(), amount_cents: 250000, status: "Paid" },
            ErpInvoice { tenant: acme.clone(), invoice_id: "INV-002".into(), customer: "ACME-CUST-2".into(), amount_cents: 1000000, status: "Open" },
            ErpInvoice { tenant: evil.clone(), invoice_id: "EVIL-INV".into(), customer: "EVIL-C".into(), amount_cents: 1, status: "Void" },
        ]);
        s.deploy_activities.write().unwrap().extend([
            DeployActivity { tenant: acme.clone(), id: "dep-001".into(), service: "web".into(), version: "v17".into(), status: "Succeeded" },
            DeployActivity { tenant: acme.clone(), id: "dep-002".into(), service: "api".into(), version: "v3".into(), status: "InProgress" },
            DeployActivity { tenant: evil.clone(), id: "evil-dep".into(), service: "evil".into(), version: "x".into(), status: "Failed" },
        ]);
        s.pipeline_runs.write().unwrap().extend([
            PipelineRun { tenant: acme.clone(), pipeline: "build-web".into(), run_id: "run-100".into(), status: "Succeeded", duration_seconds: 120 },
            PipelineRun { tenant: acme.clone(), pipeline: "build-api".into(), run_id: "run-101".into(), status: "Running", duration_seconds: 0 },
            PipelineRun { tenant: evil.clone(), pipeline: "evil-pl".into(), run_id: "evil-run".into(), status: "Failed", duration_seconds: 1 },
        ]);
        s.rollout_statuses.write().unwrap().extend([
            RolloutStatus { tenant: acme.clone(), name: "web-canary".into(), strategy: "Canary", traffic_pct: 25, state: "Progressing" },
            RolloutStatus { tenant: acme.clone(), name: "api-blue-green".into(), strategy: "BlueGreen", traffic_pct: 100, state: "Promoted" },
            RolloutStatus { tenant: evil.clone(), name: "evil-rollout".into(), strategy: "Canary", traffic_pct: 50, state: "Stuck" },
        ]);
        s.knative_services.write().unwrap().extend([
            KnativeService { tenant: acme.clone(), name: "echo-svc".into(), image: "acme/echo:v1".into(), replicas: 2, min_scale: 0, max_scale: 10 },
            KnativeService { tenant: acme.clone(), name: "sentiment-svc".into(), image: "acme/nlp:v2".into(), replicas: 5, min_scale: 1, max_scale: 20 },
            KnativeService { tenant: evil.clone(), name: "evil-svc".into(), image: "evil:x".into(), replicas: 1, min_scale: 0, max_scale: 1 },
        ]);
        s.llm_routes.write().unwrap().extend([
            LlmRoute { tenant: acme.clone(), name: "claude-proxy".into(), upstream: "anthropic.com".into(), rpm_limit: 1000, daily_tokens: 5000000 },
            LlmRoute { tenant: acme.clone(), name: "local-qwen".into(), upstream: "ollama:11434".into(), rpm_limit: 100, daily_tokens: 1000000 },
            LlmRoute { tenant: evil.clone(), name: "evil-route".into(), upstream: "evil".into(), rpm_limit: 1, daily_tokens: 1 },
        ]);
        s.local_llm_models.write().unwrap().extend([
            LocalLlmModel { tenant: acme.clone(), tag: "qwen3.6:35b-a3b-coding-mxfp8".into(), size_bytes: 22000000000, quant: "mxfp8".into(), loaded: true },
            LocalLlmModel { tenant: acme.clone(), tag: "llama3:8b-q4".into(), size_bytes: 5000000000, quant: "q4".into(), loaded: false },
            LocalLlmModel { tenant: evil.clone(), tag: "evil-model".into(), size_bytes: 1, quant: "unknown".into(), loaded: false },
        ]);
        s.tracker_issues.write().unwrap().extend([
            TrackerIssue { tenant: acme.clone(), id: "ISS-100".into(), title: "slow query on orders".into(), state: "Open", assignee: Some("alice@acme".to_string()) },
            TrackerIssue { tenant: acme.clone(), id: "ISS-101".into(), title: "flaky CI".into(), state: "InProgress", assignee: None },
            TrackerIssue { tenant: evil.clone(), id: "EVIL-1".into(), title: "evil bug".into(), state: "Open", assignee: None },
        ]);
        s.upstream_projects.write().unwrap().extend([
            UpstreamProject { tenant: acme.clone(), name: "kubernetes".into(), repo: "kubernetes/kubernetes".into(), pinned_version: "v1.31.2".into(), last_check_unix: 1003000 },
            UpstreamProject { tenant: acme.clone(), name: "istio".into(), repo: "istio/istio".into(), pinned_version: "1.23.0".into(), last_check_unix: 1003100 },
            UpstreamProject { tenant: evil.clone(), name: "evil-upstream".into(), repo: "evil/evil".into(), pinned_version: "0.0.1".into(), last_check_unix: 999000 },
        ]);
        s.container_scan_results.write().unwrap().extend([
            ContainerScanResult { tenant: acme.clone(), image: "web:v17".into(), digest: "sha256:aaa1".into(), critical_cves: 0, scanned_at_unix: 1003500 },
            ContainerScanResult { tenant: acme.clone(), image: "api:v3".into(), digest: "sha256:bbb2".into(), critical_cves: 1, scanned_at_unix: 1003600 },
            ContainerScanResult { tenant: evil.clone(), image: "evil:latest".into(), digest: "sha256:evil".into(), critical_cves: 99, scanned_at_unix: 999000 },
        ]);
        s.admission_decisions.write().unwrap().extend([
            AdmissionDecision { tenant: acme.clone(), id: "dec-1".into(), resource_kind: "Pod".into(), decision: "Allow", reason: "OK".into() },
            AdmissionDecision { tenant: acme.clone(), id: "dec-2".into(), resource_kind: "Deployment".into(), decision: "Deny", reason: "runAsRoot=true".into() },
            AdmissionDecision { tenant: evil.clone(), id: "evil-dec".into(), resource_kind: "Pod".into(), decision: "Allow", reason: "evil".into() },
        ]);
        s.cdc_pipelines.write().unwrap().extend([
            CdcPipeline { tenant: acme.clone(), name: "orders-cdc".into(), source: "pg:orders".into(), sink: "kafka:cdc-orders".into(), state: "Running" },
            CdcPipeline { tenant: acme.clone(), name: "users-cdc".into(), source: "pg:users".into(), sink: "kafka:cdc-users".into(), state: "Paused" },
            CdcPipeline { tenant: evil.clone(), name: "evil-cdc".into(), source: "evil".into(), sink: "evil".into(), state: "Stopped" },
        ]);
        s.cert_records.write().unwrap().extend([
            CertRecord { tenant: acme.clone(), subject: "acme.com".into(), issuer: "Let's Encrypt".into(), not_after_unix: 1700000000, serial: "01:23:45".into() },
            CertRecord { tenant: acme.clone(), subject: "api.acme.com".into(), issuer: "Let's Encrypt".into(), not_after_unix: 1710000000, serial: "01:23:46".into() },
            CertRecord { tenant: evil.clone(), subject: "evil.com".into(), issuer: "evil-ca".into(), not_after_unix: 999999, serial: "00".into() },
        ]);
        s.crm_accounts.write().unwrap().extend([
            CrmAccount { tenant: acme.clone(), id: "acc-1".into(), name: "Acme Robotics".into(), plan: "Enterprise", mrr_cents: 1000000 },
            CrmAccount { tenant: acme.clone(), id: "acc-2".into(), name: "Globex Co".into(), plan: "Pro", mrr_cents: 200000 },
            CrmAccount { tenant: evil.clone(), id: "evil-acc".into(), name: "Evil Corp".into(), plan: "Free", mrr_cents: 0 },
        ]);
        s.crossplane_claims.write().unwrap().extend([
            CrossplaneClaim { tenant: acme.clone(), name: "db-1".into(), kind: "PostgresInstance".into(), composition: "composition-pg".into(), state: "Ready" },
            CrossplaneClaim { tenant: acme.clone(), name: "bucket-1".into(), kind: "S3Bucket".into(), composition: "composition-s3".into(), state: "Provisioning" },
            CrossplaneClaim { tenant: evil.clone(), name: "evil-claim".into(), kind: "evil".into(), composition: "evil-comp".into(), state: "Failed" },
        ]);
        s.gitops_apps.write().unwrap().extend([
            GitopsApp { tenant: acme.clone(), name: "web-app".into(), repo: "acme/k8s-config".into(), path: "apps/web".into(), synced_at_unix: 1003000 },
            GitopsApp { tenant: acme.clone(), name: "api-app".into(), repo: "acme/k8s-config".into(), path: "apps/api".into(), synced_at_unix: 1003100 },
            GitopsApp { tenant: evil.clone(), name: "evil-app".into(), repo: "evil/cfg".into(), path: "apps/evil".into(), synced_at_unix: 999000 },
        ]);
        s.node_pools.write().unwrap().extend([
            NodePool { tenant: acme.clone(), name: "default".into(), instance_class: "m5.large".into(), max_nodes: 20, active_nodes: 12 },
            NodePool { tenant: acme.clone(), name: "gpu".into(), instance_class: "g5.xlarge".into(), max_nodes: 4, active_nodes: 2 },
            NodePool { tenant: evil.clone(), name: "evil-pool".into(), instance_class: "t2.nano".into(), max_nodes: 100, active_nodes: 99 },
        ]);
        s.virtual_machines.write().unwrap().extend([
            VirtualMachine { tenant: acme.clone(), name: "vm-1".into(), phase: "Running", cpu: 4, memory_mib: 8192 },
            VirtualMachine { tenant: acme.clone(), name: "vm-2".into(), phase: "Stopped", cpu: 2, memory_mib: 4096 },
            VirtualMachine { tenant: evil.clone(), name: "evil-vm".into(), phase: "Running", cpu: 64, memory_mib: 65536 },
        ]);
        s.ledger_entries.write().unwrap().extend([
            LedgerEntry { tenant: acme.clone(), id: "led-1".into(), actor: "alice".into(), action: "deploy.create".into(), at_unix: 1003000 },
            LedgerEntry { tenant: acme.clone(), id: "led-2".into(), actor: "bob".into(), action: "policy.update".into(), at_unix: 1003100 },
            LedgerEntry { tenant: evil.clone(), id: "evil-led".into(), actor: "mallory".into(), action: "evil".into(), at_unix: 999000 },
        ]);
        s.oncall_shifts.write().unwrap().extend([
            OncallShift { tenant: acme.clone(), rotation: "sre-primary".into(), oncaller: "alice@acme".into(), start_unix: 1003000, end_unix: 1088400 },
            OncallShift { tenant: acme, rotation: "sre-secondary".into(), oncaller: "bob@acme".into(), start_unix: 1003000, end_unix: 1088400 },
            OncallShift { tenant: evil, rotation: "evil-rotation".into(), oncaller: "mallory@evil".into(), start_unix: 999000, end_unix: 1000000 },
        ]);
        // search seeded skipped: scope already established a/e via earlier blocks
        s.search_indexes.write().unwrap().extend([
            SearchIndex { tenant: TenantId::new("acme").expect("test fixture"), name: "docs-index".into(), doc_count: 100000, size_bytes: 500000000, status: "Healthy" },
            SearchIndex { tenant: TenantId::new("acme").expect("test fixture"), name: "logs-index".into(), doc_count: 1000000000, size_bytes: 50000000000, status: "Healthy" },
            SearchIndex { tenant: TenantId::new("evil").expect("test fixture"), name: "evil-index".into(), doc_count: 1, size_bytes: 1, status: "Degraded" },
        ]);
        s
    }
}

/// Filter helper used by every view: returns only rows belonging to `tenant`.
pub fn scope<'a, T, F>(rows: &'a [T], tenant: &TenantId, f: F) -> Vec<&'a T>
where
    F: Fn(&T) -> &TenantId,
{
    rows.iter().filter(|r| f(r) == tenant).collect()
}

/// Replace all rows for `tenant` in `lock` with `fresh`. Rows belonging
/// to other tenants are preserved — the materialise path is per-tenant,
/// not a global wipe, so two concurrent calls for different tenants
/// don't stomp on each other.
pub fn replace_tenant_rows<T, F>(
    lock: &RwLock<Vec<T>>,
    tenant: &TenantId,
    fresh: Vec<T>,
    tenant_of: F,
) where
    F: Fn(&T) -> &TenantId,
{
    let mut guard = lock.write().expect("admin-state lock poisoned");
    guard.retain(|r| tenant_of(r) != tenant);
    guard.extend(fresh);
}

/// Tally helper used by tenant_dashboard.
pub fn tally_by_kind(rows: &[ActivityEntry], tenant: &TenantId) -> BTreeMap<&'static str, u64> {
    let mut out: BTreeMap<&'static str, u64> = BTreeMap::new();
    for r in rows.iter().filter(|r| &r.tenant == tenant) {
        *out.entry(r.kind).or_insert(0) += 1;
    }
    out
}

/// Seed rich KEDA fixtures (ScaledObject detail, ScaledJobs, TriggerAuth).
/// Kept in a free function rather than inline in `seeded()` so the file
/// stays scannable.
fn seed_keda_rich(s: &AdminState, acme: &TenantId, evil: &TenantId) {
    use crate::admin::keda::types::*;
    s.keda_scaled_object_details
        .write()
        .unwrap()
        .extend([
            KedaScaledObjectDetail {
                tenant: acme.clone(),
                namespace: "ingest".into(),
                name: "ingest-worker".into(),
                annotations: vec![
                    ("autoscaling.keda.sh/paused".into(), "false".into()),
                    ("scaledobject.keda.sh/transfer-hpa-ownership".into(), "true".into()),
                ],
                scale_target_ref: KedaScaleTargetRef {
                    api_version: "apps/v1".into(),
                    kind: "Deployment".into(),
                    name: "ingest-worker".into(),
                    env_source_container_name: Some("worker".into()),
                },
                min_replica_count: 1,
                max_replica_count: 50,
                idle_replica_count: Some(0),
                polling_interval_secs: 30,
                cooldown_period_secs: 300,
                initial_cooldown_period_secs: 0,
                fallback: Some(KedaFallback {
                    failure_threshold: 3,
                    replicas: 4,
                }),
                triggers: vec![
                    KedaTrigger {
                        kind: "kafka".into(),
                        name: Some("ingest-lag".into()),
                        metadata: vec![
                            ("bootstrapServers".into(), "kafka.bus.svc:9092".into()),
                            ("consumerGroup".into(), "ingest".into()),
                            ("topic".into(), "events".into()),
                            ("lagThreshold".into(), "1000".into()),
                        ],
                        auth_ref: Some(KedaAuthRef {
                            name: "kafka-sasl".into(),
                            kind: "TriggerAuthentication".into(),
                        }),
                        metric_type: "AverageValue".into(),
                        use_cached_metrics: false,
                    },
                    KedaTrigger {
                        kind: "prometheus".into(),
                        name: Some("queue-depth".into()),
                        metadata: vec![
                            ("serverAddress".into(), "http://prom.observability.svc:9090".into()),
                            ("query".into(), "sum(ingest_queue_depth)".into()),
                            ("threshold".into(), "100".into()),
                        ],
                        auth_ref: None,
                        metric_type: "AverageValue".into(),
                        use_cached_metrics: true,
                    },
                ],
                advanced: Some(KedaAdvanced {
                    restore_to_original_replica_count: true,
                    hpa_name: None,
                    hpa_behavior_yaml: Some(
                        "scaleDown:\n  policies:\n  - type: Pods\n    value: 1\n    periodSeconds: 60\n"
                            .into(),
                    ),
                }),
                status: KedaScaledObjectStatus {
                    last_active_time: Some(1_000_450),
                    original_replica_count: 2,
                    health: KedaHealth {
                        overall: "Healthy".into(),
                        message: "all triggers reporting".into(),
                    },
                    active_triggers: vec!["kafka".into()],
                    reason: "ScalingActive".into(),
                },
            },
            KedaScaledObjectDetail {
                tenant: acme.clone(),
                namespace: "reports".into(),
                name: "report-runner".into(),
                annotations: vec![
                    ("autoscaling.keda.sh/paused".into(), "true".into()),
                ],
                scale_target_ref: KedaScaleTargetRef {
                    api_version: "apps/v1".into(),
                    kind: "Deployment".into(),
                    name: "report-runner".into(),
                    env_source_container_name: None,
                },
                min_replica_count: 0,
                max_replica_count: 10,
                idle_replica_count: None,
                polling_interval_secs: 60,
                cooldown_period_secs: 600,
                initial_cooldown_period_secs: 0,
                fallback: None,
                triggers: vec![KedaTrigger {
                    kind: "cron".into(),
                    name: Some("business-hours".into()),
                    metadata: vec![
                        ("timezone".into(), "Europe/Istanbul".into()),
                        ("start".into(), "0 9 * * 1-5".into()),
                        ("end".into(), "0 18 * * 1-5".into()),
                        ("desiredReplicas".into(), "3".into()),
                    ],
                    auth_ref: None,
                    metric_type: "AverageValue".into(),
                    use_cached_metrics: false,
                }],
                advanced: None,
                status: KedaScaledObjectStatus {
                    last_active_time: None,
                    original_replica_count: 0,
                    health: KedaHealth {
                        overall: "Healthy".into(),
                        message: "paused".into(),
                    },
                    active_triggers: vec![],
                    reason: "Paused".into(),
                },
            },
            KedaScaledObjectDetail {
                tenant: evil.clone(),
                namespace: "default".into(),
                name: "evil-worker".into(),
                annotations: vec![],
                scale_target_ref: KedaScaleTargetRef {
                    api_version: "apps/v1".into(),
                    kind: "Deployment".into(),
                    name: "evil-worker".into(),
                    env_source_container_name: None,
                },
                min_replica_count: 1,
                max_replica_count: 5,
                idle_replica_count: None,
                polling_interval_secs: 15,
                cooldown_period_secs: 60,
                initial_cooldown_period_secs: 0,
                fallback: None,
                triggers: vec![KedaTrigger {
                    kind: "cpu".into(),
                    name: None,
                    metadata: vec![
                        ("type".into(), "Utilization".into()),
                        ("value".into(), "75".into()),
                    ],
                    auth_ref: None,
                    metric_type: "Utilization".into(),
                    use_cached_metrics: false,
                }],
                advanced: None,
                status: KedaScaledObjectStatus {
                    last_active_time: Some(1_000_460),
                    original_replica_count: 1,
                    health: KedaHealth {
                        overall: "Healthy".into(),
                        message: "cpu trigger active".into(),
                    },
                    active_triggers: vec!["cpu".into()],
                    reason: "ScalingActive".into(),
                },
            },
        ]);
    s.keda_scaled_jobs.write().unwrap().extend([
        KedaScaledJob {
            tenant: acme.clone(),
            namespace: "ingest".into(),
            name: "backfill-runner".into(),
            job_template_yaml:
                "spec:\n  template:\n    spec:\n      containers:\n      - name: runner\n        image: ingest:v3\n      restartPolicy: OnFailure\n"
                    .into(),
            polling_interval_secs: 30,
            successful_jobs_history_limit: 100,
            failed_jobs_history_limit: 100,
            max_replica_count: 20,
            scaling_strategy: "default".into(),
            triggers: vec![KedaTrigger {
                kind: "aws-sqs-queue".into(),
                name: Some("backfill-queue".into()),
                metadata: vec![
                    ("queueURL".into(), "https://sqs.eu-west-1.amazonaws.com/000000/backfill".into()),
                    ("queueLength".into(), "10".into()),
                    ("awsRegion".into(), "eu-west-1".into()),
                ],
                auth_ref: Some(KedaAuthRef {
                    name: "aws-irsa".into(),
                    kind: "TriggerAuthentication".into(),
                }),
                metric_type: "AverageValue".into(),
                use_cached_metrics: false,
            }],
            status: KedaScaledJobStatus {
                last_active_time: Some(1_000_500),
                running_jobs: 3,
                pending_jobs: 1,
                succeeded_jobs_24h: 142,
                failed_jobs_24h: 2,
            },
        },
        KedaScaledJob {
            tenant: evil.clone(),
            namespace: "default".into(),
            name: "evil-cron-jobs".into(),
            job_template_yaml: "spec:\n  template:\n    spec:\n      containers:\n      - name: x\n        image: evil:1\n".into(),
            polling_interval_secs: 60,
            successful_jobs_history_limit: 3,
            failed_jobs_history_limit: 3,
            max_replica_count: 2,
            scaling_strategy: "accurate".into(),
            triggers: vec![KedaTrigger {
                kind: "cron".into(),
                name: None,
                metadata: vec![
                    ("timezone".into(), "UTC".into()),
                    ("start".into(), "0 * * * *".into()),
                    ("end".into(), "30 * * * *".into()),
                    ("desiredReplicas".into(), "1".into()),
                ],
                auth_ref: None,
                metric_type: "AverageValue".into(),
                use_cached_metrics: false,
            }],
            status: KedaScaledJobStatus {
                last_active_time: None,
                running_jobs: 0,
                pending_jobs: 0,
                succeeded_jobs_24h: 24,
                failed_jobs_24h: 0,
            },
        },
    ]);
    s.keda_trigger_authentications.write().unwrap().extend([
        KedaTriggerAuthentication {
            tenant: acme.clone(),
            namespace: "ingest".into(),
            name: "kafka-sasl".into(),
            cluster_scoped: false,
            secret_refs: vec![
                KedaSecretRef {
                    parameter: "sasl".into(),
                    secret_name: "kafka-creds".into(),
                    key: "sasl".into(),
                },
                KedaSecretRef {
                    parameter: "username".into(),
                    secret_name: "kafka-creds".into(),
                    key: "username".into(),
                },
                KedaSecretRef {
                    parameter: "password".into(),
                    secret_name: "kafka-creds".into(),
                    key: "password".into(),
                },
            ],
            env_refs: vec![],
            pod_identity_provider: "none".into(),
            hashicorp_vault: None,
            azure_key_vault: None,
        },
        KedaTriggerAuthentication {
            tenant: acme.clone(),
            namespace: "ingest".into(),
            name: "aws-irsa".into(),
            cluster_scoped: false,
            secret_refs: vec![],
            env_refs: vec![KedaEnvRef {
                parameter: "AWS_REGION".into(),
                name: "AWS_REGION".into(),
                container_name: "runner".into(),
            }],
            pod_identity_provider: "aws".into(),
            hashicorp_vault: None,
            azure_key_vault: None,
        },
        KedaTriggerAuthentication {
            tenant: acme.clone(),
            namespace: "reports".into(),
            name: "vault-bound".into(),
            cluster_scoped: false,
            secret_refs: vec![],
            env_refs: vec![],
            pod_identity_provider: "none".into(),
            hashicorp_vault: Some(KedaVaultBinding {
                address: "https://vault.acme.svc:8200".into(),
                authentication: "kubernetes".into(),
                mount: "kubernetes".into(),
                role: "keda-reader".into(),
                credential_secret_name: "vault-bootstrap".into(),
                paths: vec!["secret/data/keda/reports".into()],
            }),
            azure_key_vault: None,
        },
        KedaTriggerAuthentication {
            tenant: evil.clone(),
            namespace: "default".into(),
            name: "evil-azure".into(),
            cluster_scoped: false,
            secret_refs: vec![],
            env_refs: vec![],
            pod_identity_provider: "azure-workload".into(),
            hashicorp_vault: None,
            azure_key_vault: Some(KedaAzureKvBinding {
                vault_uri: "https://evilkv.vault.azure.net".into(),
                tenant_id: "00000000-0000-0000-0000-000000000000".into(),
                client_id: "11111111-1111-1111-1111-111111111111".into(),
                secrets: vec!["queue-conn".into()],
            }),
        },
    ]);
}
