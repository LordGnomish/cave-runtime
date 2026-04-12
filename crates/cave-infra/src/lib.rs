//! CAVE Infra — LLM+MCP native IaC.
//!
//! Replaces: Terraform, Crossplane
//!
//! Infrastructure is declared as natural language or YAML "intent".
//! A local LLM interprets intent into execution plans.
//! MCP servers provide cloud provider integrations.

pub mod executor;
pub mod intent;
pub mod mcp_bridge;
pub mod models;
pub mod planner;
pub mod routes;
pub mod state;

use axum::Router;
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
        }
    }
}

/// Create the Axum router for this module.
pub fn router(state: Arc<InfraState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "infra";
