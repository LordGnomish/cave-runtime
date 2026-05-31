// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Interop with the OpenAI function-calling wire format and LangChain tools.
//!
//! The registry is the source of truth; these adapters project a
//! [`ToolSpec`] onto the shapes other ecosystems expect and parse their
//! responses back into framework calls:
//!
//! * [`to_openai_tool`] / [`to_openai_tools`] — the `{type:"function",
//!   function:{name,description,parameters}}` envelope for a request's
//!   `tools` array.
//! * [`parse_openai_tool_calls`] — extract `(id, name, arguments)` from an
//!   assistant message's `tool_calls` (arguments may be a JSON *string* or
//!   an already-parsed object).
//! * [`openai_tool_message`] — wrap a [`ToolResult`] as a `role:"tool"`
//!   message to feed back to the model.
//! * [`to_langchain_tool`] — LangChain `StructuredTool`-style descriptor
//!   exposing the parameter `args`.

use serde_json::{json, Value};

use crate::tool::{ToolResult, ToolSpec};

/// A tool call parsed out of a model response.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Project a tool spec onto the OpenAI `tools` entry (function envelope).
pub fn to_openai_tool(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.input_schema,
        }
    })
}

/// Project many specs onto the array placed in a request's `tools` field.
pub fn to_openai_tools(specs: &[ToolSpec]) -> Value {
    Value::Array(specs.iter().map(to_openai_tool).collect())
}

/// Extract tool calls from a model response. Accepts either an assistant
/// message object (with a `tool_calls` array) or a bare array of tool-call
/// objects. Each call's `arguments` may be a JSON string (OpenAI's default)
/// or an already-decoded object; both are normalised to a [`Value`].
pub fn parse_openai_tool_calls(response: &Value) -> Vec<ParsedToolCall> {
    let raw = match response {
        Value::Array(a) => a.as_slice(),
        Value::Object(_) => response
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => &[],
    };

    raw.iter()
        .filter_map(|call| {
            let func = call.get("function")?;
            let name = func.get("name").and_then(Value::as_str)?.to_string();
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let arguments = match func.get("arguments") {
                // JSON-encoded string (canonical OpenAI form).
                Some(Value::String(s)) => {
                    serde_json::from_str(s).unwrap_or_else(|_| json!({}))
                }
                // Already-decoded object/array (some providers / SDKs).
                Some(v) => v.clone(),
                None => json!({}),
            };
            Some(ParsedToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

/// Wrap a tool result as the `role:"tool"` message fed back to the model.
pub fn openai_tool_message(tool_call_id: &str, result: &ToolResult) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": result.text_output(),
    })
}

/// Project a spec onto a LangChain `StructuredTool`-style descriptor. The
/// `args` field exposes the schema's `properties` map, matching
/// `StructuredTool.args`.
pub fn to_langchain_tool(spec: &ToolSpec) -> Value {
    let args = spec
        .input_schema
        .get("properties")
        .cloned()
        .unwrap_or_else(|| json!({}));
    json!({
        "name": spec.name,
        "description": spec.description,
        "args": args,
    })
}
