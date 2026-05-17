// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-techdocs — Technical documentation hosting compatible with Backstage TechDocs.
//!
//! Port of:
//!   - `backstage/plugins/techdocs-backend/src/service/router.ts`
//!   - `backstage/plugins/techdocs-node/src/publishing/local.ts`
//!
//! # Overview
//!
//! This crate provides:
//! - [`models`]: TechDocsMetadata, EntityName, EntityMetadata types
//! - [`publisher`]: Publisher trait + LocalPublisher implementation
//! - [`generator`]: TechDocsGenerator trait + NoopGenerator
//! - [`preparer`]: TechDocsPreparer trait + NoopPreparer
//! - [`routes`]: Axum HTTP handlers compatible with the Backstage TechDocs API

pub mod generator;
pub mod models;
pub mod preparer;
pub mod publisher;
pub mod routes;

pub use models::{EntityMetadata, EntityMetadataInner, EntityName, TechDocsMetadata};
pub use publisher::local::LocalPublisher;
pub use publisher::{Publisher, TechDocsError};
pub use routes::{create_router, TechDocsState};
