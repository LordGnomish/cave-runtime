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
//! MCP server registry and tool execution bridge.
use crate::models::McpProvider;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
/// Response from an MCP tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub tool: String,
    pub provider_id: Uuid,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
}
/// Registry of connected MCP providers.
#[derive(Debug, Default)]
    providers: HashMap<Uuid, McpProvider>,
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
    /// Register a new MCP provider.
    pub fn register(&mut self, provider: McpProvider) {
        info!(
            provider_id = %provider.id,
            name = %provider.name,
            "Registered MCP provider"
        );
        self.providers.insert(provider.id, provider);
    /// Remove a provider by ID.
    pub fn deregister(&mut self, id: Uuid) {
        self.providers.remove(&id);
    /// List all registered providers.
    pub fn list(&self) -> Vec<&McpProvider> {
        self.providers.values().collect()
    /// Find a provider by cloud provider name (e.g. "aws").
    pub fn find_by_provider(&self, provider: &str) -> Option<&McpProvider> {
        self.providers
            .values()
            .find(|p| p.provider.eq_ignore_ascii_case(provider) && p.healthy)
    /// Find a provider that supports a given tool.
    pub fn find_by_tool(&self, tool: &str) -> Option<&McpProvider> {
        self.providers
            .values()
            .find(|p| p.tools.iter().any(|t| t == tool) && p.healthy)
    /// Mark a provider healthy or unhealthy.
    pub fn set_health(&mut self, id: Uuid, healthy: bool) {
        if let Some(p) = self.providers.get_mut(&id) {
            p.healthy = healthy;
            p.last_health_check = Some(Utc::now());
/// Execute a single MCP tool call against a registered provider.
/// In production this sends a JSON-RPC request to the MCP server endpoint.
/// Here we simulate the call and return a structured result.
    registry: Arc<Mutex<McpRegistry>>,
    tool: &str,
) -> McpToolResult {
    let reg = registry.lock().await;
    let provider = match reg.find_by_tool(tool) {
        Some(p) => p.clone(),
            warn!(tool = tool, "No healthy MCP provider found for tool");
            return McpToolResult {
                tool: tool.to_string(),
                provider_id: Uuid::nil(),
                success: false,
                output: serde_json::Value::Null,
                error: Some(format!("No healthy provider registered for tool '{}'", tool)),
    drop(reg);
    info!(
        tool = tool,
        provider = %provider.name,
        endpoint = %provider.endpoint,
        "Invoking MCP tool"
    );
    // Simulate MCP JSON-RPC call.
    // Real impl: reqwest::Client::new().post(&provider.endpoint).json(&payload).send().await
    let simulated_output = serde_json::json!({
        "tool": tool,
        "provider": provider.name,
        "result": "simulated_success",
        "remote_id": format!("sim-{}", Uuid::new_v4()),
    McpToolResult {
        tool: tool.to_string(),
        provider_id: provider.id,
        success: true,
        output: simulated_output,
        error: None,
/// Discover capabilities from an MCP server by fetching its tool manifest.
pub async fn discover_capabilities(endpoint: &str) -> Result<(Vec<String>, Vec<String>)> {
    info!(endpoint = endpoint, "Discovering MCP provider capabilities");
    // Real impl: GET {endpoint}/tools → parse JSON tool list.
    // Simulated: return well-known AWS tools.
    let tools = vec![
        "aws_create_s3_bucket".to_string(),
        "aws_delete_s3_bucket".to_string(),
        "aws_create_ec2_instance".to_string(),
        "aws_delete_ec2_instance".to_string(),
        "aws_create_rds_instance".to_string(),
        "aws_create_iam_role".to_string(),
        "aws_create_vpc".to_string(),
        "aws_create_subnet".to_string(),
        "aws_create_security_group".to_string(),
        "aws_create_load_balancer".to_string(),
    ];
    let capabilities = vec![
        "s3_bucket".to_string(),
        "ec2_instance".to_string(),
        "rds_instance".to_string(),
        "iam_role".to_string(),
        "vpc".to_string(),
        "subnet".to_string(),
        "security_group".to_string(),
        "load_balancer".to_string(),
    ];
    Ok((tools, capabilities))
/// Health-check a single MCP provider endpoint.
pub async fn health_check(endpoint: &str) -> bool {
    // Real impl: GET {endpoint}/health → 200 OK.
    // Simulated: always healthy.
    info!(endpoint = endpoint, "Health-checking MCP provider");
/// Execute multiple tool calls, respecting parallelism where `parallel = true`.
pub async fn batch_execute(
    registry: Arc<Mutex<McpRegistry>>,
    calls: Vec<(String, HashMap<String, serde_json::Value>)>,
    parallel: bool,
) -> Vec<McpToolResult> {
    if parallel {
        let mut handles = Vec::new();
        for (tool, params) in calls {
            let reg = Arc::clone(&registry);
            handles.push(tokio::spawn(async move {
                execute_tool(reg, &tool, &params).await
            }));
        for h in handles {
            match h.await {
                Ok(r) => results.push(r),
                Err(e) => results.push(McpToolResult {
                    tool: "unknown".into(),
                    provider_id: Uuid::nil(),
                    success: false,
                    output: serde_json::Value::Null,
                    error: Some(format!("Task join error: {}", e)),
                }),
        results
    } else {
        for (tool, params) in calls {
            results.push(execute_tool(Arc::clone(&registry), &tool, &params).await);
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
