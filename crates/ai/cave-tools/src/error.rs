// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error taxonomy for the tool-calling framework.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, ToolError>;

/// Failure modes surfaced by the framework. Every variant maps to a stable
/// machine-readable code via [`ToolError::code`] so MCP / OpenAI adapters can
/// translate it onto their respective error envelopes.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// No tool is registered under the requested name.
    #[error("tool not found: {0}")]
    NotFound(String),

    /// The caller-supplied arguments failed JSON Schema validation.
    #[error("invalid arguments for `{tool}`: {reason}")]
    InvalidArguments { tool: String, reason: String },

    /// The caller lacks permission to invoke this tool.
    #[error("permission denied for `{tool}`: {reason}")]
    PermissionDenied { tool: String, reason: String },

    /// The tool's handler reported a failure (business-logic error).
    #[error("tool `{tool}` failed: {reason}")]
    Execution { tool: String, reason: String },

    /// A sandbox boundary (path, host, resource limit) was violated.
    #[error("sandbox violation in `{tool}`: {reason}")]
    Sandbox { tool: String, reason: String },

    /// The JSON-RPC envelope was malformed (MCP transport layer).
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl ToolError {
    /// Stable error code. MCP maps these onto JSON-RPC error numbers; the
    /// codes themselves are framework-defined and version-stable.
    pub fn code(&self) -> &'static str {
        match self {
            ToolError::NotFound(_) => "tool_not_found",
            ToolError::InvalidArguments { .. } => "invalid_arguments",
            ToolError::PermissionDenied { .. } => "permission_denied",
            ToolError::Execution { .. } => "execution_error",
            ToolError::Sandbox { .. } => "sandbox_violation",
            ToolError::Protocol(_) => "protocol_error",
        }
    }

    /// JSON-RPC 2.0 error number used by the MCP server layer.
    /// Application errors use the reserved -32000..=-32099 range; malformed
    /// envelopes use the spec-defined -32600 (Invalid Request).
    pub fn json_rpc_code(&self) -> i64 {
        match self {
            ToolError::NotFound(_) => -32601, // Method not found
            ToolError::InvalidArguments { .. } => -32602, // Invalid params
            ToolError::PermissionDenied { .. } => -32001,
            ToolError::Execution { .. } => -32000,
            ToolError::Sandbox { .. } => -32002,
            ToolError::Protocol(_) => -32600, // Invalid Request
        }
    }
}
