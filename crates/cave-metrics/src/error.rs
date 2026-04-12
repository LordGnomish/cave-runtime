//! Error types for cave-metrics.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetricsError {
    #[error("TSDB error: {0}")]
    Tsdb(String),

    #[error("PromQL parse error: {0}")]
    Parse(String),

    #[error("PromQL eval error: {0}")]
    Eval(String),

    #[error("Scrape error: {0}")]
    Scrape(String),

    #[error("Remote write error: {0}")]
    RemoteWrite(String),

    #[error("Alertmanager error: {0}")]
    Alertmanager(String),

    #[error("WAL error: {0}")]
    Wal(String),

    #[error("Protobuf error: {0}")]
    Proto(String),

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type MetricsResult<T> = Result<T, MetricsError>;
