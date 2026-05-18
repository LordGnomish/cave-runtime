// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error types for cave-metrics.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("PromQL evaluation error: {0}")]
    Eval(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("ingestion error: {0}")]
    Ingestion(String),

    #[error("scrape error: {0}")]
    Scrape(String),

    #[error("alerting error: {0}")]
    Alerting(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("invalid label name: {0}")]
    InvalidLabel(String),

    #[error("type mismatch: {0}")]
    TypeMismatch(String),
}

pub type Result<T> = std::result::Result<T, MetricsError>;
