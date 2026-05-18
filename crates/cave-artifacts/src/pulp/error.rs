// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/exceptions/__init__.py
//! Error types for cave-artifacts.

use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum ArtifactsError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("plugin '{0}' not registered")]
    PluginNotFound(String),

    #[error("task {0} failed: {1}")]
    TaskFailed(Uuid, String),

    #[error("sync error: {0}")]
    SyncError(String),

    #[error("publication error: {0}")]
    PublicationError(String),

    #[error("content guard denied: {0}")]
    ContentGuardDenied(String),

    #[error("export error: {0}")]
    ExportError(String),

    #[error("signing error: {0}")]
    SigningError(String),

    #[error("upstream error: {0}")]
    UpstreamError(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl ArtifactsError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NotFound(_) => 404,
            Self::AlreadyExists(_) => 409,
            Self::InvalidRequest(_) => 400,
            Self::PluginNotFound(_) => 400,
            Self::ContentGuardDenied(_) => 403,
            _ => 500,
        }
    }
}

impl From<serde_json::Error> for ArtifactsError {
    fn from(e: serde_json::Error) -> Self {
        Self::SerializationError(e.to_string())
    }
}

impl From<anyhow::Error> for ArtifactsError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e.to_string())
    }
}
