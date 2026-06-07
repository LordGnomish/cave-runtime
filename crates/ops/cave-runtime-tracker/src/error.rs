// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-runtime-tracker.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrackerError {
    #[error("config: {0}")]
    Config(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("http: {0}")]
    Http(String),
}

pub type TrackerResult<T> = Result<T, TrackerError>;
