//! Domain models for cave-deploy.
//! Data models for cave-deploy — full ArgoCD CRD parity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Sources ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSource {
    pub repo_url: String,
    pub branch: String,
    pub path: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmSource {
    pub repo_url: String,
    pub chart: String,
    pub version: String,
    pub values: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KustomizeSource {
    pub repo_url: String,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApplicationSource {
    Git(GitSource),
    Helm(HelmSource),
    Kustomize(KustomizeSource),
}

// ─── Enums ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicy {
    Manual,
    Automated,
    AutomatedWithPrune,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Progressing,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
// ─── Application ──────────────────────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: Uuid,
    pub name: String,
    /// Namespace this Application lives in (e.g. "argocd").
    pub namespace: String,
    pub spec: ApplicationSpec,
    pub status: ApplicationStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<String>,
    /// Finalizers — e.g. "resources-finalizer.argocd.argoproj.io"
    pub finalizers: Vec<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSpec {
    pub source: ApplicationSource,
    /// Multi-source support (overrides source when present).
    #[serde(default)]
    pub sources: Vec<ApplicationSource>,
    pub destination: ApplicationDestination,
    pub project: String,
    #[serde(default)]
    pub sync_policy: Option<SyncPolicy>,
    #[serde(default)]
    pub ignore_differences: Vec<ResourceIgnoreDifference>,
    #[serde(default)]
    pub info: Vec<AppInfo>,
    pub revision_history_limit: Option<u32>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSource {
    pub path: Option<String>,
    /// Branch name, tag, or commit SHA.
    pub target_revision: Option<String>,
    pub helm: Option<HelmSource>,
    pub kustomize: Option<KustomizeSource>,
    pub directory: Option<DirectorySource>,
    /// Chart name for Helm chart repositories.
    pub chart: Option<String>,
    /// Named reference for multi-source apps.
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
    pub release_name: Option<String>,
    /// Inline values YAML.
    pub values: Option<String>,
    #[serde(default)]
    pub value_files: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<HelmParameter>,
    #[serde(default)]
    pub pass_credentials: bool,
    #[serde(default)]
    pub ignore_missing_value_files: bool,
pub struct HelmParameter {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub force_string: bool,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub common_labels: HashMap<String, String>,
    #[serde(default)]
    pub common_annotations: HashMap<String, String>,
    #[serde(default)]
    pub patches: Vec<serde_json::Value>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySource {
    #[serde(default)]
    pub recurse: bool,
    pub jsonnet: Option<JsonnetSource>,
    pub exclude: Option<String>,
    pub include: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsonnetSource {
    #[serde(default)]
    pub libs: Vec<String>,
    #[serde(default)]
    pub tlas: Vec<JsonnetVar>,
    #[serde(default)]
    pub ext_vars: Vec<JsonnetVar>,
pub struct JsonnetVar {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub code: bool,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApplicationDestination {
    /// Kubernetes API server URL; "https://kubernetes.default.svc" for in-cluster.
    pub server: Option<String>,
    /// Cluster name alias (resolved from cluster secrets).
    pub name: Option<String>,
    pub namespace: String,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncPolicy {
    pub automated: Option<AutomatedSync>,
    #[serde(default)]
    pub sync_options: Vec<String>,
    pub retry: Option<RetryStrategy>,
    pub managed_namespace_metadata: Option<ManagedNamespaceMetadata>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedSync {
    #[serde(default)]
    pub prune: bool,
    #[serde(default)]
    pub self_heal: bool,
    #[serde(default)]
    pub allow_empty: bool,
pub struct RetryStrategy {
    /// Max retries; -1 = infinite.
    pub limit: i32,
    pub backoff: Option<BackoffPolicy>,
#[serde(rename_all = "camelCase")]
pub struct BackoffPolicy {
    /// Initial backoff duration string, e.g. "5s".
    pub duration: String,
    pub factor: Option<f64>,
    pub max_duration: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManagedNamespaceMetadata {
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceIgnoreDifference {
    pub group: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub namespace: Option<String>,
    #[serde(default)]
    pub json_pointers: Vec<String>,
    #[serde(default)]
    pub jq_path_expressions: Vec<String>,
    #[serde(default)]
    pub managed_fields_managers: Vec<String>,
pub struct AppInfo {
    pub name: String,
    pub value: String,
// ─── Application Status ───────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationStatus {
    #[serde(default)]
    pub resources: Vec<ResourceStatus>,
    pub sync: SyncStatusDetail,
    pub health: HealthStatusDetail,
    #[serde(default)]
    pub history: Vec<RevisionHistory>,
    #[serde(default)]
    pub conditions: Vec<ApplicationCondition>,
    pub observed_at: Option<DateTime<Utc>>,
    pub operation_state: Option<OperationState>,
    pub source_type: Option<String>,
    pub summary: Option<ApplicationSummary>,
    #[serde(default)]
    pub retry_count: i32,
    pub reconciled_at: Option<DateTime<Utc>>,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Progressing,
    Degraded,
    Suspended,
    Missing,
    Unknown,
}

// ─── Application ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub source: ApplicationSource,
    pub target_cluster: String,
    pub sync_policy: SyncPolicy,
    pub sync_status: SyncStatus,
    pub health_status: HealthStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub revision: Option<String>,
    pub message: Option<String>,
}

// ─── Rollout ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStrategy {
    Canary,
    BlueGreen,
    Rolling,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutStep {
    pub step_index: usize,
    pub weight: u8,
    pub pause_duration_secs: Option<u64>,
    pub analysis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStatus {
    Pending,
    Progressing,
    Paused,
    Promoting,
    Aborting,
    Completed,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    pub id: Uuid,
    pub application_id: Uuid,
    pub strategy: RolloutStrategy,
    pub status: RolloutStatus,
    pub current_step: usize,
    pub steps: Vec<RolloutStep>,
    pub stable_revision: String,
    pub canary_revision: String,
    pub traffic_weight: u8,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error: Option<String>,
}

// ─── Deployment / History ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: Uuid,
    pub application_id: Uuid,
    pub revision: String,
    pub sync_status: SyncStatus,
    pub health_status: HealthStatus,
    pub deployed_at: DateTime<Utc>,
    pub deployed_by: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentHistory {
    pub application_id: Uuid,
    pub entries: Vec<Deployment>,
}

// ─── Resource / Diff ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatus {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub health: HealthStatus,
    pub sync_status: SyncStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDiff {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub application_id: Uuid,
    pub has_diff: bool,
    pub resources: Vec<ResourceDiff>,
    pub generated_at: DateTime<Utc>,
}

// ─── In-memory store ─────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DeployStore {
    pub applications: HashMap<Uuid, Application>,
    pub rollouts: HashMap<Uuid, Rollout>,
    pub history: HashMap<Uuid, Vec<Deployment>>,
}

// ─── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateApplicationRequest {
    pub name: String,
    pub namespace: String,
    pub source: ApplicationSource,
    pub target_cluster: String,
    pub sync_policy: Option<SyncPolicy>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApplicationRequest {
    pub name: Option<String>,
    pub source: Option<ApplicationSource>,
    pub sync_policy: Option<SyncPolicy>,
    pub target_cluster: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    pub revision: Option<String>,
    pub force: Option<bool>,
    #[allow(dead_code)]
    pub prune: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RollbackRequest {
    pub deployment_id: Option<Uuid>,
    pub revision: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RolloutStepRequest {
    pub weight: u8,
    pub pause_duration_secs: Option<u64>,
    pub analysis: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRolloutRequest {
    pub application_id: Uuid,
    pub strategy: RolloutStrategy,
    pub canary_revision: String,
    pub steps: Vec<RolloutStepRequest>,
impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            HealthStatus::Healthy => "Healthy",
            HealthStatus::Progressing => "Progressing",
            HealthStatus::Degraded => "Degraded",
            HealthStatus::Suspended => "Suspended",
            HealthStatus::Missing => "Missing",
            HealthStatus::Unknown => "Unknown",
        };
        write!(f, "{s}")
impl std::str::FromStr for HealthStatus {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Healthy" => HealthStatus::Healthy,
            "Progressing" => HealthStatus::Progressing,
            "Degraded" => HealthStatus::Degraded,
            "Suspended" => HealthStatus::Suspended,
            "Missing" => HealthStatus::Missing,
            _ => HealthStatus::Unknown,
        })
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Unknown,
impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SyncStatus::Synced => "Synced",
            SyncStatus::OutOfSync => "OutOfSync",
            SyncStatus::Unknown => "Unknown",
        };
        write!(f, "{s}")
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusDetail {
    pub status: String,
    pub compared_to: Option<ComparedTo>,
    #[serde(default)]
    pub revisions: Vec<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComparedTo {
    #[serde(default)]
    pub sources: Vec<ApplicationSource>,
    pub destination: ApplicationDestination,
    #[serde(default)]
    pub ignore_differences: Vec<ResourceIgnoreDifference>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthStatusDetail {
    pub status: String,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
    pub group: Option<String>,
    pub version: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub health: Option<HealthStatusDetail>,
    #[serde(default)]
    pub hook: bool,
    #[serde(default)]
    pub require_pruning: bool,
    #[serde(default)]
    pub sync_wave: i32,
#[serde(rename_all = "camelCase")]
pub struct RevisionHistory {
    pub id: u64,
    #[serde(default)]
    pub sources: Vec<ApplicationSource>,
    pub deploy_started_at: Option<DateTime<Utc>>,
    pub initiator: Option<String>,
pub struct ApplicationCondition {
    pub r#type: String,
    pub last_transition_time: Option<DateTime<Utc>>,
#[serde(rename_all = "camelCase")]
pub struct OperationState {
    pub phase: OperationPhase,
    pub sync_result: Option<SyncOperationResult>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub retry_count: i32,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationPhase {
    Running,
    Failed,
    Error,
    Succeeded,
    Terminating,
impl std::fmt::Display for OperationPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OperationPhase::Running => "Running",
            OperationPhase::Failed => "Failed",
            OperationPhase::Error => "Error",
            OperationPhase::Succeeded => "Succeeded",
            OperationPhase::Terminating => "Terminating",
        };
        write!(f, "{s}")
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncOperationResult {
    #[serde(default)]
    pub revisions: Vec<String>,
    #[serde(default)]
    pub resources: Vec<ResourceResult>,
#[serde(rename_all = "camelCase")]
pub struct ResourceResult {
    pub group: Option<String>,
    pub version: String,
    pub namespace: Option<String>,
    pub status: ResourceSyncStatus,
    pub hook_phase: Option<HookPhase>,
    pub hook_type: Option<SyncHookType>,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResourceSyncStatus {
    Synced,
    SyncFailed,
    Pruned,
    PruneSkipped,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SyncHookType {
    PreSync,
    Sync,
    PostSync,
    SyncFail,
    Skip,
impl std::fmt::Display for SyncHookType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SyncHookType::PreSync => "PreSync",
            SyncHookType::Sync => "Sync",
            SyncHookType::PostSync => "PostSync",
            SyncHookType::SyncFail => "SyncFail",
            SyncHookType::Skip => "Skip",
        };
        write!(f, "{s}")
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HookPhase {
    Running,
    Failed,
    Error,
    Succeeded,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApplicationSummary {
    #[serde(default)]
    pub external_urls: Vec<String>,
    #[serde(default)]
    pub images: Vec<String>,
// ─── AppProject ───────────────────────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
pub struct AppProject {
    pub description: Option<String>,
    #[serde(default)]
    pub source_repos: Vec<String>,
    #[serde(default)]
    pub destinations: Vec<ApplicationDestination>,
    #[serde(default)]
    pub cluster_resource_whitelist: Vec<GroupKind>,
    #[serde(default)]
    pub cluster_resource_blacklist: Vec<GroupKind>,
    #[serde(default)]
    pub namespace_resource_whitelist: Vec<GroupKind>,
    #[serde(default)]
    pub namespace_resource_blacklist: Vec<GroupKind>,
    #[serde(default)]
    pub roles: Vec<ProjectRole>,
    #[serde(default)]
    pub sync_windows: Vec<SyncWindow>,
    pub orphaned_resources: Option<OrphanedResourcesMonitor>,
pub struct GroupKind {
    pub group: String,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectRole {
    pub description: Option<String>,
    /// Casbin-style policies: "p, role:name, resource, action, allow"
    #[serde(default)]
    pub policies: Vec<String>,
    /// SSO group names mapped to this role.
    #[serde(default)]
    pub groups: Vec<String>,
pub struct SyncWindow {
    pub kind: SyncWindowKind,
    /// Cron expression.
    pub schedule: String,
    /// Duration string, e.g. "1h".
    pub duration: String,
    #[serde(default)]
    pub applications: Vec<String>,
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub clusters: Vec<String>,
    #[serde(default)]
    pub manual_sync: bool,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncWindowKind {
    Allow,
    Deny,
pub struct OrphanedResourcesMonitor {
    #[serde(default)]
    pub warn: bool,
    #[serde(default)]
    pub ignore: Vec<OrphanedResourceKey>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrphanedResourceKey {
    pub group: Option<String>,
    pub kind: Option<String>,
// ─── Cluster ──────────────────────────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub server: String,
    pub config: ClusterConfig,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    pub info: ClusterInfo,
    pub project: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClusterConfig {
    pub username: Option<String>,
    pub password: Option<String>,
    pub bearer_token: Option<String>,
    pub tls_client_config: Option<TlsClientConfig>,
    pub aws_auth_config: Option<AwsAuthConfig>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TlsClientConfig {
    #[serde(default)]
    pub insecure: bool,
    pub server_name: Option<String>,
    pub cert_data: Option<String>,
    pub key_data: Option<String>,
    pub ca_data: Option<String>,
#[serde(rename_all = "camelCase")]
pub struct AwsAuthConfig {
    pub cluster_name: String,
    pub role_arn: Option<String>,
    pub profile: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClusterInfo {
    pub connection_state: Option<ConnectionState>,
    pub server_version: Option<String>,
    #[serde(default)]
    pub applications_count: u32,
    #[serde(default)]
    pub api_versions: Vec<String>,
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub status: ConnectionStatus,
    pub attempted_at: Option<DateTime<Utc>>,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionStatus {
    Unknown,
    Failed,
    Successful,
// ─── Repository ───────────────────────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
pub struct Repository {
    /// Repository URL (HTTP/S or SSH).
    pub repo: String,
    pub username: Option<String>,
    /// Stored encrypted at rest.
    pub password: Option<String>,
    pub ssh_private_key: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub enable_lfs: bool,
    pub tls_client_cert_data: Option<String>,
    pub tls_client_cert_key: Option<String>,
    pub repo_type: RepoType,
    pub project: Option<String>,
    pub connection_state: Option<ConnectionState>,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepoType {
    Git,
    Helm,
// ─── ApplicationSet ───────────────────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
pub struct ApplicationSet {
    pub spec: ApplicationSetSpec,
    pub status: ApplicationSetStatus,
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetSpec {
    pub generators: Vec<ApplicationSetGenerator>,
    pub template: ApplicationSetTemplate,
    #[serde(default)]
    pub sync_policy: Option<ApplicationSetSyncPolicy>,
    pub strategy: Option<ApplicationSetStrategy>,
    #[serde(default)]
    pub preserve_resources_on_deletion: bool,
    #[serde(default)]
    pub go_template: bool,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetTemplate {
    pub metadata: ApplicationSetTemplateMetadata,
    pub spec: ApplicationSpec,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApplicationSetTemplateMetadata {
    pub namespace: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    #[serde(default)]
    pub finalizers: Vec<String>,
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetSyncPolicy {
    #[serde(default)]
    pub preserve_resources_on_deletion: bool,
    pub applications_sync: Option<String>,
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetStrategy {
    pub r#type: String,
    pub rolling_sync: Option<RollingSync>,
pub struct RollingSync {
    pub steps: Vec<RollingSyncStep>,
#[serde(rename_all = "camelCase")]
pub struct RollingSyncStep {
    #[serde(default)]
    pub match_expressions: Vec<LabelMatchExpression>,
    pub max_update: Option<String>,
pub struct LabelMatchExpression {
    pub key: String,
    pub operator: String,
    #[serde(default)]
    pub values: Vec<String>,
#[serde(tag = "type")]
pub enum ApplicationSetGenerator {
    List(ListGenerator),
    Clusters(ClusterGenerator),
    Git(GitGenerator),
    Matrix(MatrixGenerator),
    Merge(MergeGenerator),
    PullRequest(PullRequestGenerator),
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListGenerator {
    pub elements: Vec<serde_json::Value>,
    pub template: Option<ApplicationSetTemplate>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterGenerator {
    pub selector: Option<LabelSelector>,
    #[serde(default)]
    pub values: HashMap<String, String>,
    pub template: Option<ApplicationSetTemplate>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelector {
    #[serde(default)]
    pub match_labels: HashMap<String, String>,
    #[serde(default)]
    pub match_expressions: Vec<LabelMatchExpression>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitGenerator {
    pub repo_url: String,
    #[serde(default)]
    pub directories: Vec<GitDirectoryGeneratorItem>,
    #[serde(default)]
    pub files: Vec<GitFileGeneratorItem>,
    #[serde(default)]
    pub values: HashMap<String, String>,
    pub template: Option<ApplicationSetTemplate>,
pub struct GitDirectoryGeneratorItem {
    pub path: String,
    #[serde(default)]
    pub exclude: bool,
pub struct GitFileGeneratorItem {
    pub path: String,
pub struct MatrixGenerator {
    /// Exactly 2 generators whose parameter sets are combined.
    pub generators: Vec<ApplicationSetGenerator>,
    pub template: Option<ApplicationSetTemplate>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MergeGenerator {
    pub merge_keys: Vec<String>,
    pub generators: Vec<ApplicationSetGenerator>,
    pub template: Option<ApplicationSetTemplate>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestGenerator {
    pub github: Option<GitHubPullRequestSource>,
    pub gitlab: Option<GitLabPullRequestSource>,
    #[serde(default)]
    pub filters: Vec<PullRequestFilter>,
    pub requeue_after_seconds: Option<u64>,
    pub template: Option<ApplicationSetTemplate>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPullRequestSource {
    pub owner: String,
    pub repo: String,
    pub api: Option<String>,
    pub token_ref: Option<SecretKeyRef>,
    #[serde(default)]
    pub labels: Vec<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitLabPullRequestSource {
    pub project: String,
    pub api: Option<String>,
    pub token_ref: Option<SecretKeyRef>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub pull_request_state: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PullRequestFilter {
    pub branch_match: Option<String>,
    pub target_branch_match: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub secret_name: String,
    pub key: String,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApplicationSetStatus {
    #[serde(default)]
    pub conditions: Vec<ApplicationSetCondition>,
    #[serde(default)]
    pub application_status: Vec<AppSetApplicationStatus>,
pub struct ApplicationSetCondition {
    pub r#type: String,
    pub reason: Option<String>,
    pub status: String,
    pub last_transition_time: Option<DateTime<Utc>>,
pub struct AppSetApplicationStatus {
    pub application: String,
    pub last_transition_time: Option<DateTime<Utc>>,
    pub status: String,
    pub step: Option<String>,
// ─── Diff ─────────────────────────────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
    pub group: Option<String>,
    pub version: String,
    pub namespace: Option<String>,
    pub diff_type: DiffType,
    /// What the desired state looks like (from Git).
    pub desired: Option<serde_json::Value>,
    /// What is live in the cluster right now.
    pub live: Option<serde_json::Value>,
    /// Human-readable unified-diff patch.
    pub patch: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiffType {
    Added,
    Removed,
    Modified,
    Unchanged,
// ─── Manifest (in-memory parsed form) ─────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct Manifest {
    pub api_version: String,
    pub namespace: Option<String>,
    pub raw: serde_json::Value,
    /// argocd.argoproj.io/sync-wave
    pub sync_wave: i32,
    /// argocd.argoproj.io/hook
    pub hook_type: Option<SyncHookType>,
    /// argocd.argoproj.io/hook-delete-policy
    pub hook_delete_policy: Option<String>,
// ─── API Request / Response types ────────────────────────────────────────────
#[serde(rename_all = "camelCase")]
    pub namespace: Option<String>,
    pub project: String,
    pub spec: ApplicationSpec,
    pub spec: ApplicationSpec,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub prune: bool,
    pub strategy: Option<SyncStrategySpec>,
    pub resources: Option<Vec<SyncOperationResource>>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncStrategySpec {
    #[serde(default)]
    pub apply_force: bool,
pub struct SyncOperationResource {
    pub group: Option<String>,
    pub namespace: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
    /// ID from revision history.
    pub id: u64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub prune: bool,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceTrackingConfig {
    pub method: ResourceTrackingMethod,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ResourceTrackingMethod {
    #[default]
    Label,
    Annotation,
    AnnotationAndLabel,
}
