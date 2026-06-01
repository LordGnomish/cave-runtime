// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types shared across the cave-agent runtime.

/// Errors surfaced by the agent primitives and the self-improvement loop.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentError {
    /// A tool was invoked by a name that is not present in the registry.
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    /// A tool rejected its arguments (missing field, wrong type, domain error).
    #[error("invalid arguments for tool {tool}: {reason}")]
    InvalidArguments { tool: String, reason: String },

    /// A tool ran but failed at execution time.
    #[error("tool {tool} failed: {reason}")]
    ToolFailed { tool: String, reason: String },

    /// A plan step referenced a tool/output that could not be resolved.
    #[error("plan error: {0}")]
    Plan(String),

    /// A hot-patch failed validation (checksum mismatch, unknown target, ...).
    #[error("patch rejected: {0}")]
    PatchRejected(String),

    /// A changelog or version string could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, AgentError>;
