// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-hermes — Hermes-parity agent orchestration runtime.
//!
//! Ports the MVP surface of [NousResearch/hermes-agent] v2026.5.16 to Rust:
//! a persistent memory layer, a self-registering tool registry, a workflow
//! checkpoint/resume machine, an LLM-backed planner, a tiered model router,
//! semantic recall (hash-based fallback), and a session event log.
//!
//! [NousResearch/hermes-agent]: https://github.com/NousResearch/hermes-agent
//!
//! ## Layout
//!
//! | upstream                       | cave-hermes module            |
//! |--------------------------------|-------------------------------|
//! | `agent/memory_manager.py`      | [`memory`]                    |
//! | `agent/memory_provider.py`     | [`memory`]                    |
//! | `tools/registry.py`            | [`tool`]                      |
//! | `providers/base.py`            | [`router`]                    |
//! | `agent/retry_utils.py`         | [`workflow`]                  |
//! | `agent/rate_limit_tracker.py`  | [`router`]                    |
//! | `agent/prompt_builder.py` (∂)  | [`planner`]                   |
//! | `agent/credential_sources.py`  | [`session`]                   |
//!
//! Charter v2 gate: this crate is *backend-only* MVP. Portal admin pages,
//! cavectl subcommands, and observability dashboards are honest-deferred
//! to a follow-up sprint and documented in `PARITY_REPORT.md §7`.

#![forbid(unsafe_code)]

pub mod cli;
pub mod embedding;
pub mod error;
pub mod gateway;
pub mod llm_gateway_adapter;
pub mod memory;
pub mod openjarvis;
pub mod planner;
pub mod prompt;
pub mod provider_tools;
pub mod recall;
pub mod router;
pub mod self_improve;
pub mod session;
pub mod tool;
pub mod tools_builtin;
pub mod workflow;

pub use error::HermesError;
pub use gateway::{
    AnthropicStubGateway, CompletionRequest, CompletionResponse, LlmGateway, OllamaGateway,
};
pub use memory::{FileStore, InMemoryStore, MemoryProvider, MemoryRecord, SqliteStore};
pub use planner::{HeuristicPlanner, Plan, PlanStep, Planner};
pub use prompt::{
    AnthropicPrompt, OllamaPrompt, OpenAiPrompt, OpenRouterPrompt, PromptContext, ProviderKind,
    ProviderPrompt, ToolDescriptor,
};
pub use recall::{Embedder, EmbeddingRecall, HashEmbedder, HashRecall, RecallEngine, RecallHit};
pub use router::{ModelRouter, ModelTier, RouteDecision, TaskComplexity};

// OpenJarvis local-first primitives (ADR-RUNTIME-OPENJARVIS-ADOPTION-001).
pub use openjarvis::agent_state::{AgentState, AgentStateStore};
pub use openjarvis::backend::{Backend, BackendProfile, BackendRegistry, Device, Endpoint};
pub use openjarvis::eval::{EvalBudget, EvalHarness, EvalMetrics, ScoreWeights, rank_backends};
pub use openjarvis::multi_agent::{Message, MessageBus, MessageKind, Orchestrator};
pub use openjarvis::plan_execute::{ExecutionTrace, PlanExecutor, StepOutcome};

// Self-improvement layer (ADR-SELF-IMPROVE-001).
pub use self_improve::observe::{
    Anomaly, LogLevel, ObservationAnalyzer, ObservationWindow, Rule, Severity,
};
pub use self_improve::tune::{
    ApplyMode, ApplyOutcome, ChangeSurface, TuningEngine, TuningSuggestion,
};
pub use self_improve::upstream::{
    BumpKind, ChangelogWatcher, HotPatchQueue, PortProposal, Priority, UpstreamUpdate, Version,
};
pub use session::{Event, EventKind, SessionStore};
pub use tool::{ToolEntry, ToolRegistry, ToolResult};
pub use workflow::{Checkpoint, Workflow, WorkflowStatus};

/// Upstream pin — must match `parity.manifest.toml`.
///
/// Charter v2 always-latest gate: bump whenever the manifest is bumped.
pub const UPSTREAM_VERSION: &str = "v2026.5.16";
pub const UPSTREAM_REPO: &str = "https://github.com/NousResearch/hermes-agent";

/// Convenience constructor wiring together default backends for a brand-new
/// session: an [`InMemoryStore`] memory provider, a [`ToolRegistry`] with the
/// four built-in tools (`bash`, `file_read`, `file_write`, `web_fetch`), a
/// [`HeuristicPlanner`] planner, a tiered [`ModelRouter`], a [`HashRecall`]
/// recall engine, and an empty [`SessionStore`].
pub fn default_runtime() -> HermesRuntime {
    let mut tools = ToolRegistry::new();
    tools_builtin::register_all(&mut tools);
    HermesRuntime {
        memory: Box::new(InMemoryStore::new()),
        tools,
        planner: Box::new(HeuristicPlanner::new()),
        router: ModelRouter::tiered_default(),
        recall: Box::new(HashRecall::new()),
        session: SessionStore::new(),
    }
}

/// Bundle of orchestrator state for a single agent instance.
///
/// The `Box<dyn>` slots are pluggable — swap in an LLM-backed planner or a
/// disk-backed memory provider without touching the public surface.
pub struct HermesRuntime {
    pub memory: Box<dyn MemoryProvider>,
    pub tools: ToolRegistry,
    pub planner: Box<dyn Planner>,
    pub router: ModelRouter,
    pub recall: Box<dyn RecallEngine>,
    pub session: SessionStore,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_wires_four_builtin_tools() {
        let rt = default_runtime();
        let names: Vec<_> = rt.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"bash"), "bash builtin missing");
        assert!(names.contains(&"file_read"), "file_read builtin missing");
        assert!(names.contains(&"file_write"), "file_write builtin missing");
        assert!(names.contains(&"web_fetch"), "web_fetch builtin missing");
    }

    #[test]
    fn upstream_pin_matches_v2026_5_16() {
        assert_eq!(UPSTREAM_VERSION, "v2026.5.16");
    }
}
