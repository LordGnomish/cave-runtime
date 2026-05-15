//! Unified error type for cave-mesh.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MeshError {
    #[error("service not found: {0}")]
    ServiceNotFound(String),

    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("circuit breaker open for: {0}")]
    CircuitOpen(String),

    #[error("mTLS rejected: {0}")]
    MtlsRejected(String),

    #[error("authorization denied: {0}")]
    AuthzDenied(String),

    #[error("JWT error: {0}")]
    Jwt(String),

    #[error("rate limit exceeded for: {0}")]
    RateLimited(String),

    #[error("fault injection abort: HTTP {0}")]
    FaultAbort(u16),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("xDS error: {0}")]
    Xds(String),

    #[error("SPIFFE / certificate error: {0}")]
    Spiffe(String),

    #[error("telemetry configuration error: {0}")]
    Telemetry(String),

    #[error("multi-cluster error: {0}")]
    MultiCluster(String),

    #[error("EnvoyFilter patch error: {0}")]
    EnvoyFilter(String),

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl MeshError {
    pub fn not_found(s: impl Into<String>) -> Self {
        Self::NotFound(s.into())
    }
    pub fn conflict(s: impl Into<String>) -> Self {
        Self::Conflict(s.into())
    }
    pub fn invalid_input(s: impl Into<String>) -> Self {
        Self::InvalidInput(s.into())
    }
}

pub type MeshResult<T> = Result<T, MeshError>;
