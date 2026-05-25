// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provider-specific tool encoding.
//!
//! Each LLM provider expects tools in a slightly different JSON shape:
//!
//! - **Anthropic**: `{"name", "description", "input_schema": {"type":"object",...}}`
//! - **OpenAI**:    `{"type":"function","function":{"name","description","parameters":{...}}}`
//! - **Ollama**:    `{"type":"function","function":{"name","description","parameters":{...}}}`
//!   (Ollama mirrors the OpenAI shape since the `llama-stack` 0.5+ branch.)
//! - **OpenRouter**: passthrough OpenAI shape, model field dictates routing.
//!
//! cave-hermes stores tools as `ToolEntry` (name + description + JSON-schema
//! arguments). This module emits the per-provider JSON payload from a single
//! source of truth and also parses an `assistant`-emitted `tool_use` block
//! back into a normalised `ToolInvocation`.

use crate::tool::ToolEntry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Anthropic,
    OpenAi,
    Ollama,
    OpenRouter,
}

/// A normalised tool-use request emitted by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    #[error("missing required field `{0}` in tool_use payload")]
    MissingField(&'static str),
    #[error("arguments must be a JSON object, got: {0}")]
    InvalidArgumentType(&'static str),
}

/// Encode a tool registry entry for the given provider.
pub fn encode_tool(entry: &ToolEntry, provider: ToolProvider) -> Value {
    let schema = if entry.schema.is_null() {
        empty_object_schema()
    } else {
        entry.schema.clone()
    };
    match provider {
        ToolProvider::Anthropic => json!({
            "name": entry.name,
            "description": entry.description,
            "input_schema": schema,
        }),
        ToolProvider::OpenAi | ToolProvider::Ollama | ToolProvider::OpenRouter => json!({
            "type": "function",
            "function": {
                "name": entry.name,
                "description": entry.description,
                "parameters": schema,
            }
        }),
    }
}

/// Encode the full toolset (deterministic order — registry iteration order).
pub fn encode_tools(entries: &[ToolEntry], provider: ToolProvider) -> Value {
    Value::Array(entries.iter().map(|e| encode_tool(e, provider)).collect())
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

/// Parse a tool_use payload emitted by an assistant turn back into a
/// normalised `ToolInvocation`. Accepts both Anthropic and OpenAI/Ollama shapes.
pub fn parse_tool_use(provider: ToolProvider, payload: &Value) -> Result<ToolInvocation, EncodingError> {
    match provider {
        ToolProvider::Anthropic => parse_anthropic(payload),
        ToolProvider::OpenAi | ToolProvider::Ollama | ToolProvider::OpenRouter => parse_openai(payload),
    }
}

fn parse_anthropic(payload: &Value) -> Result<ToolInvocation, EncodingError> {
    let call_id = payload.get("id").and_then(Value::as_str).map(String::from);
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .ok_or(EncodingError::MissingField("name"))?
        .to_string();
    let arguments = payload
        .get("input")
        .cloned()
        .ok_or(EncodingError::MissingField("input"))?;
    if !arguments.is_object() {
        return Err(EncodingError::InvalidArgumentType("Anthropic input must be object"));
    }
    Ok(ToolInvocation {
        call_id,
        name,
        arguments,
    })
}

fn parse_openai(payload: &Value) -> Result<ToolInvocation, EncodingError> {
    let call_id = payload.get("id").and_then(Value::as_str).map(String::from);
    let func = payload
        .get("function")
        .ok_or(EncodingError::MissingField("function"))?;
    let name = func
        .get("name")
        .and_then(Value::as_str)
        .ok_or(EncodingError::MissingField("function.name"))?
        .to_string();
    let arguments = match func.get("arguments") {
        Some(Value::String(s)) => {
            serde_json::from_str::<Value>(s).map_err(|_| {
                EncodingError::InvalidArgumentType("OpenAI tool arguments must be JSON string of an object")
            })?
        }
        Some(other) => other.clone(),
        None => return Err(EncodingError::MissingField("function.arguments")),
    };
    if !arguments.is_object() {
        return Err(EncodingError::InvalidArgumentType("OpenAI arguments must decode to object"));
    }
    Ok(ToolInvocation {
        call_id,
        name,
        arguments,
    })
}

/// Render the model's expected `tool_choice` directive. Some providers want a
/// string (`"auto"` / `"none"`), others a structured object.
pub fn encode_tool_choice(provider: ToolProvider, choice: ToolChoice<'_>) -> Value {
    match (provider, choice) {
        (ToolProvider::Anthropic, ToolChoice::Auto) => json!({"type": "auto"}),
        (ToolProvider::Anthropic, ToolChoice::Required) => json!({"type": "any"}),
        (ToolProvider::Anthropic, ToolChoice::None) => json!({"type": "none"}),
        (ToolProvider::Anthropic, ToolChoice::Specific(name)) => json!({"type":"tool","name":name}),
        (_, ToolChoice::Auto) => json!("auto"),
        (_, ToolChoice::Required) => json!("required"),
        (_, ToolChoice::None) => json!("none"),
        (_, ToolChoice::Specific(name)) => json!({"type":"function","function":{"name":name}}),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ToolChoice<'a> {
    Auto,
    Required,
    None,
    Specific(&'a str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolEntry, ToolResult};
    use std::sync::Arc;

    fn entry() -> ToolEntry {
        ToolEntry {
            name: "echo".into(),
            toolset: "test".into(),
            description: "echo input back".into(),
            schema: json!({
                "type": "object",
                "properties": {"msg": {"type":"string"}},
                "required": ["msg"],
            }),
            handler: Arc::new(|_| Ok(ToolResult::ok("ok".to_string()))),
            check_fn: None,
            requires_env: vec![],
            is_async: false,
            max_result_size_chars: None,
        }
    }

    #[test]
    fn encode_anthropic_uses_input_schema() {
        let v = encode_tool(&entry(), ToolProvider::Anthropic);
        assert_eq!(v["name"], "echo");
        assert!(v.get("input_schema").is_some());
        assert!(v.get("function").is_none());
    }

    #[test]
    fn encode_openai_uses_function_envelope() {
        let v = encode_tool(&entry(), ToolProvider::OpenAi);
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "echo");
        assert!(v["function"].get("parameters").is_some());
    }

    #[test]
    fn encode_ollama_mirrors_openai() {
        let oai = encode_tool(&entry(), ToolProvider::OpenAi);
        let ollama = encode_tool(&entry(), ToolProvider::Ollama);
        assert_eq!(oai, ollama);
    }

    #[test]
    fn empty_schema_inserted_for_no_args_tool() {
        let mut e = entry();
        e.schema = Value::Null;
        let v = encode_tool(&e, ToolProvider::OpenAi);
        assert!(v["function"]["parameters"].is_object());
    }

    #[test]
    fn encode_tools_preserves_order() {
        let mut a = entry();
        a.name = "a".into();
        let mut b = entry();
        b.name = "b".into();
        let v = encode_tools(&[a, b], ToolProvider::OpenAi);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["function"]["name"], "a");
        assert_eq!(arr[1]["function"]["name"], "b");
    }

    #[test]
    fn parse_anthropic_tool_use() {
        let p = json!({"id":"t1","name":"echo","input":{"msg":"hi"}});
        let inv = parse_tool_use(ToolProvider::Anthropic, &p).unwrap();
        assert_eq!(inv.call_id.as_deref(), Some("t1"));
        assert_eq!(inv.name, "echo");
        assert_eq!(inv.arguments["msg"], "hi");
    }

    #[test]
    fn parse_anthropic_missing_input_errors() {
        let p = json!({"name":"x"});
        assert!(parse_tool_use(ToolProvider::Anthropic, &p).is_err());
    }

    #[test]
    fn parse_openai_string_arguments() {
        let p = json!({"id":"c1","function":{"name":"echo","arguments":"{\"msg\":\"hi\"}"}});
        let inv = parse_tool_use(ToolProvider::OpenAi, &p).unwrap();
        assert_eq!(inv.name, "echo");
        assert_eq!(inv.arguments["msg"], "hi");
    }

    #[test]
    fn parse_openai_object_arguments() {
        let p = json!({"function":{"name":"echo","arguments":{"msg":"hi"}}});
        let inv = parse_tool_use(ToolProvider::OpenAi, &p).unwrap();
        assert_eq!(inv.arguments["msg"], "hi");
    }

    #[test]
    fn parse_openai_invalid_arguments_string_errors() {
        let p = json!({"function":{"name":"echo","arguments":"not json"}});
        assert!(parse_tool_use(ToolProvider::OpenAi, &p).is_err());
    }

    #[test]
    fn tool_choice_anthropic_specific() {
        let v = encode_tool_choice(ToolProvider::Anthropic, ToolChoice::Specific("echo"));
        assert_eq!(v["type"], "tool");
        assert_eq!(v["name"], "echo");
    }

    #[test]
    fn tool_choice_openai_specific() {
        let v = encode_tool_choice(ToolProvider::OpenAi, ToolChoice::Specific("echo"));
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "echo");
    }

    #[test]
    fn tool_choice_string_modes() {
        for mode in [ToolChoice::Auto, ToolChoice::Required, ToolChoice::None] {
            let v = encode_tool_choice(ToolProvider::OpenAi, mode);
            assert!(v.is_string());
        }
    }

    #[test]
    fn tool_choice_anthropic_uses_object_form() {
        for mode in [ToolChoice::Auto, ToolChoice::Required, ToolChoice::None] {
            let v = encode_tool_choice(ToolProvider::Anthropic, mode);
            assert!(v.is_object());
            assert!(v.get("type").is_some());
        }
    }
}
