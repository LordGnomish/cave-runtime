// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("not found: {kind}/{name}")]
    NotFound { kind: String, name: String },
    #[error("already exists: {kind}/{name}")]
    AlreadyExists { kind: String, name: String },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;
