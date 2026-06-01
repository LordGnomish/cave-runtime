// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP surface for cave-hermes (Backend track).
//!
//! Exposes the agent loop, multi-agent orchestrator, and tool catalogue over
//! a small Axum router that `cave-runtime` mounts under `/api/hermes`. Each
//! request runs against a *fresh* [`HermesRuntime`] built from the state's
//! [`RuntimeFactory`], so a request never sees another request's memory,
//! recall, or session state — the same isolation the orchestrator gives its
//! workers.
//!
//! Request handling logic lives in plain `do_*` functions so it is unit-
//! testable without spinning up an HTTP server; the async handlers are thin
//! wrappers that deserialize, call the logic, and serialize.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::agent::AgentExecutor;
use crate::orchestrator::{Orchestrator, Subtask};
use crate::{HermesRuntime, default_runtime};

/// Shared state: a factory that mints a fresh runtime per request.
#[derive(Clone)]
pub struct HermesState {
    factory: Arc<dyn Fn() -> HermesRuntime + Send + Sync>,
}

impl HermesState {
    pub fn new(factory: Arc<dyn Fn() -> HermesRuntime + Send + Sync>) -> Self {
        Self { factory }
    }

    fn runtime(&self) -> HermesRuntime {
        (self.factory)()
    }
}

impl Default for HermesState {
    fn default() -> Self {
        Self {
            factory: Arc::new(default_runtime),
        }
    }
}

/// Convenience constructor mirroring the workspace `new_state()` convention.
pub fn new_state() -> Arc<HermesState> {
    Arc::new(HermesState::default())
}

// ── DTOs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
    pub goal: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepDto {
    pub tool: String,
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResponse {
    pub goal: String,
    pub final_response: String,
    pub steps: Vec<StepDto>,
    pub recalled: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubtaskDto {
    pub id: String,
    pub goal: String,
    #[serde(default)]
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrchestrateRequest {
    pub subtasks: Vec<SubtaskDto>,
    #[serde(default)]
    pub pool_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerDto {
    pub subtask_id: String,
    pub worker: usize,
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrchestrateResponse {
    pub completed: usize,
    pub failed: usize,
    pub results: Vec<WorkerDto>,
}

// ── Logic (pure, testable) ─────────────────────────────────────────────────

fn do_tools(_state: &HermesState) -> serde_json::Value {
    // RED placeholder
    serde_json::Value::Array(Vec::new())
}

fn do_run(_state: &HermesState, req: RunRequest) -> crate::error::Result<RunResponse> {
    // RED placeholder
    Ok(RunResponse {
        goal: req.goal,
        final_response: String::new(),
        steps: Vec::new(),
        recalled: Vec::new(),
    })
}

fn do_orchestrate(
    _state: &HermesState,
    _req: OrchestrateRequest,
) -> crate::error::Result<OrchestrateResponse> {
    // RED placeholder
    Ok(OrchestrateResponse {
        completed: 0,
        failed: 0,
        results: Vec::new(),
    })
}

// ── Handlers ───────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "module": "hermes",
        "upstream": crate::UPSTREAM_VERSION,
    }))
}

async fn tools(State(s): State<Arc<HermesState>>) -> Json<serde_json::Value> {
    Json(do_tools(&s))
}

async fn run(
    State(s): State<Arc<HermesState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    match do_run(&s, req) {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn orchestrate(
    State(s): State<Arc<HermesState>>,
    Json(req): Json<OrchestrateRequest>,
) -> impl IntoResponse {
    match do_orchestrate(&s, req) {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Build the Axum router for cave-hermes, mounted under `/api/hermes`.
pub fn router(state: Arc<HermesState>) -> Router {
    Router::new()
        .route("/api/hermes/health", get(health))
        .route("/api/hermes/tools", get(tools))
        .route("/api/hermes/agent/run", post(run))
        .route("/api/hermes/orchestrate", post(orchestrate))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_endpoint_lists_the_four_builtins() {
        let s = HermesState::default();
        let cat = do_tools(&s);
        let arr = cat.as_array().expect("catalogue is an array");
        assert_eq!(arr.len(), 4, "expected 4 built-in tools, got {}", arr.len());
        let names: Vec<&str> = arr.iter().filter_map(|t| t["name"].as_str()).collect();
        for must in ["bash", "file_read", "file_write", "web_fetch"] {
            assert!(names.contains(&must), "missing tool {must} in {names:?}");
        }
    }

    #[test]
    fn run_endpoint_executes_a_goal() {
        let s = HermesState::default();
        let resp = do_run(
            &s,
            RunRequest {
                goal: "run echo hermes-http".into(),
                scope: None,
            },
        )
        .unwrap();
        assert_eq!(resp.steps[0].tool, "bash");
        assert!(
            resp.final_response.contains("hermes-http"),
            "final response should echo the command output: {}",
            resp.final_response
        );
        assert!(resp.steps.iter().any(|s| s.tool == "respond"));
    }

    #[test]
    fn orchestrate_endpoint_runs_a_subtask_graph() {
        let s = HermesState::default();
        let resp = do_orchestrate(
            &s,
            OrchestrateRequest {
                subtasks: vec![
                    SubtaskDto {
                        id: "a".into(),
                        goal: "run echo one".into(),
                        deps: vec![],
                    },
                    SubtaskDto {
                        id: "b".into(),
                        goal: "run echo two".into(),
                        deps: vec!["a".into()],
                    },
                ],
                pool_size: Some(2),
            },
        )
        .unwrap();
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.completed + resp.failed, 2);
        assert!(resp.results.iter().any(|r| r.subtask_id == "b"));
    }
}
