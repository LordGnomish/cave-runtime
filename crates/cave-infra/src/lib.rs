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
