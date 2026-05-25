// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

pub type VaultResult<T> = Result<T, VaultError>;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("Vault is sealed")]
    Sealed,
    #[error("Vault is already initialized")]
    AlreadyInitialized,
    #[error("Vault is not initialized")]
    NotInitialized,
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("bad token")]
    BadToken,
    #[error("token not found")]
    TokenNotFound,
    #[error("{0}")]
    Crypto(String),
    #[error("{0}")]
    Pki(String),
    #[error("{0}")]
    Internal(String),
    #[error("lease not found")]
    LeaseNotFound,
    #[error("lease expired")]
    LeaseExpired,
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error("mount not found: {0}")]
    MountNotFound(String),
    #[error("policy not found: {0}")]
    PolicyNotFound(String),
    #[error("role not found: {0}")]
    RoleNotFound(String),
    #[error("secret not found")]
    SecretNotFound,
    #[error("invalid mount type: {0}")]
    InvalidMountType(String),
    #[error("check-and-set failed")]
    CasFailed,
    #[error("wrapping token expired or not found")]
    WrapNotFound,
    #[error("{0}")]
    Auth(String),
}

impl VaultError {
    pub fn status(&self) -> StatusCode {
        match self {
            VaultError::PermissionDenied => StatusCode::FORBIDDEN,
            VaultError::BadToken | VaultError::TokenNotFound => StatusCode::FORBIDDEN,
            VaultError::NotFound(_)
            | VaultError::KeyNotFound(_)
            | VaultError::MountNotFound(_)
            | VaultError::PolicyNotFound(_)
            | VaultError::RoleNotFound(_)
            | VaultError::SecretNotFound
            | VaultError::LeaseNotFound
            | VaultError::WrapNotFound => StatusCode::NOT_FOUND,
            VaultError::Sealed | VaultError::NotInitialized => StatusCode::SERVICE_UNAVAILABLE,
            VaultError::AlreadyInitialized
            | VaultError::InvalidRequest(_)
            | VaultError::CasFailed => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for VaultError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(json!({ "errors": [self.to_string()] }));
        (status, body).into_response()
    }
}
