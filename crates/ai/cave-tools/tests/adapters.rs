// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 10 (RED→GREEN): OpenAI function-calling + LangChain adapters.

use cave_tools::adapters::{
    openai_tool_message, parse_openai_tool_calls, to_langchain_tool, to_openai_tool,
    to_openai_tools,
};
use cave_tools::tool::{FnTool, ToolRegistry, ToolResult, ToolSpec};
use serde_json::json;

fn add_spec() -> ToolSpec {
    ToolSpec {
        name: "add".into(),
        description: "add two integers".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
            "required": ["a", "b"]
        }),
        toolset: "math".into(),
    }
}

#[test]
fn to_openai_tool_has_function_envelope() {
    let v = to_openai_tool(&add_spec());
    assert_eq!(v["type"], "function");
    assert_eq!(v["function"]["name"], "add");
    assert_eq!(v["function"]["description"], "add two integers");
    assert_eq!(v["function"]["parameters"]["required"][0], "a");
}

#[test]
fn to_openai_tools_returns_array() {
    let v = to_openai_tools(&[add_spec()]);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["function"]["name"], "add");
}

#[test]
fn parse_tool_calls_from_message_with_string_arguments() {
    // The shape an OpenAI assistant message carries.
    let msg = json!({
        "role": "assistant",
        "tool_calls": [
            {"id": "call_1", "type": "function",
             "function": {"name": "add", "arguments": "{\"a\": 2, \"b\": 5}"}}
        ]
    });
    let calls = parse_openai_tool_calls(&msg);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "add");
    assert_eq!(calls[0].arguments["a"], 2);
    assert_eq!(calls[0].arguments["b"], 5);
}

#[test]
fn parse_tool_calls_accepts_bare_array_and_object_arguments() {
    // Some providers hand back arguments already parsed as an object.
    let arr = json!([
        {"id": "c2", "function": {"name": "add", "arguments": {"a": 1, "b": 1}}}
    ]);
    let calls = parse_openai_tool_calls(&arr);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments["a"], 1);
}

#[test]
fn to_langchain_tool_exposes_args_properties() {
    let v = to_langchain_tool(&add_spec());
    assert_eq!(v["name"], "add");
    assert_eq!(v["description"], "add two integers");
    // LangChain's StructuredTool surfaces the properties as `args`.
    assert!(v["args"]["a"].is_object());
    assert!(v["args"]["b"].is_object());
}

#[test]
fn openai_tool_message_wraps_result_for_the_model() {
    let res = ToolResult::text("7");
    let msg = openai_tool_message("call_1", &res);
    assert_eq!(msg["role"], "tool");
    assert_eq!(msg["tool_call_id"], "call_1");
    assert_eq!(msg["content"], "7");
}

#[test]
fn full_round_trip_openai_to_registry_to_message() {
    let mut reg = ToolRegistry::new();
    reg.register(FnTool::new(
        "add",
        "add two integers",
        json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
            "required": ["a", "b"]
        }),
        |args| {
            let s = args["a"].as_i64().unwrap() + args["b"].as_i64().unwrap();
            Ok(ToolResult::text(s.to_string()))
        },
    ));
    // 1. Advertise the tool to the model.
    let _tools = to_openai_tools(&reg.list_specs());
    // 2. Model replies with a tool call.
    let msg = json!({
        "tool_calls": [
            {"id": "call_9", "function": {"name": "add", "arguments": "{\"a\": 4, \"b\": 3}"}}
        ]
    });
    // 3. Parse, execute, format the tool-result message.
    let call = &parse_openai_tool_calls(&msg)[0];
    let result = reg.invoke_validated(&call.name, &call.arguments).unwrap();
    let tool_msg = openai_tool_message(&call.id, &result);
    assert_eq!(tool_msg["tool_call_id"], "call_9");
    assert_eq!(tool_msg["content"], "7");
}
