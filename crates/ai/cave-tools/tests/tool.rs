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
        other => panic!("expected text content, got {other:?}"),
    }
}

// ── Cycle 11 (RED→GREEN): full MCP ContentBlock union ───────────────────────
// schema/2025-11-25/schema.ts: ContentBlock =
//   TextContent | ImageContent | AudioContent | ResourceLink | EmbeddedResource

#[test]
fn image_content_serializes_with_data_and_mime() {
    let c = Content::image("aGVsbG8=", "image/png");
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["type"], "image");
    assert_eq!(v["data"], "aGVsbG8=");
    assert_eq!(v["mimeType"], "image/png");
}

#[test]
fn audio_content_serializes_with_data_and_mime() {
    let c = Content::audio("YXVkaW8=", "audio/wav");
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["type"], "audio");
    assert_eq!(v["data"], "YXVkaW8=");
    assert_eq!(v["mimeType"], "audio/wav");
}

#[test]
fn embedded_text_resource_serializes_in_mcp_shape() {
    let c = Content::resource_text("file:///a.txt", Some("text/plain"), "body");
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["type"], "resource");
    assert_eq!(v["resource"]["uri"], "file:///a.txt");
    assert_eq!(v["resource"]["mimeType"], "text/plain");
    assert_eq!(v["resource"]["text"], "body");
    assert!(v["resource"].get("blob").is_none(), "text resource omits blob");
}

#[test]
fn embedded_blob_resource_serializes_in_mcp_shape() {
    let c = Content::resource_blob("file:///a.bin", Some("application/octet-stream"), "AAEC");
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["type"], "resource");
    assert_eq!(v["resource"]["uri"], "file:///a.bin");
    assert_eq!(v["resource"]["blob"], "AAEC");
    assert!(v["resource"].get("text").is_none(), "blob resource omits text");
}

#[test]
fn resource_link_serializes_with_uri_and_name() {
    let c = Content::resource_link("file:///main.rs", "main.rs");
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["type"], "resource_link");
    assert_eq!(v["uri"], "file:///main.rs");
    assert_eq!(v["name"], "main.rs");
}

#[test]
fn content_blocks_round_trip_through_serde() {
    let blocks = vec![
        Content::text("t"),
        Content::image("ZA==", "image/jpeg"),
        Content::audio("ZA==", "audio/ogg"),
        Content::resource_text("file:///x", None, "x"),
        Content::resource_blob("file:///y", None, "eQ=="),
        Content::resource_link("file:///z", "z"),
    ];
    let wire = serde_json::to_value(&blocks).unwrap();
    let back: Vec<Content> = serde_json::from_value(wire).unwrap();
    assert_eq!(back, blocks);
}

#[test]
fn text_output_collects_only_text_blocks() {
    let r = ToolResult {
        content: vec![
            Content::text("hello"),
            Content::image("ZA==", "image/png"),
            Content::text("world"),
        ],
        is_error: false,
        structured: None,
    };
    // Non-text blocks contribute nothing to the textual projection.
    assert_eq!(r.text_output(), "hello\nworld");
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
