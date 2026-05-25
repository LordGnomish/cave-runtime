// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-deploy.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeployError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("Kubernetes error: {0}")]
    Kubernetes(String),

    #[error("Manifest parse error: {0}")]
    ManifestParse(String),

    #[error("Sync failed: {0}")]
    SyncFailed(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Invalid: {0}")]
    Invalid(String),

    #[error("Notification error: {0}")]
    Notification(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<serde_json::Error> for DeployError {
    fn from(e: serde_json::Error) -> Self {
        DeployError::ManifestParse(e.to_string())
    }
}

impl From<serde_yaml::Error> for DeployError {
    fn from(e: serde_yaml::Error) -> Self {
        DeployError::ManifestParse(e.to_string())
    }
}

impl From<std::io::Error> for DeployError {
    fn from(e: std::io::Error) -> Self {
        DeployError::Git(e.to_string())
    }
}

impl From<reqwest::Error> for DeployError {
    fn from(e: reqwest::Error) -> Self {
        DeployError::Notification(e.to_string())
    }
}

impl IntoResponse for DeployError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            DeployError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            DeployError::AlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            DeployError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
            DeployError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            DeployError::Invalid(_) | DeployError::ManifestParse(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_render_distinct_status() {
        let not_found = DeployError::NotFound("app1".into()).into_response();
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
        let conflict = DeployError::AlreadyExists("dup".into()).into_response();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let forbidden = DeployError::Forbidden("nope".into()).into_response();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        let invalid = DeployError::Invalid("bad".into()).into_response();
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn from_json_error_yields_manifest_parse() {
        let bad = serde_json::from_str::<serde_json::Value>("{not json").unwrap_err();
        let err: DeployError = bad.into();
        assert!(matches!(err, DeployError::ManifestParse(_)));
    }
}
