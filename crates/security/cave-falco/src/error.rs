// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FalcoError {
    #[error("rule parse: {0}")]
    RuleParse(String),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("engine compile: {0}")]
    Compile(String),
    #[error("plugin: {0}")]
    Plugin(String),
    #[error("not-found: {0}")]
    NotFound(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, FalcoError>;
