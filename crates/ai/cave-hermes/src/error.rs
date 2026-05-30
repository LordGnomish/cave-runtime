// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Crate-wide error type. Ports the *shape* of Hermes' loose Python
//! exceptions into a single explicit enum so callers can match on the
//! failure mode rather than parsing strings.

use thiserror::Error;

/// All recoverable failures surfaced by cave-hermes.
#[derive(Debug, Error)]
pub enum HermesError {
    #[error("memory backend error: {0}")]
    Memory(String),

    #[error("tool '{name}' not registered")]
    ToolNotFound { name: String },

    #[error("tool '{name}' failed: {reason}")]
    ToolFailed { name: String, reason: String },

    #[error("tool '{name}' rejected arguments: {reason}")]
    ToolArguments { name: String, reason: String },

    #[error("workflow checkpoint '{0}' not found")]
    CheckpointMissing(String),

    #[error("planner refused task: {0}")]
    PlannerRejected(String),

    #[error("model router has no models registered for tier {0:?}")]
    RouterEmpty(crate::router::ModelTier),

    #[error("rate-limited: retry after {seconds}s")]
    RateLimited { seconds: u64 },

    #[error("session log corrupted: {0}")]
    SessionCorrupted(String),

    #[error("recall index error: {0}")]
    Recall(String),

    #[error("agent loop failed: {0}")]
    AgentFailed(String),

    #[error("orchestration error: {0}")]
    Orchestration(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, HermesError>;
