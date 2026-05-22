// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-llm-tracker.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrackerError {
    #[error("config: {0}")]
    Config(String),

    #[error("registry source {source_slug}: {reason}")]
    RegistrySource { source_slug: String, reason: String },

    #[error("bench: {0}")]
    Bench(String),

    #[error("report: {0}")]
    Report(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml-de: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("toml-ser: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("http: {0}")]
    Http(String),
}

pub type TrackerResult<T> = Result<T, TrackerError>;
