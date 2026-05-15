// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Unified error types for the CAVE Runtime.

use thiserror::Error;

pub type CaveResult<T> = Result<T, CaveError>;

#[derive(Error, Debug)]
pub enum CaveError {
    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Authorization denied: {0}")]
    Forbidden(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Module {module} is not enabled")]
    ModuleDisabled { module: String },

    #[error("Upstream sync error for {project}: {message}")]
    UpstreamSync { project: String, message: String },

    #[error("Internal error: {0}")]
    Internal(String),
}

impl CaveError {
    /// HTTP status code for this error
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Auth(_) => 401,
            Self::Forbidden(_) => 403,
            Self::NotFound(_) => 404,
            Self::Validation(_) => 422,
            Self::ModuleDisabled { .. } => 503,
            _ => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_error_is_401() {
        let err = CaveError::Auth("bad token".to_string());
        assert_eq!(err.status_code(), 401);
    }

    #[test]
    fn test_forbidden_is_403() {
        let err = CaveError::Forbidden("access denied".to_string());
        assert_eq!(err.status_code(), 403);
    }

    #[test]
    fn test_not_found_is_404() {
        let err = CaveError::NotFound("resource missing".to_string());
        assert_eq!(err.status_code(), 404);
    }

    #[test]
    fn test_validation_is_422() {
        let err = CaveError::Validation("invalid input".to_string());
        assert_eq!(err.status_code(), 422);
    }

    #[test]
    fn test_module_disabled_is_503() {
        let err = CaveError::ModuleDisabled { module: "cave-flags".to_string() };
        assert_eq!(err.status_code(), 503);
    }
}
