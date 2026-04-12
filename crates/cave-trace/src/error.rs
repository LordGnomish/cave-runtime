#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("trace not found: {0}")]
    TraceNotFound(String),
    #[error("span not found: {0}")]
    SpanNotFound(String),
    #[error("invalid trace id: {0}")]
    InvalidTraceId(String),
    #[error("storage error: {0}")]
    StorageError(String),
    #[error("query error: {0}")]
    QueryError(String),
    #[error("otlp parse error: {0}")]
    OtlpError(String),
}

pub type TraceResult<T> = Result<T, TraceError>;
