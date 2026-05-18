// SPDX-License-Identifier: AGPL-3.0-or-later
//! MCP (Model Context Protocol) tool server for infrastructure actions.
//!
//! Implements the JSON-RPC 2.0 protocol used by MCP:
//!   POST /mcp  →  { jsonrpc: "2.0", method: "tools/call", params: { name, arguments } }

use crate::error::{InfraError, InfraResult};
use crate::resource::{ResourceKind, ResourceSpec, ResourceState, ResourceStore};
use crate::plan::generate_plan;
use crate::provider::ProviderRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// ── JSON-RPC types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    pub fn err(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message, data: None }),
        }
    }
}

// ── MCP tool definition ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub fn all_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "infra_list_resources".into(),
            description: "List all infrastructure resources and their current status".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "description": "Optional resource kind filter"}
                }
            }),
        },
        McpTool {
            name: "infra_get_resource".into(),
            description: "Get details of a specific infrastructure resource".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind", "name"],
                "properties": {
                    "kind": {"type": "string"},
                    "name": {"type": "string"}
                }
            }),
        },
        McpTool {
            name: "infra_provision".into(),
            description: "Provision a new infrastructure resource".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind", "name", "provider"],
                "properties": {
                    "kind": {"type": "string"},
                    "name": {"type": "string"},
                    "provider": {"type": "string"},
                    "properties": {"type": "object"}
                }
            }),
        },
        McpTool {
            name: "infra_plan".into(),
            description: "Generate a change plan for a set of desired resources".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["resources"],
                "properties": {
                    "resources": {
                        "type": "array",
                        "items": {"type": "object"}
                    }
                }
            }),
        },
        McpTool {
            name: "infra_delete_resource".into(),
            description: "Delete an infrastructure resource".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind", "name"],
                "properties": {
                    "kind": {"type": "string"},
                    "name": {"type": "string"}
                }
            }),
        },
        McpTool {
            name: "infra_detect_drift".into(),
            description: "Detect drift between desired and actual resource state".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpTool {
            name: "infra_list_providers".into(),
            description: "List available infrastructure providers".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

// ── MCP server ────────────────────────────────────────────────────────────────

pub struct McpServer {
    store: Arc<ResourceStore>,
    registry: Arc<ProviderRegistry>,
}

impl McpServer {
    pub fn new(store: Arc<ResourceStore>, registry: Arc<ProviderRegistry>) -> Self {
        Self { store, registry }
    }

    pub async fn handle(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => JsonRpcResponse::ok(req.id, serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "cave-infra", "version": "1.0.0"}
            })),

            "tools/list" => JsonRpcResponse::ok(req.id, serde_json::json!({
                "tools": all_tools()
            })),

            "tools/call" => {
                let params = req.params.unwrap_or(Value::Null);
                let tool_name = params["name"].as_str().unwrap_or("").to_string();
                let args = &params["arguments"];
                match self.call_tool(&tool_name, args).await {
                    Ok(result) => JsonRpcResponse::ok(req.id, serde_json::json!({
                        "content": [{"type": "text", "text": result.to_string()}]
                    })),
                    Err(e) => JsonRpcResponse::err(req.id, -32603, e.to_string()),
                }
            }

            other => JsonRpcResponse::err(
                req.id,
                -32601,
                format!("method not found: {other}"),
            ),
        }
    }

    async fn call_tool(&self, name: &str, args: &Value) -> InfraResult<Value> {
        match name {
            "infra_list_resources" => {
                let resources = self.store.list();
                Ok(serde_json::json!(resources.iter().map(|r| serde_json::json!({
                    "key": r.key(),
                    "status": format!("{:?}", r.status),
                    "provider": r.spec.provider,
                    "provider_id": r.provider_id,
                })).collect::<Vec<_>>()))
            }

            "infra_get_resource" => {
                let kind_str = args["kind"].as_str().unwrap_or("");
                let name = args["name"].as_str().unwrap_or("");
                let key = format!("{kind_str}/{name}");
                let state = self.store.get(&key)?;
                Ok(serde_json::json!({
                    "key": state.key(),
                    "spec": state.spec,
                    "status": format!("{:?}", state.status),
                    "outputs": state.outputs,
                    "error": state.error,
                }))
            }

            "infra_provision" => {
                let kind_str = args["kind"].as_str().unwrap_or("Server");
                let res_name = args["name"].as_str().unwrap_or("").to_string();
                let provider = args["provider"].as_str().unwrap_or("noop").to_string();
                let properties: HashMap<String, Value> = args["properties"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                let spec = ResourceSpec {
                    kind: ResourceKind::from_str(kind_str),
                    name: res_name.clone(),
                    provider: provider.clone(),
                    properties,
                    depends_on: vec![],
                    tags: HashMap::new(),
                };

                let result = self.registry.create(&provider, &spec).await?;
                let mut state = ResourceState::new(spec);
                state.apply_actual(result.actual, Some(result.provider_id));
                let key = self.store.upsert(state);

                Ok(serde_json::json!({
                    "key": key,
                    "status": "running",
                    "message": format!("Resource {res_name} provisioned successfully")
                }))
            }

            "infra_plan" => {
                let resources_json = args["resources"].as_array().cloned().unwrap_or_default();
                let mut specs = Vec::new();
                for r in resources_json {
                    let props: HashMap<String, Value> = r["properties"]
                        .as_object()
                        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();
                    specs.push(ResourceSpec {
                        kind: ResourceKind::from_str(r["kind"].as_str().unwrap_or("Server")),
                        name: r["name"].as_str().unwrap_or("").to_string(),
                        provider: r["provider"].as_str().unwrap_or("noop").to_string(),
                        properties: props,
                        depends_on: vec![],
                        tags: HashMap::new(),
                    });
                }
                let plan = generate_plan(&specs, &self.store)?;
                Ok(serde_json::json!({
                    "plan_id": plan.id,
                    "summary": plan.summary,
                    "changes": plan.changes,
                }))
            }

            "infra_delete_resource" => {
                let kind_str = args["kind"].as_str().unwrap_or("");
                let res_name = args["name"].as_str().unwrap_or("");
                let key = format!("{kind_str}/{res_name}");
                self.store.delete(&key)?;
                Ok(serde_json::json!({
                    "message": format!("Resource {key} deleted")
                }))
            }

            "infra_detect_drift" => {
                let report = crate::drift::detect_drift(&self.store, &self.registry).await;
                Ok(serde_json::json!({
                    "total": report.total_resources,
                    "drifted": report.drifted.len(),
                    "healthy": report.healthy,
                    "unreachable": report.unreachable,
                    "drifted_resources": report.drifted,
                }))
            }

            "infra_list_providers" => {
                Ok(serde_json::json!(self.registry.list_names()))
            }

            other => Err(InfraError::McpToolNotFound(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> McpServer {
        let store = Arc::new(ResourceStore::new());
        let registry = Arc::new(ProviderRegistry::new());
        McpServer::new(store, registry)
    }

    fn rpc(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Value::Number(1.into()),
            method: method.to_string(),
            params: Some(params),
        }
    }

    #[tokio::test]
    async fn initialize() {
        let s = server();
        let resp = s.handle(rpc("initialize", Value::Null)).await;
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn tools_list() {
        let s = server();
        let resp = s.handle(rpc("tools/list", Value::Null)).await;
        let tools = &resp.result.unwrap()["tools"];
        assert!(tools.as_array().unwrap().len() >= 5);
    }

    #[tokio::test]
    async fn provision_resource() {
        let s = server();
        let resp = s.handle(rpc("tools/call", serde_json::json!({
            "name": "infra_provision",
            "arguments": {
                "kind": "Server",
                "name": "test-server",
                "provider": "noop",
                "properties": {"cpu": 4}
            }
        }))).await;
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn list_resources_empty() {
        let s = server();
        let resp = s.handle(rpc("tools/call", serde_json::json!({
            "name": "infra_list_resources",
            "arguments": {}
        }))).await;
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn unknown_method_returns_error() {
        let s = server();
        let resp = s.handle(rpc("unknown/method", Value::Null)).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}
