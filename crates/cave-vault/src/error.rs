//! Vault error types.

use axum::{http::StatusCode, response::IntoResponse, Json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault is sealed")]
    Sealed,
    #[error("vault not initialized")]
    NotInitialized,
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid token")]
    InvalidToken,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("crypto error: {0}")]
    CryptoError(String),
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error("lease expired")]
    LeaseExpired,
    #[error("secret deleted")]
    SecretDeleted,
    #[error("secret destroyed")]
    SecretDestroyed,
    #[error("already initialized")]
    AlreadyInitialized,
    #[error("internal error: {0}")]
    Internal(String),
}

impl VaultError {
    pub fn http_status(&self) -> StatusCode {
        match self {
            VaultError::Sealed => StatusCode::SERVICE_UNAVAILABLE,
            VaultError::NotInitialized => StatusCode::INTERNAL_SERVER_ERROR,
            VaultError::PermissionDenied(_) => StatusCode::FORBIDDEN,
            VaultError::NotFound(_) => StatusCode::NOT_FOUND,
            VaultError::InvalidToken => StatusCode::FORBIDDEN,
            VaultError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            VaultError::CryptoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            VaultError::KeyNotFound(_) => StatusCode::NOT_FOUND,
            VaultError::LeaseExpired => StatusCode::FORBIDDEN,
            VaultError::SecretDeleted => StatusCode::NOT_FOUND,
            VaultError::SecretDestroyed => StatusCode::NOT_FOUND,
            VaultError::AlreadyInitialized => StatusCode::BAD_REQUEST,
            VaultError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for VaultError {
    fn into_response(self) -> axum::response::Response {
        let status = self.http_status();
        let body = serde_json::json!({ "errors": [self.to_string()] });
        (status, Json(body)).into_response()
    }
}
