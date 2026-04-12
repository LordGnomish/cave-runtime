//! MCP server registry and tool execution bridge.

use crate::models::McpProvider;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

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
pub struct McpRegistry {
    providers: HashMap<Uuid, McpProvider>,
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new MCP provider.
    pub fn register(&mut self, provider: McpProvider) {
        info!(
            provider_id = %provider.id,
            name = %provider.name,
            "Registered MCP provider"
        );
        self.providers.insert(provider.id, provider);
    }

    /// Remove a provider by ID.
    pub fn deregister(&mut self, id: Uuid) {
        self.providers.remove(&id);
    }

    /// List all registered providers.
    pub fn list(&self) -> Vec<&McpProvider> {
        self.providers.values().collect()
    }

    /// Find a provider by cloud provider name (e.g. "aws").
    pub fn find_by_provider(&self, provider: &str) -> Option<&McpProvider> {
        self.providers
            .values()
            .find(|p| p.provider.eq_ignore_ascii_case(provider) && p.healthy)
    }

    /// Find a provider that supports a given tool.
    pub fn find_by_tool(&self, tool: &str) -> Option<&McpProvider> {
        self.providers
            .values()
            .find(|p| p.tools.iter().any(|t| t == tool) && p.healthy)
    }

    /// Mark a provider healthy or unhealthy.
    pub fn set_health(&mut self, id: Uuid, healthy: bool) {
        if let Some(p) = self.providers.get_mut(&id) {
            p.healthy = healthy;
            p.last_health_check = Some(Utc::now());
        }
    }
}

/// Execute a single MCP tool call against a registered provider.
///
/// In production this sends a JSON-RPC request to the MCP server endpoint.
/// Here we simulate the call and return a structured result.
pub async fn execute_tool(
    registry: Arc<Mutex<McpRegistry>>,
    tool: &str,
    params: &HashMap<String, serde_json::Value>,
) -> McpToolResult {
    let reg = registry.lock().await;
    let provider = match reg.find_by_tool(tool) {
        Some(p) => p.clone(),
        None => {
            warn!(tool = tool, "No healthy MCP provider found for tool");
            return McpToolResult {
                tool: tool.to_string(),
                provider_id: Uuid::nil(),
                success: false,
                output: serde_json::Value::Null,
                error: Some(format!("No healthy provider registered for tool '{}'", tool)),
            };
        }
    };
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
        "params": params,
        "result": "simulated_success",
        "remote_id": format!("sim-{}", Uuid::new_v4()),
    });

    McpToolResult {
        tool: tool.to_string(),
        provider_id: provider.id,
        success: true,
        output: simulated_output,
        error: None,
    }
}

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
}

/// Health-check a single MCP provider endpoint.
pub async fn health_check(endpoint: &str) -> bool {
    // Real impl: GET {endpoint}/health → 200 OK.
    // Simulated: always healthy.
    info!(endpoint = endpoint, "Health-checking MCP provider");
    true
}

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
        }
        let mut results = Vec::new();
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
            }
        }
        results
    } else {
        let mut results = Vec::new();
        for (tool, params) in calls {
            results.push(execute_tool(Arc::clone(&registry), &tool, &params).await);
        }
        results
    }
}
