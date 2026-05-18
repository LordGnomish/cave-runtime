// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: META — cave-artifacts nexus sub-module root
//! Sonatype Nexus 3-compatible universal artifact repository module.
//!
//! Initial port (Faz 2): Repository CRUD (hosted/proxy/group), Component
//! and Asset CRUD with cascading delete, content-addressable blob storage
//! with refcounted dedupe, Cleanup policies (age/last-downloaded/regex),
//! Routing rules (allow/block precedence), Format adapter trait, and one
//! end-to-end format (`raw`) backing the upstream
//! `/repository/{name}/*path` upload+download surface.
//!
//! Subsequent format adapters (maven2, npm, docker, pypi, …) plug into the
//! same [`format::FormatAdapter`] trait and the cleanup/routing machinery
//! unchanged.

pub mod cleanup;
pub mod error;
pub mod format;
pub mod models;
pub mod routes;
pub mod routing;
pub mod store;
#[cfg(test)]
mod tests;

pub use error::NexusError;
pub use models::*;
pub use routes::{router, NexusState};

pub const MODULE_NAME: &str = "nexus";
