// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! A transport-agnostic Model Context Protocol server.
//!
//! Ports the request-handling half of the MCP spec (`schema/2025-11-25`):
//! the `initialize` handshake (capability negotiation + `serverInfo`),
//! `tools/list` (cursor pagination), and `tools/call` (returning a
//! `CallToolResult`). It speaks JSON-RPC 2.0 over [`serde_json::Value`]s;
//! the caller owns the actual stdio / HTTP / SSE transport.
//!
//! Error mapping follows the spec's split:
//! * *protocol* failures (unknown method, unknown tool, invalid params)
//!   become JSON-RPC `error` responses;
//! * *tool execution* failures become a successful response whose
//!   `result.isError` is `true`, so the model can see and react to them.

use serde_json::{json, Value};

use crate::error::ToolError;
use crate::tool::ToolRegistry;

/// Default number of tools returned per `tools/list` page when the client
/// does not constrain it. Large enough that small servers never paginate.
const DEFAULT_PAGE_SIZE: usize = 100;

/// An MCP server bound to a [`ToolRegistry`].
pub struct McpServer {
    registry: ToolRegistry,
    server_name: String,
    page_size: usize,
}

impl McpServer {
    pub fn new(registry: ToolRegistry, server_name: impl Into<String>) -> Self {
        Self {
            registry,
            server_name: server_name.into(),
            page_size: DEFAULT_PAGE_SIZE,
        }
    }

    /// Override the `tools/list` page size (must be >= 1).
    pub fn with_page_size(mut self, n: usize) -> Self {
        self.page_size = n.max(1);
        self
    }

    /// Borrow the underlying registry (e.g. to add tools after construction
    /// requires rebuilding; this is read-only access for inspection).
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Handle one JSON-RPC message. Returns `Some(response)` for requests
    /// and `None` for notifications (messages without an `id`).
    pub fn handle(&self, req: &Value) -> Option<Value> {
        // A notification has no `id`. Validate the envelope first, but a
        // missing-`id` *and* missing-`jsonrpc` message is still a
        // notification we silently drop.
        let id = req.get("id").cloned();
        let is_notification = id.is_none();

        if req.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            if is_notification {
                return None;
            }
            return Some(error_response(
                id,
                ToolError::Protocol("missing or invalid `jsonrpc` field".into()),
            ));
        }

        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        if is_notification {
            // Notifications produce no response. We accept and drop the
            // standard lifecycle notifications (initialized, cancelled, …).
            return None;
        }

        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list(&params)),
            "tools/call" => return Some(self.tools_call(id, &params)),
            other => Err(ToolError::NotFound(other.to_string())),
        };

        match result {
            Ok(v) => Some(success_response(id, v)),
            Err(e) => Some(error_response(id, e)),
        }
    }

    fn initialize(&self, _params: &Value) -> Value {
        json!({
            "protocolVersion": crate::MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false }
            },
            "serverInfo": {
                "name": self.server_name,
                "version": crate::SERVER_VERSION
            }
        })
    }

    fn tools_list(&self, params: &Value) -> Value {
        let specs = self.registry.list_specs();
        let start: usize = params
            .get("cursor")
            .and_then(Value::as_str)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let end = (start + self.page_size).min(specs.len());
        let page = &specs[start.min(specs.len())..end];
        let tools: Vec<Value> = page
            .iter()
            .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
            .collect();
        let mut result = json!({ "tools": tools });
        if end < specs.len() {
            result["nextCursor"] = Value::String(end.to_string());
        }
        result
    }

    fn tools_call(&self, id: Option<Value>, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        match self.registry.invoke_validated(name, &args) {
            Ok(res) => success_response(id, serde_json::to_value(res).unwrap_or(Value::Null)),
            // Tool-execution / sandbox failures: surface to the model as a
            // result with isError:true (per MCP tool-error semantics).
            Err(e @ (ToolError::Execution { .. } | ToolError::Sandbox { .. })) => {
                let res = crate::tool::ToolResult::error(e.to_string());
                success_response(id, serde_json::to_value(res).unwrap_or(Value::Null))
            }
            // Protocol-level failures: JSON-RPC error response.
            Err(e) => error_response(id, e),
        }
    }
}

fn success_response(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn error_response(id: Option<Value>, e: ToolError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": e.json_rpc_code(),
            "message": e.to_string(),
            "data": { "code": e.code() }
        }
    })
}
