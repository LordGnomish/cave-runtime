// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 3 (RED→GREEN): MCP JSON-RPC 2.0 server compatibility.

use cave_tools::mcp::McpServer;
use cave_tools::tool::{FnTool, ToolRegistry, ToolResult};
use serde_json::json;

fn server() -> McpServer {
    let mut reg = ToolRegistry::new();
    reg.register(FnTool::new(
        "add",
        "add two integers a + b",
        json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
            "required": ["a", "b"]
        }),
        |args| {
            let a = args["a"].as_i64().unwrap();
            let b = args["b"].as_i64().unwrap();
            Ok(ToolResult::text((a + b).to_string()))
        },
    ));
    reg.register(FnTool::new(
        "boom",
        "always fails",
        json!({"type": "object"}),
        |_| Err(cave_tools::ToolError::Execution {
            tool: "boom".into(),
            reason: "kaboom".into(),
        }),
    ));
    McpServer::new(reg, "cave-tools-test")
}

#[test]
fn initialize_negotiates_capabilities() {
    let s = server();
    let req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "c", "version": "0"}}
    });
    let resp = s.handle(&req).unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(resp["result"]["serverInfo"]["name"], "cave-tools-test");
    assert!(resp["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn tools_list_returns_specs() {
    let s = server();
    let resp = s
        .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .unwrap();
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    let add = tools.iter().find(|t| t["name"] == "add").unwrap();
    assert!(add["inputSchema"].is_object());
    assert!(add["description"].as_str().unwrap().contains("add"));
}

#[test]
fn tools_list_paginates_with_cursor() {
    let s = server().with_page_size(1);
    let p1 = s
        .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}))
        .unwrap();
    assert_eq!(p1["result"]["tools"].as_array().unwrap().len(), 1);
    let cursor = p1["result"]["nextCursor"].as_str().unwrap().to_string();
    let p2 = s
        .handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {"cursor": cursor}}))
        .unwrap();
    assert_eq!(p2["result"]["tools"].as_array().unwrap().len(), 1);
    // Second (last) page has no further cursor.
    assert!(p2["result"].get("nextCursor").is_none());
    // The two pages cover distinct tools.
    assert_ne!(p1["result"]["tools"][0]["name"], p2["result"]["tools"][0]["name"]);
}

#[test]
fn tools_call_success_returns_content() {
    let s = server();
    let resp = s
        .handle(&json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "add", "arguments": {"a": 2, "b": 3}}
        }))
        .unwrap();
    assert_eq!(resp["result"]["content"][0]["text"], "5");
    assert_eq!(resp["result"]["isError"], false);
}

#[test]
fn tools_call_unknown_tool_is_jsonrpc_error() {
    let s = server();
    let resp = s
        .handle(&json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": {"name": "ghost", "arguments": {}}
        }))
        .unwrap();
    assert!(resp.get("result").is_none());
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn tools_call_invalid_args_is_jsonrpc_error() {
    let s = server();
    let resp = s
        .handle(&json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": "add", "arguments": {"a": 1}}
        }))
        .unwrap();
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn tools_call_handler_error_is_result_with_iserror() {
    // Per MCP, tool *execution* failures are returned as a normal result
    // with isError:true so the model can react — not a protocol error.
    let s = server();
    let resp = s
        .handle(&json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": {"name": "boom", "arguments": {}}
        }))
        .unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(resp["result"]["isError"], true);
    assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("kaboom"));
}

#[test]
fn unknown_method_is_method_not_found() {
    let s = server();
    let resp = s
        .handle(&json!({"jsonrpc": "2.0", "id": 9, "method": "frobnicate"}))
        .unwrap();
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn notification_yields_no_response() {
    let s = server();
    // No `id` ⇒ JSON-RPC notification ⇒ no response.
    let out = s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert!(out.is_none());
}

#[test]
fn malformed_envelope_is_invalid_request() {
    let s = server();
    let resp = s.handle(&json!({"id": 10, "method": "tools/list"})).unwrap();
    assert_eq!(resp["error"]["code"], -32600);
}

// ── Cycle 12 (RED→GREEN): tools list_changed notifications ──────────────────
// server/tools.mdx: a server whose tool list can change advertises
// capabilities.tools.listChanged = true and emits
// notifications/tools/list_changed (a JSON-RPC notification: no id) when it
// mutates. Transport delivery rides the (separately-skipped) transport seam.

#[test]
fn initialize_advertises_list_changed_capability() {
    let s = server();
    let resp = s
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .unwrap();
    assert_eq!(resp["result"]["capabilities"]["tools"]["listChanged"], true);
}

#[test]
fn fresh_server_has_no_pending_list_changed() {
    let s = server();
    assert!(!s.list_changed_pending());
    assert!(s.take_list_changed_notification().is_none());
}

#[test]
fn registering_a_tool_makes_list_changed_pending() {
    let mut s = server();
    assert!(!s.list_changed_pending());
    s.register_tool(FnTool::new(
        "ping_tool",
        "p",
        json!({"type": "object"}),
        |_| Ok(ToolResult::text("pong")),
    ));
    assert!(s.list_changed_pending());
    // tools/list now reflects the new tool.
    let resp = s
        .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .unwrap();
    assert_eq!(resp["result"]["tools"].as_array().unwrap().len(), 3);
}

#[test]
fn list_changed_notification_is_idless_and_well_formed() {
    let mut s = server();
    s.register_tool(FnTool::new(
        "x",
        "x",
        json!({"type": "object"}),
        |_| Ok(ToolResult::text("x")),
    ));
    let note = s.take_list_changed_notification().expect("pending notification");
    assert_eq!(note["jsonrpc"], "2.0");
    assert_eq!(note["method"], "notifications/tools/list_changed");
    assert!(note.get("id").is_none(), "a notification carries no id");
    // Draining clears the pending flag (idempotent until the next change).
    assert!(!s.list_changed_pending());
    assert!(s.take_list_changed_notification().is_none());
}

#[test]
fn removing_a_tool_also_signals_list_changed() {
    let mut s = server();
    let removed = s.unregister_tool("add");
    assert!(removed, "add was present");
    assert!(s.list_changed_pending());
    // Removing an absent tool is a no-op and signals nothing new.
    let _ = s.take_list_changed_notification();
    assert!(!s.unregister_tool("nope"));
    assert!(!s.list_changed_pending());
}
