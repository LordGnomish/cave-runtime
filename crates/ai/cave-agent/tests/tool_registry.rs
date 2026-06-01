// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tool-registry primitive: registration, schema catalog, invocation, and the
//! pure built-in tools.

use cave_agent::error::AgentError;
use cave_agent::tool::{builtins, Tool, ToolRegistry};
use serde_json::json;

#[test]
fn register_then_invoke_roundtrips() {
    let mut reg = ToolRegistry::new();
    reg.register(Tool::new(
        "double",
        "double an integer",
        json!({"type": "object", "required": ["n"]}),
        |args| {
            let n = args.get("n").and_then(|v| v.as_i64()).ok_or_else(|| {
                AgentError::InvalidArguments {
                    tool: "double".into(),
                    reason: "n must be an integer".into(),
                }
            })?;
            Ok(json!({ "result": n * 2 }))
        },
    ));
    assert_eq!(reg.len(), 1);
    let out = reg.invoke("double", &json!({"n": 21})).unwrap();
    assert_eq!(out["result"], 42);
}

#[test]
fn invoke_unknown_tool_errors() {
    let reg = ToolRegistry::new();
    let err = reg.invoke("ghost", &json!({})).unwrap_err();
    assert_eq!(err, AgentError::UnknownTool("ghost".into()));
}

#[test]
fn invoke_missing_required_field_rejected_before_handler() {
    let mut reg = ToolRegistry::new();
    // Handler would panic if reached without `n`; registry must reject first
    // because the schema marks `n` required.
    reg.register(Tool::new(
        "needs_n",
        "requires n",
        json!({"type": "object", "required": ["n"]}),
        |args| Ok(json!({ "n": args["n"] })),
    ));
    let err = reg.invoke("needs_n", &json!({"other": 1})).unwrap_err();
    match err {
        AgentError::InvalidArguments { tool, .. } => assert_eq!(tool, "needs_n"),
        other => panic!("expected InvalidArguments, got {other:?}"),
    }
}

#[test]
fn catalog_is_sorted_and_serializable() {
    let mut reg = ToolRegistry::new();
    for n in ["zeta", "alpha", "mu"] {
        reg.register(Tool::new(n, "x", json!({}), |_| Ok(json!(null))));
    }
    let cat = reg.catalog();
    let names: Vec<&str> = cat.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["alpha", "mu", "zeta"], "catalog must be name-sorted");
    // round-trips through serde
    let s = serde_json::to_string(&cat).unwrap();
    assert!(s.contains("alpha"));
}

#[test]
fn builtin_calc_evaluates_four_ops() {
    let reg = builtins();
    assert_eq!(reg.invoke("calc", &json!({"op":"add","a":2,"b":3})).unwrap()["result"], 5.0);
    assert_eq!(reg.invoke("calc", &json!({"op":"sub","a":7,"b":4})).unwrap()["result"], 3.0);
    assert_eq!(reg.invoke("calc", &json!({"op":"mul","a":6,"b":7})).unwrap()["result"], 42.0);
    assert_eq!(reg.invoke("calc", &json!({"op":"div","a":9,"b":2})).unwrap()["result"], 4.5);
}

#[test]
fn builtin_calc_div_by_zero_is_tool_failure() {
    let reg = builtins();
    let err = reg.invoke("calc", &json!({"op":"div","a":1,"b":0})).unwrap_err();
    assert!(matches!(err, AgentError::ToolFailed { .. }));
}

#[test]
fn builtin_str_tools_present() {
    let reg = builtins();
    assert!(reg.names().contains(&"str_upper".to_string()));
    assert_eq!(
        reg.invoke("str_upper", &json!({"s":"jarvis"})).unwrap()["result"],
        "JARVIS"
    );
    assert_eq!(reg.invoke("str_len", &json!({"s":"abcd"})).unwrap()["result"], 4);
}
