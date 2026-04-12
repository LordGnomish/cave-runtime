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
