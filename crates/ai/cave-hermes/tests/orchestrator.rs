// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 2 — multi-agent orchestration ("amele" worker pattern).
//!
//! Ports the coordinator half of Hermes' `acp_adapter/` + `acp_registry/`
//! (Agent Communication Protocol): a coordinator decomposes a goal into
//! subtasks, dispatches each to an isolated worker agent ("amele"), honours
//! declared dependencies via topological ordering, and aggregates the
//! per-worker results into a single report.

use std::sync::Arc;

use cave_hermes::error::HermesError;
use cave_hermes::orchestrator::{Orchestrator, RuntimeFactory, Subtask};
use cave_hermes::planner::LlmPlanner;
use cave_hermes::recall::HashRecall;
use cave_hermes::router::ModelRouter;
use cave_hermes::{HermesRuntime, InMemoryStore, SessionStore, ToolRegistry, default_runtime, tools_builtin};

/// Factory producing fresh default runtimes — one isolated agent per worker.
fn default_factory() -> RuntimeFactory {
    Arc::new(default_runtime)
}

/// Factory whose planner always emits a plan referencing an unregistered
/// tool, so every worker's run aborts with AgentFailed.
fn failing_factory() -> RuntimeFactory {
    Arc::new(|| {
        let mut tools = ToolRegistry::new();
        tools_builtin::register_all(&mut tools);
        HermesRuntime {
            memory: Box::new(InMemoryStore::new()),
            tools,
            planner: Box::new(LlmPlanner::new(Arc::new(|_g: &str| {
                Ok(r#"{"goal":"g","steps":[{"tool":"missing_tool","rationale":"x","args":{}}]}"#
                    .to_string())
            }))),
            router: ModelRouter::tiered_default(),
            recall: Box::new(HashRecall::new()),
            session: SessionStore::new(),
        }
    })
}

#[test]
fn runs_independent_subtasks_round_robin() {
    let orch = Orchestrator::new(default_factory()).with_pool_size(2);
    let report = orch
        .run(vec![
            Subtask::new("t1", "run echo aaa"),
            Subtask::new("t2", "run echo bbb"),
        ])
        .unwrap();

    assert_eq!(report.results.len(), 2);
    assert_eq!(report.completed, 2);
    assert_eq!(report.failed, 0);
    assert!(report.all_ok());
    assert!(report.get("t1").unwrap().output.contains("aaa"));
    assert!(report.get("t2").unwrap().output.contains("bbb"));
    // Round-robin worker assignment across the pool.
    assert_eq!(report.get("t1").unwrap().worker, 0);
    assert_eq!(report.get("t2").unwrap().worker, 1);
}

#[test]
fn orders_subtasks_by_dependency() {
    // Provided out of order: the dependent appears before its prerequisite.
    let orch = Orchestrator::new(default_factory());
    let report = orch
        .run(vec![
            Subtask::new("build", "run echo building").after("fetch"),
            Subtask::new("fetch", "run echo fetching"),
        ])
        .unwrap();
    // Topological order must place `fetch` before `build`.
    assert_eq!(report.results[0].subtask_id, "fetch");
    assert_eq!(report.results[1].subtask_id, "build");
}

#[test]
fn detects_dependency_cycle() {
    let orch = Orchestrator::new(default_factory());
    let err = orch
        .run(vec![
            Subtask::new("a", "run echo a").after("b"),
            Subtask::new("b", "run echo b").after("a"),
        ])
        .unwrap_err();
    assert!(matches!(err, HermesError::Orchestration(_)), "got {err:?}");
}

#[test]
fn rejects_duplicate_subtask_ids() {
    let orch = Orchestrator::new(default_factory());
    let err = orch
        .run(vec![
            Subtask::new("dup", "run echo a"),
            Subtask::new("dup", "run echo b"),
        ])
        .unwrap_err();
    assert!(matches!(err, HermesError::Orchestration(_)), "got {err:?}");
}

#[test]
fn rejects_unknown_dependency() {
    let orch = Orchestrator::new(default_factory());
    let err = orch
        .run(vec![Subtask::new("a", "run echo a").after("ghost")])
        .unwrap_err();
    assert!(matches!(err, HermesError::Orchestration(_)), "got {err:?}");
}

#[test]
fn reports_failed_subtask_without_aborting_siblings() {
    let orch = Orchestrator::new(failing_factory());
    let report = orch
        .run(vec![
            Subtask::new("t1", "anything"),
            Subtask::new("t2", "anything"),
        ])
        .unwrap();
    // Both are independent; both fail, but the run completes.
    assert_eq!(report.results.len(), 2);
    assert_eq!(report.failed, 2);
    assert!(!report.all_ok());
}

#[test]
fn skips_dependents_of_a_failed_subtask() {
    let orch = Orchestrator::new(failing_factory());
    let report = orch
        .run(vec![
            Subtask::new("root", "anything"),
            Subtask::new("leaf", "anything").after("root"),
        ])
        .unwrap();
    assert!(!report.get("root").unwrap().ok);
    let leaf = report.get("leaf").unwrap();
    assert!(!leaf.ok);
    assert!(
        leaf.output.contains("dependency"),
        "skipped dependent should explain the cause, got {:?}",
        leaf.output
    );
}

#[test]
fn fail_fast_stops_after_first_failure() {
    let orch = Orchestrator::new(failing_factory()).fail_fast(true);
    let report = orch
        .run(vec![
            Subtask::new("t1", "anything"),
            Subtask::new("t2", "anything"),
        ])
        .unwrap();
    // Bailed out after t1 failed; t2 never ran.
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].subtask_id, "t1");
}

#[test]
fn run_goal_splits_lines_into_subtasks() {
    let orch = Orchestrator::new(default_factory()).with_pool_size(3);
    let report = orch.run_goal("run echo one\nrun echo two\nrun echo three").unwrap();
    assert_eq!(report.results.len(), 3);
    assert_eq!(report.completed, 3);
    assert!(report.all_ok());
}

#[test]
fn run_goal_rejects_empty_goal() {
    let orch = Orchestrator::new(default_factory());
    let err = orch.run_goal("   \n  \n").unwrap_err();
    assert!(matches!(err, HermesError::Orchestration(_)), "got {err:?}");
}
