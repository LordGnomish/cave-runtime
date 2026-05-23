// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gateway error model.
//!
//! Maps Kong's `kong.response.exit()` semantics + Envoy's `ResponseFlags`
//! into a single Rust error enum.

use thiserror::Error;

pub type AGwResult<T> = Result<T, AGwError>;

#[derive(Debug, Error)]
pub enum AGwError {
    #[error("route not found: {0}")]
    RouteNotFound(String),

    #[error("service not found: {0}")]
    ServiceNotFound(String),

    #[error("upstream not found: {0}")]
    UpstreamNotFound(String),

    #[error("consumer not found: {0}")]
    ConsumerNotFound(String),

    #[error("plugin not found: {0}")]
    PluginNotFound(String),

    #[error("plugin error [{plugin}]: {reason}")]
    Plugin { plugin: String, reason: String },

    #[error("authentication failed: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("rate limited: retry after {retry_after_s}s")]
    RateLimited { retry_after_s: u32 },

    #[error("upstream unhealthy: {0}")]
    UpstreamUnhealthy(String),

    #[error("upstream timeout after {ms}ms")]
    UpstreamTimeout { ms: u64 },

    #[error("circuit open for {service}")]
    CircuitOpen { service: String },

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("internal: {0}")]
    Internal(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

impl AGwError {
    /// HTTP status corresponding to this error — mirrors Kong's exit codes.
    pub fn http_status(&self) -> u16 {
        match self {
            AGwError::RouteNotFound(_)
            | AGwError::ServiceNotFound(_)
            | AGwError::UpstreamNotFound(_)
            | AGwError::ConsumerNotFound(_)
            | AGwError::PluginNotFound(_) => 404,
            AGwError::Unauthorized(_) => 401,
            AGwError::Forbidden(_) => 403,
            AGwError::RateLimited { .. } => 429,
            AGwError::UpstreamUnhealthy(_) | AGwError::CircuitOpen { .. } => 503,
            AGwError::UpstreamTimeout { .. } => 504,
            AGwError::BadRequest(_) | AGwError::Plugin { .. } => 400,
            AGwError::Conflict(_) => 409,
            AGwError::NotImplemented(_) => 501,
            AGwError::Internal(_) | AGwError::Io(_) | AGwError::Json(_) | AGwError::Yaml(_) => 500,
        }
    }

    /// Whether the request can safely be retried at a different upstream target.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            AGwError::UpstreamUnhealthy(_)
                | AGwError::UpstreamTimeout { .. }
                | AGwError::CircuitOpen { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_mappings() {
        assert_eq!(AGwError::RouteNotFound("x".into()).http_status(), 404);
        assert_eq!(AGwError::Unauthorized("x".into()).http_status(), 401);
        assert_eq!(AGwError::Forbidden("x".into()).http_status(), 403);
        assert_eq!(AGwError::RateLimited { retry_after_s: 5 }.http_status(), 429);
        assert_eq!(AGwError::UpstreamUnhealthy("x".into()).http_status(), 503);
        assert_eq!(AGwError::UpstreamTimeout { ms: 100 }.http_status(), 504);
        assert_eq!(AGwError::BadRequest("x".into()).http_status(), 400);
        assert_eq!(AGwError::Conflict("x".into()).http_status(), 409);
        assert_eq!(AGwError::NotImplemented("x".into()).http_status(), 501);
        assert_eq!(AGwError::Internal("x".into()).http_status(), 500);
    }

    #[test]
    fn retryable_flag() {
        assert!(AGwError::UpstreamUnhealthy("x".into()).is_retryable());
        assert!(AGwError::UpstreamTimeout { ms: 1 }.is_retryable());
        assert!(AGwError::CircuitOpen { service: "x".into() }.is_retryable());
        assert!(!AGwError::Unauthorized("x".into()).is_retryable());
        assert!(!AGwError::Forbidden("x".into()).is_retryable());
    }
}
