// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 8 (RED→GREEN): batch tool execution.

use cave_tools::batch::{BatchCall, BatchExecutor, BatchMode, CallStatus};
use cave_tools::tool::{FnTool, ToolRegistry, ToolResult};
use cave_tools::ToolError;
use serde_json::json;

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(FnTool::new(
        "echo",
        "echo msg",
        json!({"type": "object", "properties": {"msg": {"type": "string"}}, "required": ["msg"]}),
        |a| Ok(ToolResult::text(a["msg"].as_str().unwrap_or("").to_string())),
    ));
    reg.register(FnTool::new(
        "boom",
        "always errors",
        json!({"type": "object"}),
        |_| Err(ToolError::Execution { tool: "boom".into(), reason: "nope".into() }),
    ));
    reg
}

#[test]
fn independent_calls_all_run() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    let calls = vec![
        BatchCall::new("a", "echo", json!({"msg": "1"})),
        BatchCall::new("b", "echo", json!({"msg": "2"})),
    ];
    let out = ex.run(&calls, BatchMode::ContinueOnError).unwrap();
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|o| o.id == "a").unwrap();
    assert!(matches!(&a.status, CallStatus::Ok(r) if r.text_output() == "1"));
}

#[test]
fn fail_fast_aborts_remaining() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    let calls = vec![
        BatchCall::new("a", "echo", json!({"msg": "ok"})),
        BatchCall::new("b", "boom", json!({})),
        BatchCall::new("c", "echo", json!({"msg": "never"})),
    ];
    let out = ex.run(&calls, BatchMode::FailFast).unwrap();
    let by = |id: &str| out.iter().find(|o| o.id == id).unwrap().status.clone();
    assert!(matches!(by("a"), CallStatus::Ok(_)));
    assert!(matches!(by("b"), CallStatus::Failed(_)));
    assert!(matches!(by("c"), CallStatus::Skipped(_)));
}

#[test]
fn continue_on_error_runs_all_independent_calls() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    let calls = vec![
        BatchCall::new("a", "boom", json!({})),
        BatchCall::new("b", "echo", json!({"msg": "still runs"})),
    ];
    let out = ex.run(&calls, BatchMode::ContinueOnError).unwrap();
    let by = |id: &str| out.iter().find(|o| o.id == id).unwrap().status.clone();
    assert!(matches!(by("a"), CallStatus::Failed(_)));
    assert!(matches!(by("b"), CallStatus::Ok(_)));
}

#[test]
fn dependencies_order_execution() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    // b depends on a; declared out of order — a must still run first.
    let calls = vec![
        BatchCall::new("b", "echo", json!({"msg": "second"})).after(["a"]),
        BatchCall::new("a", "echo", json!({"msg": "first"})),
    ];
    let out = ex.run(&calls, BatchMode::ContinueOnError).unwrap();
    let pos = |id: &str| out.iter().position(|o| o.id == id).unwrap();
    assert!(pos("a") < pos("b"));
}

#[test]
fn dependent_of_failed_call_is_skipped() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    let calls = vec![
        BatchCall::new("a", "boom", json!({})),
        BatchCall::new("b", "echo", json!({"msg": "x"})).after(["a"]),
    ];
    let out = ex.run(&calls, BatchMode::ContinueOnError).unwrap();
    let by = |id: &str| out.iter().find(|o| o.id == id).unwrap().status.clone();
    assert!(matches!(by("a"), CallStatus::Failed(_)));
    match by("b") {
        CallStatus::Skipped(reason) => assert!(reason.contains("a")),
        other => panic!("expected skipped, got {other:?}"),
    }
}

#[test]
fn dependency_cycle_is_rejected() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    let calls = vec![
        BatchCall::new("a", "echo", json!({"msg": "1"})).after(["b"]),
        BatchCall::new("b", "echo", json!({"msg": "2"})).after(["a"]),
    ];
    let err = ex.run(&calls, BatchMode::ContinueOnError).unwrap_err();
    assert_eq!(err.code(), "protocol_error");
}

#[test]
fn unknown_dependency_is_rejected() {
    let reg = registry();
    let ex = BatchExecutor::new(&reg);
    let calls = vec![BatchCall::new("a", "echo", json!({"msg": "1"})).after(["ghost"])];
    let err = ex.run(&calls, BatchMode::ContinueOnError).unwrap_err();
    assert_eq!(err.code(), "protocol_error");
}
