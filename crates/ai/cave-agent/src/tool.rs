// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tool registry — the composable building block every other primitive leans
//! on. A [`Tool`] is a named, schema-described unit of work; a [`ToolRegistry`]
//! holds them, validates arguments against the declared schema, and dispatches
//! by name.
//!
//! OpenJarvis upstream: `jarvis/tools/registry.py` (registration + catalog) and
//! `jarvis/tools/builtins.py` (the on-device pure tools). The dynamic
//! Python-import discovery walk is scope-cut — Rust registers explicitly.

use crate::error::{AgentError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The handler signature: pure function from a JSON argument object to a JSON
/// result (or an [`AgentError`]).
pub type Handler = Box<dyn Fn(&Value) -> Result<Value> + Send + Sync>;

/// A registered tool: an identity, a human description, a JSON-schema fragment
/// describing its arguments, and the handler that runs it.
pub struct Tool {
    /// Stable invocation name.
    pub name: String,
    /// One-line human description (surfaced in the catalog).
    pub description: String,
    /// JSON-schema object. Only the `required` array is enforced by the
    /// registry; richer validation is the handler's job.
    pub schema: Value,
    handler: Handler,
}

impl Tool {
    /// Construct a tool from its parts. The handler may be any closure.
    pub fn new<N, D, F>(name: N, description: D, schema: Value, handler: F) -> Self
    where
        N: Into<String>,
        D: Into<String>,
        F: Fn(&Value) -> Result<Value> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
            handler: Box::new(handler),
        }
    }

    /// The `required` field names declared by this tool's schema, if any.
    fn required_fields(&self) -> Vec<&str> {
        self.schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default()
    }
}

/// The serializable view of a tool used by the catalog and HTTP surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// A name-indexed collection of tools.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Tool>,
}

impl ToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a tool. Last writer wins on name collision.
    pub fn register(&mut self, tool: Tool) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry holds no tools.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// All tool names, sorted (the [`BTreeMap`] keeps order).
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Borrow a tool by name.
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// The name-sorted, serializable catalog of every registered tool.
    pub fn catalog(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|t| ToolSpec {
                name: t.name.clone(),
                description: t.description.clone(),
                schema: t.schema.clone(),
            })
            .collect()
    }

    /// Validate `args` against the tool's declared `required` fields and, if
    /// they are all present, dispatch to the handler.
    pub fn invoke(&self, name: &str, args: &Value) -> Result<Value> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| AgentError::UnknownTool(name.to_string()))?;
        for field in tool.required_fields() {
            let present = args.get(field).map(|v| !v.is_null()).unwrap_or(false);
            if !present {
                return Err(AgentError::InvalidArguments {
                    tool: name.to_string(),
                    reason: format!("missing required field `{field}`"),
                });
            }
        }
        (tool.handler)(args)
    }
}

/// Read a required string argument or raise [`AgentError::InvalidArguments`].
fn arg_str<'a>(tool: &str, args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::InvalidArguments {
            tool: tool.to_string(),
            reason: format!("`{key}` must be a string"),
        })
}

/// Read a required numeric argument as f64.
fn arg_f64(tool: &str, args: &Value, key: &str) -> Result<f64> {
    args.get(key)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| AgentError::InvalidArguments {
            tool: tool.to_string(),
            reason: format!("`{key}` must be a number"),
        })
}

/// A registry pre-loaded with the pure, on-device built-in tools. These run
/// without any IO so they are safe to invoke in tests, plans, and the
/// evaluation harness.
pub fn builtins() -> ToolRegistry {
    let mut reg = ToolRegistry::new();

    reg.register(Tool::new(
        "calc",
        "evaluate a binary arithmetic op (add|sub|mul|div)",
        serde_json::json!({"type":"object","required":["op","a","b"]}),
        |args| {
            let op = arg_str("calc", args, "op")?;
            let a = arg_f64("calc", args, "a")?;
            let b = arg_f64("calc", args, "b")?;
            let result = match op {
                "add" => a + b,
                "sub" => a - b,
                "mul" => a * b,
                "div" => {
                    if b == 0.0 {
                        return Err(AgentError::ToolFailed {
                            tool: "calc".into(),
                            reason: "division by zero".into(),
                        });
                    }
                    a / b
                }
                other => {
                    return Err(AgentError::InvalidArguments {
                        tool: "calc".into(),
                        reason: format!("unknown op `{other}`"),
                    })
                }
            };
            Ok(serde_json::json!({ "result": result }))
        },
    ));

    reg.register(Tool::new(
        "str_upper",
        "uppercase a string",
        serde_json::json!({"type":"object","required":["s"]}),
        |args| {
            let s = arg_str("str_upper", args, "s")?;
            Ok(serde_json::json!({ "result": s.to_uppercase() }))
        },
    ));

    reg.register(Tool::new(
        "str_len",
        "count the characters in a string",
        serde_json::json!({"type":"object","required":["s"]}),
        |args| {
            let s = arg_str("str_len", args, "s")?;
            Ok(serde_json::json!({ "result": s.chars().count() }))
        },
    ));

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn last_writer_wins_on_name_collision() {
        let mut reg = ToolRegistry::new();
        reg.register(Tool::new("t", "v1", json!({}), |_| Ok(json!(1))));
        reg.register(Tool::new("t", "v2", json!({}), |_| Ok(json!(2))));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.invoke("t", &json!({})).unwrap(), json!(2));
    }

    #[test]
    fn builtins_count_is_three() {
        assert_eq!(builtins().len(), 3);
    }
}
