// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 1 — agent executor loop (ports `agent/run_agent.py`).
//!
//! The executor ties the existing planner / tool-registry / memory /
//! recall / session subsystems into a single think→act→observe run loop:
//! plan a goal, execute each step against the tool registry, persist tool
//! outputs to memory + recall, journal every transition to the session
//! event log, and synthesise a final response.

use std::sync::Arc;

use cave_hermes::agent::{AgentExecutor, AgentRun};
use cave_hermes::error::HermesError;
use cave_hermes::planner::LlmPlanner;
use cave_hermes::recall::HashRecall;
use cave_hermes::router::ModelRouter;
use cave_hermes::session::EventKind;
use cave_hermes::{HermesRuntime, InMemoryStore, SessionStore, ToolRegistry, default_runtime, tools_builtin};

/// Build a runtime whose planner returns a fixed JSON plan. Lets tests
/// drive the executor down deterministic tool paths without an LLM.
fn runtime_with_canned_plan(plan_json: &'static str) -> HermesRuntime {
    let mut tools = ToolRegistry::new();
    tools_builtin::register_all(&mut tools);
    HermesRuntime {
        memory: Box::new(InMemoryStore::new()),
        tools,
        planner: Box::new(LlmPlanner::new(Arc::new(move |_g: &str| Ok(plan_json.to_string())))),
        router: ModelRouter::tiered_default(),
        recall: Box::new(HashRecall::new()),
        session: SessionStore::new(),
    }
}

#[test]
fn agent_runs_bash_step_and_journals_events() {
    let mut exec = AgentExecutor::new(default_runtime());
    let run: AgentRun = exec.run("run echo hello-cave").unwrap();

    // HeuristicPlanner emits [bash, respond] for a "run …" goal.
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.steps[0].tool, "bash");
    assert!(run.steps[0].ok, "bash step should succeed: {:?}", run.steps[0]);
    assert!(
        run.final_response.contains("hello-cave"),
        "final response should carry the command output, got {:?}",
        run.final_response
    );

    // Session journal must record the plan, the tool call, the tool result,
    // and the assistant turn.
    let session = exec.session();
    assert_eq!(session.of_kind(EventKind::PlanCreated).len(), 1);
    assert_eq!(session.of_kind(EventKind::ToolCall).len(), 1);
    assert_eq!(session.of_kind(EventKind::ToolResult).len(), 1);
    assert_eq!(session.of_kind(EventKind::AssistantTurn).len(), 1);
}

#[test]
fn agent_respond_only_goal_invokes_no_tools() {
    let mut exec = AgentExecutor::new(default_runtime());
    let run = exec.run("tell me something interesting").unwrap();
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.steps[0].tool, "respond");
    assert_eq!(exec.session().of_kind(EventKind::ToolCall).len(), 0);
}

#[test]
fn agent_persists_tool_output_to_memory_and_recall() {
    let mut exec = AgentExecutor::new(default_runtime()).with_scope("sess-42");
    exec.run("run echo persisted-token").unwrap();

    // The bash output is stored as a memory record in the configured scope…
    let recs = exec.memory().list_scope("sess-42").unwrap();
    assert!(!recs.is_empty(), "expected at least one persisted memory record");
    assert!(
        recs.iter().any(|r| r.body.contains("persisted-token")),
        "tool output should be persisted verbatim"
    );

    // …and indexed for recall.
    let hits = exec.recall_query("persisted-token", 5).unwrap();
    assert!(!hits.is_empty(), "expected recall to surface the persisted output");
}

#[test]
fn agent_tool_error_surfaces_as_agent_failed() {
    // Plan references a tool that isn't registered → invoke returns
    // ToolNotFound → the loop aborts with AgentFailed.
    let rt = runtime_with_canned_plan(
        r#"{"goal":"g","steps":[{"tool":"does_not_exist","rationale":"boom","args":{}}]}"#,
    );
    let mut exec = AgentExecutor::new(rt);
    let err = exec.run("anything").unwrap_err();
    assert!(matches!(err, HermesError::AgentFailed(_)), "got {err:?}");
    // The failure must be journalled as an Error event.
    assert_eq!(exec.session().of_kind(EventKind::Error).len(), 1);
}

#[test]
fn agent_enforces_max_step_guard() {
    // Three respond steps but a max of two → the loop trips the guard.
    let rt = runtime_with_canned_plan(
        r#"{"goal":"g","steps":[
            {"tool":"respond","rationale":"a","args":{}},
            {"tool":"respond","rationale":"b","args":{}},
            {"tool":"respond","rationale":"c","args":{}}
        ]}"#,
    );
    let mut exec = AgentExecutor::new(rt).with_max_steps(2);
    let err = exec.run("anything").unwrap_err();
    assert!(matches!(err, HermesError::AgentFailed(_)), "got {err:?}");
}

#[test]
fn agent_run_is_repeatable_on_same_executor() {
    let mut exec = AgentExecutor::new(default_runtime());
    let a = exec.run("run echo one").unwrap();
    let b = exec.run("run echo two").unwrap();
    assert!(a.final_response.contains("one"));
    assert!(b.final_response.contains("two"));
    // Both runs share the session log: two plans journalled.
    assert_eq!(exec.session().of_kind(EventKind::PlanCreated).len(), 2);
}

#[test]
fn agent_empty_goal_is_rejected_by_planner() {
    let mut exec = AgentExecutor::new(default_runtime());
    let err = exec.run("   ").unwrap_err();
    assert!(matches!(err, HermesError::PlannerRejected(_)), "got {err:?}");
}
