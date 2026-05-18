// SPDX-License-Identifier: AGPL-3.0-or-later
//! MCP Provider Bridge — register, discover, execute, health-check MCP servers.
//!
//! MCP (Model Context Protocol) servers provide the actual cloud-provider
//! integrations: AWS, Azure, GCP, Hetzner, Kubernetes. This bridge handles
//! server lifecycle and tool execution on their behalf.

use crate::models::{McpProvider, McpTool};
use chrono::Utc;
use std::collections::HashMap;
use tracing::{info, warn};
use uuid::Uuid;

/// Registry of connected MCP servers.
#[derive(Default)]
pub struct McpRegistry {
    pub providers: Vec<McpProvider>,
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new MCP server and pre-populate its capabilities.
    pub fn register(&mut self, name: String, endpoint: String) -> McpProvider {
        let provider = McpProvider {
            id: Uuid::new_v4(),
            name: name.clone(),
            endpoint: endpoint.clone(),
            capabilities: default_capabilities(&name),
            healthy: true,
            registered_at: Utc::now(),
        };
        info!(provider = %name, endpoint = %endpoint, "MCP provider registered");
        self.providers.push(provider.clone());
        provider
    }

    /// Call a tool on a provider. Returns the tool's JSON output.
    ///
    /// In production this makes an HTTP (or stdio) call to the MCP server.
    /// Here we return a simulated success response.
    pub async fn execute_tool(
        &self,
        provider_name: &str,
        tool_name: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, McpError> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.name == provider_name)
            .ok_or_else(|| McpError::ProviderNotFound(provider_name.to_string()))?;

        if !provider.healthy {
            return Err(McpError::ProviderUnhealthy(provider_name.to_string()));
        }

        if !provider.capabilities.iter().any(|t| t.name == tool_name) {
            return Err(McpError::ToolNotFound {
                provider: provider_name.to_string(),
                tool: tool_name.to_string(),
            });
        }

        info!(provider = %provider_name, tool = %tool_name, "Executing MCP tool");

        Ok(serde_json::json!({
            "status": "ok",
            "provider": provider_name,
            "tool": tool_name,
            "params": params,
            "resource_id": Uuid::new_v4().to_string(),
        }))
    }

    /// Query an MCP server for its available tools and update the registry.
    pub async fn discover_capabilities(
        &mut self,
        provider_name: &str,
    ) -> Result<Vec<McpTool>, McpError> {
        let provider = self
            .providers
            .iter_mut()
            .find(|p| p.name == provider_name)
            .ok_or_else(|| McpError::ProviderNotFound(provider_name.to_string()))?;

        // In production: GET {endpoint}/tools/list
        let caps = default_capabilities(provider_name);
        provider.capabilities = caps.clone();
        info!(provider = %provider_name, tools = caps.len(), "MCP capabilities discovered");
        Ok(caps)
    }

    /// Ping a provider and update its health flag.
    pub async fn health_check(&mut self, provider_name: &str) -> bool {
        match self.providers.iter_mut().find(|p| p.name == provider_name) {
            Some(p) => {
                // In production: GET {endpoint}/health
                p.healthy = true;
                info!(provider = %provider_name, "MCP health check: ok");
                true
            }
            None => {
                warn!(provider = %provider_name, "MCP health check: provider not found");
                false
            }
        }
    }

    /// Execute multiple MCP calls in dependency order.
    pub async fn batch_execute(&self, calls: Vec<McpCall>) -> Vec<McpCallResult> {
        let mut results = Vec::new();
        for call in calls {
            let result = self
                .execute_tool(&call.provider, &call.tool, &call.params)
                .await;
            results.push(McpCallResult {
                call_id: call.id,
                provider: call.provider,
                tool: call.tool,
                result,
            });
        }
        results
    }
}

#[derive(Debug, Clone)]
pub struct McpCall {
    pub id: Uuid,
    pub provider: String,
    pub tool: String,
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug)]
pub struct McpCallResult {
    pub call_id: Uuid,
    pub provider: String,
    pub tool: String,
    pub result: Result<serde_json::Value, McpError>,
}

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("provider unhealthy: {0}")]
    ProviderUnhealthy(String),
    #[error("tool '{tool}' not found on provider '{provider}'")]
    ToolNotFound { provider: String, tool: String },
    #[error("execution error: {0}")]
    ExecutionError(String),
}

fn tool(name: &str, description: &str) -> McpTool {
    McpTool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

fn default_capabilities(provider_name: &str) -> Vec<McpTool> {
    match provider_name {
        "aws" => vec![
            tool("create_rds_cluster", "Create an RDS PostgreSQL cluster"),
            tool("delete_rds_cluster", "Delete an RDS cluster"),
            tool("create_object_storage", "Create an S3 bucket"),
            tool("delete_object_storage", "Delete an S3 bucket"),
            tool("create_virtual_machine", "Launch an EC2 instance"),
            tool("delete_virtual_machine", "Terminate an EC2 instance"),
            tool("update_virtual_machine", "Modify an EC2 instance"),
        ],
        "azure" => vec![
            tool("create_virtual_machine", "Create an Azure VM"),
            tool("delete_virtual_machine", "Delete an Azure VM"),
            tool("create_object_storage", "Create Azure Blob Storage"),
            tool("delete_object_storage", "Delete Azure Blob Storage"),
        ],
        "gcp" => vec![
            tool("create_virtual_machine", "Create a GCE instance"),
            tool("delete_virtual_machine", "Delete a GCE instance"),
            tool("create_kubernetes_cluster", "Create a GKE cluster"),
            tool("delete_kubernetes_cluster", "Delete a GKE cluster"),
            tool("create_object_storage", "Create a GCS bucket"),
        ],
        "kubernetes" => vec![
            tool("create_kubernetes_cluster", "Create a Kubernetes cluster"),
            tool("delete_kubernetes_cluster", "Delete a Kubernetes cluster"),
        ],
        "hetzner" => vec![
            tool("create_virtual_machine", "Create a Hetzner server"),
            tool("delete_virtual_machine", "Delete a Hetzner server"),
            tool("update_virtual_machine", "Resize a Hetzner server"),
        ],
        _ => vec![],
    }
}
