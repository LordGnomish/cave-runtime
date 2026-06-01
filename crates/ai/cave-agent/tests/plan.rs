// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plan-and-execute loop: heuristic decomposition, the step state machine, and
//! replan-on-failure (retry → skip|abort).

use cave_agent::plan::{Executor, OnExhausted, Planner, ReplanPolicy, Step, StepState};
use cave_agent::tool::{Tool, ToolRegistry};
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn decompose_splits_on_connectors() {
    let plan = Planner::decompose("fetch the data then clean it and then summarise it");
    let descs: Vec<&str> = plan.steps.iter().map(|s| s.description.as_str()).collect();
    assert_eq!(descs, ["fetch the data", "clean it", "summarise it"]);
    assert!(plan.steps.iter().all(|s| s.state == StepState::Pending));
    let ids: Vec<usize> = plan.steps.iter().map(|s| s.id).collect();
    assert_eq!(ids, [0, 1, 2]);
}

#[test]
fn decompose_single_goal_is_one_step() {
    let plan = Planner::decompose("just do the thing");
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.goal, "just do the thing");
}

#[test]
fn execute_all_success_marks_every_step_done() {
    let mut reg = ToolRegistry::new();
    reg.register(Tool::new("noop", "ok", json!({}), |_| Ok(json!("ok"))));
    let mut plan = Planner::decompose("a then b then c");
    for s in &mut plan.steps {
        s.tool = Some("noop".into());
    }
    let out = Executor::run(&mut plan, &reg, &ReplanPolicy::default());
    assert_eq!(out.completed, 3);
    assert_eq!(out.failed, 0);
    assert!(!out.aborted);
    assert!(plan.steps.iter().all(|s| s.state == StepState::Done));
}

#[test]
fn execute_retries_flaky_step_until_success() {
    let counter = Arc::new(AtomicU32::new(0));
    let c2 = counter.clone();
    let mut reg = ToolRegistry::new();
    reg.register(Tool::new("flaky", "fails twice", json!({}), move |_| {
        let n = c2.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(cave_agent::AgentError::ToolFailed {
                tool: "flaky".into(),
                reason: "transient".into(),
            })
        } else {
            Ok(json!("recovered"))
        }
    }));
    let mut plan = Planner::decompose("flaky work");
    plan.steps[0].tool = Some("flaky".into());
    let policy = ReplanPolicy {
        max_retries: 3,
        on_exhausted: OnExhausted::Abort,
    };
    let out = Executor::run(&mut plan, &reg, &policy);
    assert_eq!(out.completed, 1);
    assert_eq!(plan.steps[0].state, StepState::Done);
    assert_eq!(plan.steps[0].attempts, 3); // failed, failed, ok
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[test]
fn execute_skip_policy_continues_past_failure() {
    let mut reg = ToolRegistry::new();
    reg.register(Tool::new("bad", "always fails", json!({}), |_| {
        Err(cave_agent::AgentError::ToolFailed {
            tool: "bad".into(),
            reason: "nope".into(),
        })
    }));
    reg.register(Tool::new("ok", "ok", json!({}), |_| Ok(json!("ok"))));
    let mut plan = Planner::decompose("bad step then good step");
    plan.steps[0].tool = Some("bad".into());
    plan.steps[1].tool = Some("ok".into());
    let policy = ReplanPolicy {
        max_retries: 1,
        on_exhausted: OnExhausted::Skip,
    };
    let out = Executor::run(&mut plan, &reg, &policy);
    assert_eq!(out.skipped, 1);
    assert_eq!(out.completed, 1);
    assert_eq!(plan.steps[0].state, StepState::Failed);
    assert_eq!(plan.steps[1].state, StepState::Done);
}

#[test]
fn execute_abort_policy_halts_and_leaves_rest_pending() {
    let mut reg = ToolRegistry::new();
    reg.register(Tool::new("bad", "always fails", json!({}), |_| {
        Err(cave_agent::AgentError::ToolFailed {
            tool: "bad".into(),
            reason: "nope".into(),
        })
    }));
    reg.register(Tool::new("ok", "ok", json!({}), |_| Ok(json!("ok"))));
    let mut plan = Planner::decompose("bad step then never reached");
    plan.steps[0].tool = Some("bad".into());
    plan.steps[1].tool = Some("ok".into());
    let policy = ReplanPolicy {
        max_retries: 0,
        on_exhausted: OnExhausted::Abort,
    };
    let out = Executor::run(&mut plan, &reg, &policy);
    assert!(out.aborted);
    assert_eq!(plan.steps[0].state, StepState::Failed);
    assert_eq!(plan.steps[1].state, StepState::Pending);
}

#[test]
fn step_without_tool_binding_is_skipped_as_unresolved() {
    let reg = ToolRegistry::new();
    let mut plan = Planner::decompose("unbound");
    assert!(plan.steps[0].tool.is_none());
    let out = Executor::run(&mut plan, &reg, &ReplanPolicy::default());
    // No tool to run -> not a hard failure, counts as skipped.
    assert_eq!(out.skipped, 1);
    assert_eq!(plan.steps[0].state, StepState::Failed);
}

#[test]
fn step_builder_constructs_bound_step() {
    let s = Step::new(7, "label").with_tool("calc", json!({"op": "add"}));
    assert_eq!(s.id, 7);
    assert_eq!(s.tool.as_deref(), Some("calc"));
    assert_eq!(s.state, StepState::Pending);
}
