//! Domain models for cave-infra.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Intent ────────────────────────────────────────────────────────────────────

/// Natural language or structured YAML intent describing desired infrastructure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraIntent {
    pub id: Uuid,
    /// Human-readable name for this intent.
    pub name: String,
    /// Raw natural language description (e.g. "Create an S3 bucket for user uploads").
    pub natural_language: Option<String>,
    /// Structured YAML/JSON intent (provider-agnostic).
    pub structured: Option<serde_json::Value>,
    /// Target environment (dev/staging/prod).
    pub environment: String,
    /// Cloud provider hint (aws/gcp/azure/k8s).
    pub provider_hint: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl InfraIntent {
    pub fn new(name: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            natural_language: None,
            structured: None,
            environment: environment.into(),
            provider_hint: None,
            created_at: Utc::now(),
        }
    }
}

// ── Resource ──────────────────────────────────────────────────────────────────

/// Lifecycle state of a managed resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    /// Resource exists in desired state.
    Synced,
    /// Resource needs to be created.
    Pending,
    /// Resource configuration has drifted from desired.
    Drifted,
    /// Resource is being created/updated/deleted.
    InProgress,
    /// Resource creation/update failed.
    Failed,
    /// Resource has been deleted.
    Deleted,
}

/// A single managed infrastructure resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraResource {
    pub id: Uuid,
    /// Logical name (e.g. "user-uploads-bucket").
    pub name: String,
    /// Cloud provider (aws/gcp/azure/k8s).
    pub provider: String,
    /// Resource type (e.g. "s3_bucket", "gke_cluster").
    pub resource_type: String,
    /// Provider-specific configuration.
    pub config: HashMap<String, serde_json::Value>,
    /// Current lifecycle state.
    pub state: ResourceState,
    /// IDs of resources this one depends on.
    pub dependencies: Vec<Uuid>,
    /// Remote resource identifier (e.g. ARN, resource ID).
    pub remote_id: Option<String>,
    /// Last known remote state.
    pub remote_state: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl InfraResource {
    pub fn new(
        name: impl Into<String>,
        provider: impl Into<String>,
        resource_type: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            provider: provider.into(),
            resource_type: resource_type.into(),
            config: HashMap::new(),
            state: ResourceState::Pending,
            dependencies: Vec::new(),
            remote_id: None,
            remote_state: None,
            created_at: now,
            updated_at: now,
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

/// Snapshot of current vs desired infrastructure state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraState {
    pub id: Uuid,
    /// All managed resources (desired state).
    pub desired: Vec<InfraResource>,
    /// Last observed actual state from providers.
    pub actual: Vec<InfraResource>,
    /// Whether state is locked for concurrent writes.
    pub locked: bool,
    /// Who holds the lock.
    pub lock_holder: Option<String>,
    pub version: u64,
    pub last_synced: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Default for InfraState {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            desired: Vec::new(),
            actual: Vec::new(),
            locked: false,
            lock_holder: None,
            version: 0,
            last_synced: None,
            created_at: Utc::now(),
        }
    }
}

// ── Execution Plan ────────────────────────────────────────────────────────────

/// Operation type for a plan step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOperation {
    Create,
    Update,
    Delete,
    NoOp,
}

/// A single step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: Uuid,
    pub operation: StepOperation,
    pub resource_name: String,
    pub provider: String,
    pub resource_type: String,
    /// MCP tool to invoke (e.g. "aws_s3_create_bucket").
    pub mcp_tool: String,
    /// Parameters to pass to the MCP tool.
    pub provider_params: HashMap<String, serde_json::Value>,
    /// Step IDs this step must complete before running.
    pub depends_on: Vec<Uuid>,
    /// Whether this step can run in parallel with siblings.
    pub parallelizable: bool,
    /// Human-readable description of what this step does.
    pub description: String,
}

impl PlanStep {
    pub fn new(
        operation: StepOperation,
        resource_name: impl Into<String>,
        provider: impl Into<String>,
        resource_type: impl Into<String>,
        mcp_tool: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            operation,
            resource_name: resource_name.into(),
            provider: provider.into(),
            resource_type: resource_type.into(),
            mcp_tool: mcp_tool.into(),
            provider_params: HashMap::new(),
            depends_on: Vec::new(),
            parallelizable: false,
            description: description.into(),
        }
    }
}

/// Full execution plan produced by the LLM planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub steps: Vec<PlanStep>,
    /// Ordered rollback steps (reverse of apply).
    pub rollback_steps: Vec<PlanStep>,
    /// Estimated cost breakdown.
    pub cost_estimate: Option<CostEstimate>,
    /// Risk score 0.0–1.0 (blast radius).
    pub risk_score: f64,
    /// LLM-generated plain-language explanation.
    pub explanation: String,
    pub created_at: DateTime<Utc>,
}

impl ExecutionPlan {
    pub fn new(intent_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            steps: Vec::new(),
            rollback_steps: Vec::new(),
            cost_estimate: None,
            risk_score: 0.0,
            explanation: String::new(),
            created_at: Utc::now(),
        }
    }
}

// ── MCP Provider ──────────────────────────────────────────────────────────────

/// An MCP server registered as a cloud provider integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpProvider {
    pub id: Uuid,
    pub name: String,
    /// Cloud provider this MCP server targets.
    pub provider: String,
    /// MCP server endpoint URL.
    pub endpoint: String,
    /// Supported resource types.
    pub capabilities: Vec<String>,
    /// Available tool names.
    pub tools: Vec<String>,
    pub healthy: bool,
    pub last_health_check: Option<DateTime<Utc>>,
    pub registered_at: DateTime<Utc>,
}

impl McpProvider {
    pub fn new(
        name: impl Into<String>,
        provider: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            provider: provider.into(),
            endpoint: endpoint.into(),
            capabilities: Vec::new(),
            tools: Vec::new(),
            healthy: false,
            last_health_check: None,
            registered_at: Utc::now(),
        }
    }
}

// ── Drift ─────────────────────────────────────────────────────────────────────

/// Describes divergence between desired and actual state for one resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftItem {
    pub resource_id: Uuid,
    pub resource_name: String,
    pub provider: String,
    pub resource_type: String,
    /// Fields that differ between desired and actual.
    pub drifted_fields: Vec<String>,
    pub desired: serde_json::Value,
    pub actual: serde_json::Value,
}

/// Full drift report across all managed resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub id: Uuid,
    pub drifted: Vec<DriftItem>,
    /// Resources in desired state but not found remotely.
    pub missing: Vec<String>,
    /// Resources found remotely but not in desired state (orphans).
    pub orphaned: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

impl DriftReport {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            drifted: Vec::new(),
            missing: Vec::new(),
            orphaned: Vec::new(),
            generated_at: Utc::now(),
        }
    }
}

impl Default for DriftReport {
    fn default() -> Self {
        Self::new()
    }
}

// ── Policy ────────────────────────────────────────────────────────────────────

/// Result of evaluating a policy rule against a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCheck {
    pub policy_name: String,
    pub passed: bool,
    pub violations: Vec<String>,
    pub severity: PolicySeverity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySeverity {
    Info,
    Warning,
    Error,
    Critical,
}

// ── Cost ──────────────────────────────────────────────────────────────────────

/// Cost estimate for a plan or resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    /// Estimated monthly cost in USD.
    pub monthly_usd: f64,
    /// Cost breakdown per resource.
    pub breakdown: HashMap<String, f64>,
    /// Confidence level (0.0–1.0).
    pub confidence: f64,
    pub currency: String,
    pub notes: Vec<String>,
}

impl CostEstimate {
    pub fn zero() -> Self {
        Self {
            monthly_usd: 0.0,
            breakdown: HashMap::new(),
            confidence: 1.0,
            currency: "USD".into(),
            notes: Vec::new(),
        }
    }
}
