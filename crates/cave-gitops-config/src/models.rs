<<<<<<< HEAD
//! Data models for cave-gitops-config.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Promise ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromiseStatus {
    Active,
    Deprecated,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageType {
    /// Modify the resource manifest.
    Transform,
    /// Add default configurations.
    Configure,
    /// Write to the state store.
    Deploy,
    /// Check constraints on the spec.
    Validate,
    /// Log a notification.
    Notify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    pub name: String,
    pub description: String,
    pub stage_type: PipelineStageType,
    pub config: serde_json::Value,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationSelector {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Promise {
    pub id: Uuid,
    /// Unique short name (e.g., "postgresql", "redis").
    pub name: String,
    pub version: String,
    pub description: String,
    /// JSON Schema for validating resource requests.
    pub api_schema: serde_json::Value,
    pub pipeline: Vec<PipelineStage>,
    /// Names of other promises this one depends on.
    pub dependencies: Vec<String>,
    pub destination_selectors: Vec<DestinationSelector>,
=======
//! Domain models for the CAVE GitOps Config / Platform API.
//!
//! Core concepts mirror Kratix Promises + Crossplane Compositions:
//! - A `Promise` declares a self-service capability platform teams offer.
//! - A `PromiseRequest` is a developer's claim against a Promise.
//! - A `Composition` describes the ordered set of CAVE module calls needed
//!   to fulfil a Promise.
//! - A `ResourceClaim` tracks what was actually provisioned.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Promise
// ---------------------------------------------------------------------------

/// A platform capability that can be self-serviced by application teams.
///
/// Analogous to a Kratix Promise or an AWS Service Catalog product.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Promise {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// SemVer — bump when the input schema or pipeline changes.
    pub version: String,
    /// Kubernetes-style API group (e.g. "platform.cave.io").
    pub api_group: String,
    /// JSON Schema that validates developer requests for this Promise.
    pub input_schema: serde_json::Value,
    /// Ordered fulfillment pipeline.
    pub pipeline: Vec<CompositionStep>,
>>>>>>> claude/modest-yonath
    pub status: PromiseStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

<<<<<<< HEAD
// ─── Resource Request ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRequestStatus {
    Pending,
    InPipeline,
    Ready,
    Failed,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStageResult {
    pub stage_name: String,
    pub status: StageStatus,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: Uuid,
    pub resource_request_id: Uuid,
    pub promise_name: String,
    pub stages: Vec<PipelineStageResult>,
    pub status: PipelineRunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub id: Uuid,
    pub promise_name: String,
    pub promise_version: String,
    pub namespace: String,
    pub name: String,
    /// Must conform to the promise's api_schema.
    pub spec: serde_json::Value,
    pub requester: Uuid,
    pub status: ResourceRequestStatus,
    pub pipeline_run: Option<PipelineRun>,
    /// Cluster names where the resource was deployed.
    pub destinations: Vec<String>,
=======
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromiseStatus {
    /// Promise is registered and accepting requests.
    Active,
    /// Promise is available but hidden from the self-service catalog.
    Deprecated,
    /// Promise is disabled — new requests are rejected.
    Inactive,
}

// ---------------------------------------------------------------------------
// PromiseRequest
// ---------------------------------------------------------------------------

/// A developer's request for a platform capability.
///
/// Analogous to a Kratix Resource Request or a Crossplane Claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromiseRequest {
    pub id: Uuid,
    pub promise_id: Uuid,
    pub promise_name: String,
    /// The environment this resource should be provisioned in.
    pub environment: String,
    /// Developer-supplied parameters — validated against `Promise.input_schema`.
    pub parameters: serde_json::Value,
    /// ID of the user / service account that raised the request.
    pub requested_by: String,
    pub status: RequestStatus,
    /// Human-readable status message (last error, progress note, …).
    pub message: Option<String>,
    /// IDs of `ResourceClaim`s produced by fulfilling this request.
    pub claim_ids: Vec<Uuid>,
>>>>>>> claude/modest-yonath
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

<<<<<<< HEAD
// ─── State Store ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Unknown,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStoreEntry {
    pub id: Uuid,
    /// e.g. "clusters/prod/postgresql/default/my-db.yaml"
    pub path: String,
    pub cluster: String,
    /// YAML content
    pub content: String,
    pub checksum: String,
    pub promise_name: String,
    pub resource_request_id: Uuid,
    pub last_synced: Option<DateTime<Utc>>,
    pub sync_status: SyncStatus,
}

// ─── Cluster ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterDestination {
    pub name: String,
    pub api_server: String,
    pub labels: HashMap<String, String>,
    pub status: ClusterStatus,
    pub registered_at: DateTime<Utc>,
}

// ─── Request Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePromiseRequest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub api_schema: serde_json::Value,
    pub pipeline: Vec<PipelineStage>,
    pub dependencies: Option<Vec<String>>,
    pub destination_selectors: Option<Vec<DestinationSelector>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateResourceRequestRequest {
    pub promise_name: String,
    pub promise_version: String,
    pub namespace: String,
    pub name: String,
    pub spec: serde_json::Value,
    pub requester: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterClusterRequest {
    pub name: String,
    pub api_server: String,
    pub labels: Option<HashMap<String, String>>,
=======
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Validating,
    Provisioning,
    Ready,
    Failed,
    /// Pipeline was reversed after a failure.
    RolledBack,
    Deleting,
    Deleted,
}

// ---------------------------------------------------------------------------
// Composition + CompositionStep
// ---------------------------------------------------------------------------

/// Describes how a set of CAVE modules are combined to fulfil a Promise.
///
/// Analogous to a Crossplane Composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Composition {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub promise_id: Uuid,
    pub steps: Vec<CompositionStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single module invocation within a Composition pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositionStep {
    /// Display name for this step (used in logs and status messages).
    pub name: String,
    /// CAVE module to call (e.g. "cave-pg", "cave-vault", "cave-dns").
    pub module: String,
    /// Operation within the module (e.g. "create_database", "write_secret").
    pub operation: String,
    /// JSONPath / template expressions that map Promise request parameters
    /// to this module's input.  Key = module param name, Value = source
    /// expression (e.g. "$.parameters.engine_version").
    pub parameter_mapping: serde_json::Value,
    /// Names of steps that must complete before this one runs.
    pub depends_on: Vec<String>,
    /// Whether a failure here should abort and roll back the whole pipeline.
    pub required: bool,
    /// Timeout in seconds for this step (0 = no timeout).
    pub timeout_secs: u64,
}

// ---------------------------------------------------------------------------
// PlatformConfig
// ---------------------------------------------------------------------------

/// Global platform configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub id: Uuid,
    /// Human-readable name for this platform instance.
    pub platform_name: String,
    pub environments: Vec<Environment>,
    /// Default values applied to every PromiseRequest.
    pub global_defaults: serde_json::Value,
    /// Naming pattern applied to every provisioned resource.
    /// Supports template variables: `{env}`, `{promise}`, `{request_id}`.
    pub naming_convention: String,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

/// A deployment environment (dev / staging / production, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub name: String,
    pub description: String,
    pub tier: EnvironmentTier,
    /// Constraints applied to every request in this environment.
    pub constraints: EnvironmentConstraints,
    /// Default parameter overrides for this environment.
    pub defaults: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentTier {
    Development,
    Staging,
    Production,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConstraints {
    /// Maximum hourly cost in USD cents allowed for a single request.
    pub max_cost_cents_per_hour: Option<u64>,
    /// Promises that are blocked in this environment.
    pub blocked_promises: Vec<String>,
    /// If true, all requests require a second approval before provisioning.
    pub require_approval: bool,
}

// ---------------------------------------------------------------------------
// ResourceClaim
// ---------------------------------------------------------------------------

/// Tracks a concrete resource provisioned as part of fulfilling a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceClaim {
    pub id: Uuid,
    pub request_id: Uuid,
    pub promise_id: Uuid,
    /// The CAVE module that owns this resource (e.g. "cave-pg").
    pub module: String,
    /// Module-specific identifier of the resource (e.g. a database name).
    pub resource_id: String,
    /// Module-specific type (e.g. "PostgresDatabase", "VaultSecret").
    pub resource_type: String,
    pub environment: String,
    pub status: ClaimStatus,
    /// Connection details or other outputs produced by provisioning.
    pub outputs: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set when the resource is deleted.
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Provisioning,
    Ready,
    Degraded,
    Deleting,
    Deleted,
    Failed,
}

// ---------------------------------------------------------------------------
// ComplianceCheck
// ---------------------------------------------------------------------------

/// A validation rule that is evaluated before provisioning begins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceCheck {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Promises this check applies to.  Empty = applies to all.
    pub applies_to_promises: Vec<String>,
    /// Environments this check applies to.  Empty = applies to all.
    pub applies_to_environments: Vec<String>,
    pub rule: ComplianceRule,
    pub severity: ComplianceSeverity,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComplianceRule {
    /// The request parameter at `field` must be ≤ `max_value`.
    MaxValue { field: String, max_value: u64 },
    /// The request parameter at `field` must match one of `allowed_values`.
    AllowedValues {
        field: String,
        allowed_values: Vec<String>,
    },
    /// The request must target an environment in `allowed_tiers`.
    EnvironmentTier {
        allowed_tiers: Vec<EnvironmentTier>,
    },
    /// Estimated cost must not exceed `max_cents_per_hour`.
    CostLimit { max_cents_per_hour: u64 },
    /// Custom JSONPath expression that must evaluate to `true`.
    JsonPath { expression: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceSeverity {
    /// Log the violation but allow provisioning.
    Warning,
    /// Block provisioning.
    Error,
}

// ---------------------------------------------------------------------------
// API request/response helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreatePromiseRequest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub api_group: String,
    pub input_schema: serde_json::Value,
    pub pipeline: Vec<CompositionStep>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCapabilityRequest {
    pub promise_name: String,
    pub environment: String,
    pub parameters: serde_json::Value,
    pub requested_by: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCompositionRequest {
    pub name: String,
    pub description: String,
    pub promise_id: Uuid,
    pub steps: Vec<CompositionStep>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEnvironmentRequest {
    pub name: String,
    pub description: String,
    pub tier: EnvironmentTier,
    pub constraints: EnvironmentConstraints,
    pub defaults: serde_json::Value,
>>>>>>> claude/modest-yonath
}
