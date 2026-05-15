// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Publisher trait — port of PublisherBase from backstage-plugin-techdocs-node.
//!
//! Upstream: PublisherBase in @backstage/plugin-techdocs-node/src/publishing/PublisherBase.ts

use crate::models::{EntityName, TechDocsMetadata};
use async_trait::async_trait;
use thiserror::Error;

pub mod local;

/// Errors that can occur in TechDocs operations.
///
/// Upstream: errors thrown by LocalPublisher in techdocs-node/src/publishing/local.ts
#[derive(Debug, Error)]
pub enum TechDocsError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Publisher trait — abstracts where generated docs are stored and retrieved from.
///
/// Upstream: Publisher interface in @backstage/plugin-techdocs-node
#[async_trait]
pub trait Publisher: Send + Sync {
    /// Publish the generated docs directory for an entity.
    ///
    /// Upstream: publish(entity, directory) → void
    async fn publish(
        &self,
        entity: &EntityName,
        docs_path: &std::path::Path,
    ) -> Result<(), TechDocsError>;

    /// Fetch metadata (techdocs_metadata.json).
    ///
    /// Upstream: fetchTechDocsMetadata(entityName) → TechDocsMetadata
    async fn fetch_metadata(&self, entity: &EntityName) -> Result<TechDocsMetadata, TechDocsError>;

    /// Check if docs exist for entity.
    ///
    /// Upstream: hasDocsBeenGenerated(entityName) → bool
    async fn has_docs(&self, entity: &EntityName) -> Result<bool, TechDocsError>;

    /// Read a static file from the published docs.
    ///
    /// Upstream: migrateDocsCase / fetchStaticFile in local.ts
    async fn read_file(
        &self,
        entity: &EntityName,
        path: &str,
    ) -> Result<Vec<u8>, TechDocsError>;
}
