// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-infra.

use thiserror::Error;

pub type InfraResult<T> = Result<T, InfraError>;

#[derive(Error, Debug)]
pub enum InfraError {
    #[error("Resource not found: {kind}/{name}")]
    NotFound { kind: String, name: String },

    #[error("Resource already exists: {kind}/{name}")]
    AlreadyExists { kind: String, name: String },

    #[error("Invalid resource spec: {0}")]
    InvalidSpec(String),

    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Provider error ({provider}): {message}")]
    ProviderError { provider: String, message: String },

    #[error("Dependency cycle detected: {0}")]
    DependencyCycle(String),

    #[error("Dependency not met: resource={resource}, depends_on={depends_on}")]
    DependencyNotMet { resource: String, depends_on: String },

    #[error("Plan conflict: {0}")]
    PlanConflict(String),

    #[error("Rollback failed: {0}")]
    RollbackFailed(String),

    #[error("Drift detected: resource={resource}, field={field}, expected={expected}, actual={actual}")]
    DriftDetected {
        resource: String,
        field: String,
        expected: String,
        actual: String,
    },

    #[error("Template not found: {0}")]
    TemplateNotFound(String),

    #[error("Template render error: {template}: {message}")]
    TemplateRenderError { template: String, message: String },

    #[error("NLP intent parse error: {0}")]
    IntentParseError(String),

    #[error("MCP protocol error: {0}")]
    McpProtocolError(String),

    #[error("MCP tool not found: {0}")]
    McpToolNotFound(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
