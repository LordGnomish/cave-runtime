//! Error types for cave-policy.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("policy not found: {0}")]
    NotFound(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("evaluation error: {0}")]
    Eval(String),

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("mutation error: {0}")]
    Mutation(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("bundle error: {0}")]
    Bundle(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("crypto error: {0}")]
    Crypto(String),
}

impl PolicyError {
    pub fn http_status(&self) -> u16 {
        match self {
            PolicyError::NotFound(_) => 404,
            PolicyError::InvalidRequest(_) => 400,
            PolicyError::Conflict(_) => 409,
            PolicyError::Unauthorized(_) => 401,
            PolicyError::Unsupported(_) => 501,
            _ => 500,
        }
    }
}

pub type PolicyResult<T> = Result<T, PolicyError>;

impl From<serde_json::Error> for PolicyError {
    fn from(e: serde_json::Error) -> Self {
        PolicyError::Serialization(e.to_string())
    }
}

impl From<serde_yaml::Error> for PolicyError {
    fn from(e: serde_yaml::Error) -> Self {
        PolicyError::Serialization(e.to_string())
    }
}
