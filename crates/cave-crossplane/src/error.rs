// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-crossplane.

use thiserror::Error;

pub type CrossplaneResult<T> = Result<T, CrossplaneError>;

#[derive(Error, Debug)]
pub enum CrossplaneError {
    #[error("XRD not found: {0}")]
    XrdNotFound(String),

    #[error("Composition not found: {0}")]
    CompositionNotFound(String),

    #[error("Claim not found: {0}")]
    ClaimNotFound(String),

    #[error("Composite not found: {0}")]
    CompositeNotFound(String),

    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("XRD validation error: {0}")]
    XrdValidation(String),

    #[error("Composition validation error: {0}")]
    CompositionValidation(String),

    #[error("Claim validation error: {0}")]
    ClaimValidation(String),

    #[error("Patch transform error: {0}")]
    PatchTransform(String),

    #[error("Reconcile error: {0}")]
    ReconcileError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl CrossplaneError {
    pub fn status_code(&self) -> u16 {
        match self {
            CrossplaneError::XrdNotFound(_)
            | CrossplaneError::CompositionNotFound(_)
            | CrossplaneError::ClaimNotFound(_)
            | CrossplaneError::CompositeNotFound(_)
            | CrossplaneError::ProviderNotFound(_) => 404,
            CrossplaneError::XrdValidation(_)
            | CrossplaneError::CompositionValidation(_)
            | CrossplaneError::ClaimValidation(_)
            | CrossplaneError::PatchTransform(_) => 400,
            CrossplaneError::ReconcileError(_) | CrossplaneError::Internal(_) => 500,
        }
    }
}
