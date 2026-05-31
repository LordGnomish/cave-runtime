// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 1 (RED→GREEN): Tool trait, ToolResult, ToolSpec, ToolRegistry.

use cave_tools::tool::{Content, FnTool, Tool, ToolRegistry, ToolResult};
use serde_json::json;

fn echo() -> FnTool {
    FnTool::new(
        "echo",
        "Echo the `msg` argument back to the caller",
        json!({
            "type": "object",
            "properties": {"msg": {"type": "string"}},
            "required": ["msg"]
        }),
        |args| {
            let msg = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
            Ok(ToolResult::text(msg))
        },
    )
    .with_toolset("core")
}

#[test]
fn tool_trait_surface() {
    let t = echo();
    assert_eq!(t.name(), "echo");
    assert_eq!(t.toolset(), "core");
    assert!(t.description().contains("Echo"));
    assert_eq!(t.input_schema()["type"], "object");
}

#[test]
fn tool_result_text_helper_is_not_error() {
    let r = ToolResult::text("hi");
    assert!(!r.is_error);
    assert_eq!(r.content.len(), 1);
    assert_eq!(r.text_output(), "hi");
    match &r.content[0] {
        Content::Text { text } => assert_eq!(text, "hi"),
    }
}

#[test]
fn tool_result_error_helper_marks_is_error() {
    let r = ToolResult::error("boom");
    assert!(r.is_error);
    assert_eq!(r.text_output(), "boom");
}

#[test]
fn tool_result_serializes_in_mcp_shape() {
    let r = ToolResult::text("ok").with_structured(json!({"n": 1}));
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["content"][0]["text"], "ok");
    assert_eq!(v["isError"], false);
    assert_eq!(v["structuredContent"]["n"], 1);
}

#[test]
fn registry_register_get_invoke_roundtrip() {
    let mut reg = ToolRegistry::new();
    reg.register(echo());
    assert_eq!(reg.len(), 1);
    assert!(reg.get("echo").is_some());
    let out = reg.invoke("echo", &json!({"msg": "yo"})).unwrap();
    assert_eq!(out.text_output(), "yo");
}

#[test]
fn registry_invoke_unknown_is_not_found() {
    let reg = ToolRegistry::new();
    let err = reg.invoke("nope", &json!({})).unwrap_err();
    assert_eq!(err.code(), "tool_not_found");
}

#[test]
fn registry_names_sorted_and_specs_match_mcp_fields() {
    let mut reg = ToolRegistry::new();
    reg.register(echo());
    reg.register(FnTool::new(
        "alpha",
        "first",
        json!({"type": "object"}),
        |_| Ok(ToolResult::text("a")),
    ));
    assert_eq!(reg.names(), vec!["alpha".to_string(), "echo".to_string()]);
    let specs = reg.list_specs();
    assert_eq!(specs.len(), 2);
    let echo_spec = specs.iter().find(|s| s.name == "echo").unwrap();
    assert_eq!(echo_spec.description, "Echo the `msg` argument back to the caller");
    assert_eq!(echo_spec.input_schema["type"], "object");
    // MCP tools/list entry shape.
    let wire = serde_json::to_value(echo_spec).unwrap();
    assert_eq!(wire["name"], "echo");
    assert!(wire.get("inputSchema").is_some());
}

#[test]
fn registry_register_returns_previous_on_replace() {
    let mut reg = ToolRegistry::new();
    reg.register(echo());
    let prev = reg.register(FnTool::new(
        "echo",
        "v2",
        json!({"type": "object"}),
        |_| Ok(ToolResult::text("v2")),
    ));
    assert!(prev.is_some());
    assert_eq!(reg.invoke("echo", &json!({})).unwrap().text_output(), "v2");
}
