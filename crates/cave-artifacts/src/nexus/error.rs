// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type for the Nexus module.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NexusError {
    #[error("repository not found: {0}")]
    RepositoryNotFound(String),

    #[error("repository already exists: {0}")]
    RepositoryAlreadyExists(String),

    #[error("repository member missing: {0}")]
    GroupMemberMissing(String),

    #[error("component not found: {0}")]
    ComponentNotFound(String),

    #[error("asset not found: {0}")]
    AssetNotFound(String),

    #[error("blob not found: {0}")]
    BlobNotFound(String),

    #[error("cleanup policy not found: {0}")]
    CleanupPolicyNotFound(String),

    #[error("routing rule not found: {0}")]
    RoutingRuleNotFound(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("invalid regex: {0}")]
    InvalidRegex(String),

    #[error("write policy denies upload to immutable asset: {0}")]
    WritePolicyDeny(String),

    #[error("unsupported repository type for this operation: {0}")]
    UnsupportedRepositoryType(String),

    #[error("routing rule denies request to path: {0}")]
    RoutingDenied(String),

    #[error("format adapter unavailable for: {0}")]
    FormatUnavailable(String),
}
