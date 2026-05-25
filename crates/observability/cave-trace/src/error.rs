// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-trace.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraceError {
    #[error("trace not found: {0}")]
    NotFound(String),

    #[error("invalid trace ID '{0}': {1}")]
    InvalidTraceId(String, String),

    #[error("invalid span ID '{0}': {1}")]
    InvalidSpanId(String, String),

    #[error("parse error: {0}")]
    ParseError(String),

    #[error("storage error: {0}")]
    StorageError(String),

    #[error("query error: {0}")]
    QueryError(String),

    #[error("TraceQL syntax error at position {pos}: {msg}")]
    TraceQlSyntax { pos: usize, msg: String },

    #[error("TraceQL evaluation error: {0}")]
    TraceQlEval(String),

    #[error("tenant not found: {0}")]
    TenantNotFound(String),

    #[error("thrift decode error: {0}")]
    ThriftError(String),

    #[error("unsupported content type: {0}")]
    UnsupportedContentType(String),

    #[error("protobuf ingestion requires compiled protos — use HTTP/JSON variant")]
    ProtobufNotSupported,

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, TraceError>;
