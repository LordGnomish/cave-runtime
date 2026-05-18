// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error types for cave-datafusion.
//!
//! Mirrors the categories of apache/datafusion datafusion-common DataFusionError.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataFusionError {
    #[error("plan error: {0}")]
    Plan(String),
    #[error("expression error: {0}")]
    Expr(String),
    #[error("execution error: {0}")]
    Execution(String),
    #[error("schema error: column '{0}' not found")]
    ColumnNotFound(String),
    #[error("type mismatch: {0}")]
    TypeMismatch(String),
    #[error("invalid tenant_id: {0}")]
    InvalidTenant(String),
}

pub type DfResult<T> = Result<T, DataFusionError>;
