<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/interesting-khorana
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
=======
//! CAVE Infra — LLM+MCP native IaC.
//!
//! Replaces: Terraform, Crossplane
//!
//! Infrastructure is declared as natural language or YAML "intent".
//! A local LLM interprets intent into execution plans.
//! MCP servers provide cloud provider integrations.
>>>>>>> claude/silly-matsumoto

pub mod executor;
pub mod intent;
pub mod mcp_bridge;
pub mod models;
pub mod planner;
pub mod routes;
pub mod state;

use axum::Router;
<<<<<<< HEAD
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
=======
use mcp_bridge::McpRegistry;
use models::{ExecutionPlan, InfraIntent};
use state::InfraStateStore;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Shared module state — all fields wrapped for concurrent access.
pub struct InfraState {
    /// Registered MCP cloud provider integrations.
    pub registry: Arc<Mutex<McpRegistry>>,
    /// In-memory infrastructure state store.
    pub store: Arc<Mutex<InfraStateStore>>,
    /// Submitted intents awaiting planning.
    pub intents: Arc<Mutex<Vec<InfraIntent>>>,
    /// Generated execution plans.
    pub plans: Arc<Mutex<Vec<ExecutionPlan>>>,
}

impl Default for InfraState {
    fn default() -> Self {
        Self {
            registry: Arc::new(Mutex::new(McpRegistry::new())),
            store: Arc::new(Mutex::new(InfraStateStore::new())),
            intents: Arc::new(Mutex::new(Vec::new())),
            plans: Arc::new(Mutex::new(Vec::new())),
>>>>>>> claude/silly-matsumoto
        }
    }
}

<<<<<<< HEAD
/// Create the axum router for this module.
pub fn router(state: Arc<InfraModuleState>) -> Router {
=======
/// Create the Axum router for this module.
pub fn router(state: Arc<InfraState>) -> Router {
>>>>>>> claude/silly-matsumoto
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "infra";
<<<<<<< HEAD
<<<<<<< HEAD
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
=======
>>>>>>> claude/interesting-khorana
=======
>>>>>>> claude/silly-matsumoto
