//! Backstage-compatible catalog module for cave-portal.
//!
//! Provides entity / location / refresh-state persistence with two backends:
//! - [`store::MemoryCatalogStore`] — in-process, for tests and local dev.
//! - [`store::PostgresCatalogStore`] — production, backed by `cave-db`.

pub mod migrations;
pub mod models;
pub mod store;

// Convenience re-exports.
pub use migrations::CATALOG_SCHEMA_SQL;
pub use models::{Entity, EntityFilter, EntityMetadata, EntityRelation, Location, RefreshStateRecord};
pub use store::{CatalogStore, CatalogStoreError, MemoryCatalogStore, PostgresCatalogStore};
