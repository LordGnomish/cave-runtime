// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Top-level error type for cave-datafusion.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("plan error: {0}")]
    Plan(String),

    #[error("execution error: {0}")]
    Execution(String),

    #[error("schema error: {0}")]
    Schema(String),

    #[error("sql parse error: {0}")]
    SqlParse(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("type mismatch: {0}")]
    TypeMismatch(String),

    #[error("io error: {0}")]
    Io(String),
}
