// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl chat --tool-mode` — function-calling primitives.
//!
//! Tool registry → execution dispatch. A tool exposes a JSON-Schema for inputs,
//! a per-tenant grant gate, and an in-process executor. The CLI surfaces:
//!   - tool list (`/tools` REPL command),
//!   - tool dispatch (LLM-issued `ToolCall`),
//!   - result feedback to the conversation.

use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tenant_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub tenant_id: String,
    pub tool_name: String,
    pub ok: bool,
    pub output: serde_json::Value,
}

pub type ToolHandler = Arc<dyn Fn(&serde_json::Value) -> Result<serde_json::Value> + Send + Sync>;

#[derive(Default)]
pub struct ToolMode {
    /// tool_name → handler
    handlers: Arc<RwLock<HashMap<String, ToolHandler>>>,
    /// tenant_id → set of granted tool names
    grants: Arc<RwLock<HashMap<String, HashSet<String>>>>,
}

impl ToolMode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&self, tool_name: impl Into<String>, handler: F)
    where
        F: Fn(&serde_json::Value) -> Result<serde_json::Value> + Send + Sync + 'static,
    {
        self.handlers
            .write()
            .insert(tool_name.into(), Arc::new(handler));
    }

    pub fn grant(&self, tenant_id: impl Into<String>, tool_name: impl Into<String>) {
        let tid = tenant_id.into();
        let name = tool_name.into();
        self.grants
            .write()
            .entry(tid)
            .or_default()
            .insert(name);
    }

    pub fn revoke(&self, tenant_id: &str, tool_name: &str) {
        if let Some(set) = self.grants.write().get_mut(tenant_id) {
            set.remove(tool_name);
        }
    }

    pub fn granted_for(&self, tenant_id: &str) -> Vec<String> {
        let g = self.grants.read();
        let mut out: Vec<String> = g
            .get(tenant_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        out.sort();
        out
    }

    pub fn is_granted(&self, tenant_id: &str, tool_name: &str) -> bool {
        self.grants
            .read()
            .get(tenant_id)
            .map(|s| s.contains(tool_name))
            .unwrap_or(false)
    }

    pub fn dispatch(&self, call: &ToolCall) -> ToolResult {
        if !self.is_granted(&call.tenant_id, &call.tool_name) {
            return ToolResult {
                call_id: call.call_id.clone(),
                tenant_id: call.tenant_id.clone(),
                tool_name: call.tool_name.clone(),
                ok: false,
                output: serde_json::json!({
                    "error": format!(
                        "tool '{}' not granted for tenant '{}'",
                        call.tool_name, call.tenant_id
                    ),
                }),
            };
        }
        let handler = match self.handlers.read().get(&call.tool_name).cloned() {
            Some(h) => h,
            None => {
                return ToolResult {
                    call_id: call.call_id.clone(),
                    tenant_id: call.tenant_id.clone(),
                    tool_name: call.tool_name.clone(),
                    ok: false,
                    output: serde_json::json!({
                        "error": format!("unknown tool '{}'", call.tool_name),
                    }),
                }
            }
        };
        match handler(&call.arguments) {
            Ok(v) => ToolResult {
                call_id: call.call_id.clone(),
                tenant_id: call.tenant_id.clone(),
                tool_name: call.tool_name.clone(),
                ok: true,
                output: v,
            },
            Err(e) => ToolResult {
                call_id: call.call_id.clone(),
                tenant_id: call.tenant_id.clone(),
                tool_name: call.tool_name.clone(),
                ok: false,
                output: serde_json::json!({ "error": e.to_string() }),
            },
        }
    }
}

/// Helper: build a ToolCall with a generated id.
pub fn make_call(
    tenant_id: impl Into<String>,
    tool_name: impl Into<String>,
    args: serde_json::Value,
) -> ToolCall {
    ToolCall {
        call_id: uuid::Uuid::new_v4().to_string(),
        tenant_id: tenant_id.into(),
        tool_name: tool_name.into(),
        arguments: args,
    }
}

/// Standard echo tool useful for tests + REPL smoke-checks.
pub fn echo_handler(args: &serde_json::Value) -> Result<serde_json::Value> {
    Ok(serde_json::json!({ "echoed": args.clone() }))
}

/// Standard add tool: { a: number, b: number } → { sum: number }
pub fn add_handler(args: &serde_json::Value) -> Result<serde_json::Value> {
    let a = args
        .get("a")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| anyhow!("missing numeric 'a'"))?;
    let b = args
        .get("b")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| anyhow!("missing numeric 'b'"))?;
    Ok(serde_json::json!({ "sum": a + b }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cite: tool grant — tenant gets per-tool authorization
    #[test]
    fn toolmode_acme_grant_lists_tool() {
        let tenant_id = "acme";
        let m = ToolMode::new();
        m.grant(tenant_id, "echo");
        assert!(m.is_granted(tenant_id, "echo"));
        assert_eq!(m.granted_for(tenant_id), vec!["echo".to_string()]);
    }

    /// cite: tool grant — revoke removes the entry
    #[test]
    fn toolmode_globex_revoke_removes_grant() {
        let tenant_id = "globex";
        let m = ToolMode::new();
        m.grant(tenant_id, "echo");
        m.revoke(tenant_id, "echo");
        assert!(!m.is_granted(tenant_id, "echo"));
    }

    /// cite: tool dispatch — successful echo
    #[test]
    fn toolmode_acme_dispatch_echo_succeeds() {
        let tenant_id = "acme";
        let m = ToolMode::new();
        m.register("echo", echo_handler);
        m.grant(tenant_id, "echo");
        let call = make_call(tenant_id, "echo", serde_json::json!({"hi": 1}));
        let res = m.dispatch(&call);
        assert!(res.ok);
        assert_eq!(res.output["echoed"]["hi"], serde_json::json!(1));
    }

    /// cite: tool dispatch — ungranted tool rejected for that tenant
    #[test]
    fn toolmode_globex_dispatch_ungranted_rejected() {
        let tenant_id = "globex";
        let m = ToolMode::new();
        m.register("echo", echo_handler);
        // grant for a different tenant
        m.grant("acme", "echo");
        let call = make_call(tenant_id, "echo", serde_json::json!({}));
        let res = m.dispatch(&call);
        assert!(!res.ok);
        assert!(res.output["error"]
            .as_str()
            .unwrap()
            .contains("not granted"));
    }

    /// cite: tool dispatch — unknown tool reported as such
    #[test]
    fn toolmode_initech_dispatch_unknown_tool() {
        let tenant_id = "initech";
        let m = ToolMode::new();
        m.grant(tenant_id, "ghost");
        let call = make_call(tenant_id, "ghost", serde_json::json!({}));
        let res = m.dispatch(&call);
        assert!(!res.ok);
        assert!(res.output["error"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }

    /// cite: tool dispatch — handler error wrapped in ok=false output
    #[test]
    fn toolmode_acme_handler_error_wrapped() {
        let tenant_id = "acme";
        let m = ToolMode::new();
        m.register("add", add_handler);
        m.grant(tenant_id, "add");
        let call = make_call(tenant_id, "add", serde_json::json!({"a": 1}));
        let res = m.dispatch(&call);
        assert!(!res.ok);
        assert!(res.output["error"].as_str().unwrap().contains("'b'"));
    }

    /// cite: tool dispatch — add handler computes correct sum
    #[test]
    fn toolmode_acme_add_handler_sums_numbers() {
        let tenant_id = "acme";
        let m = ToolMode::new();
        m.register("add", add_handler);
        m.grant(tenant_id, "add");
        let call = make_call(tenant_id, "add", serde_json::json!({"a": 2.5, "b": 3.5}));
        let res = m.dispatch(&call);
        assert!(res.ok);
        assert_eq!(res.output["sum"], serde_json::json!(6.0));
    }

    /// cite: tool grant — granted_for is sorted and per-tenant
    #[test]
    fn toolmode_acme_granted_for_is_sorted() {
        let tenant_id = "acme";
        let m = ToolMode::new();
        m.grant(tenant_id, "echo");
        m.grant(tenant_id, "add");
        m.grant("globex", "danger");
        assert_eq!(m.granted_for(tenant_id), vec!["add".to_string(), "echo".to_string()]);
    }

    /// cite: tool dispatch — call_id propagated to result for correlation
    #[test]
    fn toolmode_acme_dispatch_propagates_call_id() {
        let tenant_id = "acme";
        let m = ToolMode::new();
        m.register("echo", echo_handler);
        m.grant(tenant_id, "echo");
        let call = make_call(tenant_id, "echo", serde_json::json!({}));
        let id = call.call_id.clone();
        let res = m.dispatch(&call);
        assert_eq!(res.call_id, id);
    }
}
