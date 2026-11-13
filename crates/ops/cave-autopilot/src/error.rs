// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type for the autopilot daemon.

use thiserror::Error;

/// All fallible operations in the autopilot funnel through this.
#[derive(Debug, Error)]
pub enum AutopilotError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json (de)serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml parse error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("tracker state error: {0}")]
    Tracker(String),

    #[error("LLM backend error: {0}")]
    Llm(String),

    #[error("worktree operation failed: {0}")]
    Worktree(String),

    #[error("charter compliance violation: {0}")]
    Charter(String),

    #[error("config error: {0}")]
    Config(String),
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, AutopilotError>;
