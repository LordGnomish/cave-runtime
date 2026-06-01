// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composable execution patterns over the tool registry:
//! Tool / Chain / Parallel / Fallback / Retry, and their nesting.

use cave_agent::compose::{run, Pattern};
use cave_agent::tool::{Tool, ToolRegistry};
use cave_agent::AgentError;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

fn reg() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Tool::new("up", "uppercase", json!({"required":["s"]}), |a| {
        Ok(json!({ "result": a["s"].as_str().unwrap_or("").to_uppercase() }))
    }));
    r.register(Tool::new("echo_prev", "echo _prev", json!({}), |a| {
        Ok(json!({ "echoed": a.get("_prev").cloned().unwrap_or(json!(null)) }))
    }));
    r.register(Tool::new("ok1", "ok", json!({}), |_| Ok(json!(1))));
    r.register(Tool::new("ok2", "ok", json!({}), |_| Ok(json!(2))));
    r.register(Tool::new("bad", "fail", json!({}), |_| {
        Err(AgentError::ToolFailed { tool: "bad".into(), reason: "x".into() })
    }));
    r
}

#[test]
fn single_tool_pattern_runs() {
    let out = run(&Pattern::tool("up", json!({"s": "hi"})), &reg()).unwrap();
    assert_eq!(out["result"], "HI");
}

#[test]
fn chain_injects_previous_output_as_prev() {
    let p = Pattern::Chain(vec![
        Pattern::tool("up", json!({"s": "hello"})),
        Pattern::tool("echo_prev", json!({})),
    ]);
    let out = run(&p, &reg()).unwrap();
    assert_eq!(out["echoed"]["result"], "HELLO");
}

#[test]
fn parallel_collects_outputs_in_order() {
    let p = Pattern::Parallel(vec![Pattern::tool("ok1", json!({})), Pattern::tool("ok2", json!({}))]);
    let out = run(&p, &reg()).unwrap();
    assert_eq!(out, json!([1, 2]));
}

#[test]
fn parallel_propagates_first_error() {
    let p = Pattern::Parallel(vec![Pattern::tool("ok1", json!({})), Pattern::tool("bad", json!({}))]);
    assert!(run(&p, &reg()).is_err());
}

#[test]
fn fallback_returns_first_success() {
    let p = Pattern::Fallback(vec![Pattern::tool("bad", json!({})), Pattern::tool("ok2", json!({}))]);
    assert_eq!(run(&p, &reg()).unwrap(), json!(2));
}

#[test]
fn fallback_all_failing_errors() {
    let p = Pattern::Fallback(vec![Pattern::tool("bad", json!({})), Pattern::tool("bad", json!({}))]);
    assert!(run(&p, &reg()).is_err());
}

#[test]
fn retry_succeeds_on_third_attempt() {
    let counter = Arc::new(AtomicU32::new(0));
    let c2 = counter.clone();
    let mut r = reg();
    r.register(Tool::new("flaky", "x", json!({}), move |_| {
        let n = c2.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(AgentError::ToolFailed { tool: "flaky".into(), reason: "t".into() })
        } else {
            Ok(json!("won"))
        }
    }));
    let p = Pattern::Retry { inner: Box::new(Pattern::tool("flaky", json!({}))), max: 3 };
    assert_eq!(run(&p, &r).unwrap(), json!("won"));
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[test]
fn nested_fallback_over_retry() {
    let p = Pattern::Fallback(vec![
        Pattern::Retry { inner: Box::new(Pattern::tool("bad", json!({}))), max: 1 },
        Pattern::tool("ok1", json!({})),
    ]);
    assert_eq!(run(&p, &reg()).unwrap(), json!(1));
}
