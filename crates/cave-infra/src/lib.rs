<<<<<<< HEAD
//! cave-infra — LLM+MCP-native Infrastructure-as-Code.
//!
//! Replaces: Terraform, Crossplane
//!
//! Instead of HCL files (Terraform) or Kubernetes CRDs (Crossplane), infrastructure
//! is declared as **intent** in natural language or structured YAML. A local LLM
//! (ollama/llama.cpp) interprets the intent and generates an ExecutionPlan. MCP
//! (Model Context Protocol) servers provide the actual cloud-provider integrations
//! for AWS, Azure, GCP, Hetzner, and Kubernetes.
//!
//! # Architecture
//!
//! ```text
//! NL Intent ──▶ intent::parse_intent()
//!                      │
//!                      ▼
//!              planner::generate_plan()   ←─ local LLM in prod
//!                      │
//!                      ▼
//!              executor::execute_plan()
//!                      │
//!              ┌───────┴───────┐
//!              ▼               ▼
//!        mcp_bridge       state::InfraStateStore
//!      (AWS/GCP/…)        (versioned, lockable)
//! ```

pub mod executor;
pub mod intent;
pub mod mcp_bridge;
pub mod models;
pub mod planner;
pub mod routes;
pub mod state;

use axum::Router;
use std::sync::Arc;
use tokio::sync::Mutex;

/// All mutable state for the infra module, shared across request handlers.
pub struct InfraModuleState {
    /// Registered MCP provider servers.
    pub registry: Mutex<mcp_bridge::McpRegistry>,
    /// Persistent infrastructure state (desired + actual).
    pub store: Mutex<state::InfraStateStore>,
    /// Generated and applied execution plans.
    pub plans: Mutex<Vec<models::ExecutionPlan>>,
    /// Submitted intents.
    pub intents: Mutex<Vec<models::InfraIntent>>,
}

impl Default for InfraModuleState {
    fn default() -> Self {
        Self {
            registry: Mutex::new(mcp_bridge::McpRegistry::new()),
            store: Mutex::new(state::InfraStateStore::default()),
            plans: Mutex::new(Vec::new()),
            intents: Mutex::new(Vec::new()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<InfraModuleState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "infra";
=======
//! `cave-infra` — LLM+MCP intent-based infrastructure as code.
//!
//! Replaces Terraform/Crossplane with natural-language infrastructure management
//! backed by an LLM planner and an MCP execution bridge.

pub mod approval;
pub mod graph;
pub mod intent;
pub mod mcp;
pub mod planner;
pub mod providers;
pub mod rollback;
pub mod state;

// ── Convenient re-exports ─────────────────────────────────────────────────────

pub use approval::{ApprovalRequest, ApprovalStatus, ApprovalWorkflow};
pub use graph::{DependencyGraph, GraphError};
pub use intent::{InfraIntent, IntentParser, ParsedIntent, ResourceRequest};
pub use mcp::{McpBridge, McpToolCall, McpToolResult, MockMcpBridge};
pub use planner::{InfraPlan, InfraPlanner, LlmPlanner, MockPlanner, PlanStatus, PlannerError, PlannedResource};
pub use providers::{MockProvider, ProvisionResult, ResourceProvider, ResourceType};
pub use rollback::{RollbackAction, RollbackManager, RollbackPlan, RollbackStatus, RollbackStep};
pub use state::{InfraResource, InfraState, ResourceState, StateManager};
>>>>>>> claude/great-sanderson
