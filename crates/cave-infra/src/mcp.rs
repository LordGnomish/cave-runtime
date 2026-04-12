//! MCP bridge — executes infrastructure operations via the Model Context Protocol.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── McpToolCall ───────────────────────────────────────────────────────────────

/// A single MCP tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCall {
    pub id: Uuid,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub issued_at: DateTime<Utc>,
}

impl McpToolCall {
    pub fn new(tool_name: &str, parameters: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool_name: tool_name.to_string(),
            parameters,
            issued_at: Utc::now(),
        }
    }
}

// ── McpToolResult ─────────────────────────────────────────────────────────────

/// Result returned from an MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub call_id: Uuid,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub completed_at: DateTime<Utc>,
}

impl McpToolResult {
    pub fn success(call_id: Uuid, output: serde_json::Value, duration_ms: u64) -> Self {
        Self {
            call_id,
            success: true,
            output,
            error: None,
            duration_ms,
            completed_at: Utc::now(),
        }
    }

    pub fn failure(call_id: Uuid, error: &str, duration_ms: u64) -> Self {
        Self {
            call_id,
            success: false,
            output: serde_json::Value::Null,
            error: Some(error.to_string()),
            duration_ms,
            completed_at: Utc::now(),
        }
    }
}

// ── McpBridge trait ───────────────────────────────────────────────────────────

/// Abstraction over the MCP transport layer.
#[async_trait::async_trait]
pub trait McpBridge: Send + Sync {
    async fn call(&self, tool: McpToolCall) -> McpToolResult;
    async fn list_tools(&self) -> Vec<String>;
}

// ── MockMcpBridge ─────────────────────────────────────────────────────────────

/// Simulated MCP bridge for unit tests.
pub struct MockMcpBridge {
    /// Tools this bridge advertises.
    pub available_tools: Vec<String>,
    /// Tools that will return failures when called.
    pub fail_tools: HashSet<String>,
}

impl MockMcpBridge {
    /// Create a bridge where every call succeeds.
    pub fn new() -> Self {
        Self {
            available_tools: vec![
                "infra.create_vm".to_string(),
                "infra.create_vpc".to_string(),
                "infra.create_storage".to_string(),
                "infra.destroy".to_string(),
                "infra.describe".to_string(),
            ],
            fail_tools: HashSet::new(),
        }
    }

    /// Mark a specific tool as always failing.
    pub fn with_failing_tool(mut self, tool: &str) -> Self {
        self.fail_tools.insert(tool.to_string());
        self
    }
}

impl Default for MockMcpBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl McpBridge for MockMcpBridge {
    async fn call(&self, tool: McpToolCall) -> McpToolResult {
        // Simulate async I/O latency.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        if self.fail_tools.contains(&tool.tool_name) {
            McpToolResult::failure(tool.id, "simulated failure", 10)
        } else {
            McpToolResult::success(
                tool.id,
                serde_json::json!({
                    "status": "created",
                    "id": Uuid::new_v4().to_string()
                }),
                10,
            )
        }
    }

    async fn list_tools(&self) -> Vec<String> {
        self.available_tools.clone()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_bridge_success() {
        let bridge = MockMcpBridge::new();

        let tools = bridge.list_tools().await;
        assert!(!tools.is_empty());

        let call = McpToolCall::new(
            "infra.create_vm",
            serde_json::json!({ "cpu": 2, "ram": 4 }),
        );
        let call_id = call.id;
        let result = bridge.call(call).await;

        assert!(result.success);
        assert_eq!(result.call_id, call_id);
        assert!(result.error.is_none());
        assert!(result.output.get("status").is_some());
    }

    #[tokio::test]
    async fn test_mock_bridge_failure_for_specific_tool() {
        let bridge = MockMcpBridge::new().with_failing_tool("infra.create_vm");

        // The failing tool should fail.
        let bad_call = McpToolCall::new("infra.create_vm", serde_json::json!({}));
        let result = bridge.call(bad_call).await;
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("simulated failure"));

        // Other tools should still succeed.
        let ok_call = McpToolCall::new("infra.create_vpc", serde_json::json!({}));
        let ok_result = bridge.call(ok_call).await;
        assert!(ok_result.success);
    }
}
