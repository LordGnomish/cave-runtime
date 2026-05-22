// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg Catalog trait.
//!
//! Upstream: `crates/iceberg/src/catalog.rs`
//! Spec: <https://iceberg.apache.org/spec/#catalog>
//!
//! The Catalog is the entry point — it lists/loads tables given a
//! `TableIdent` (namespace + name). The MVP defines the trait and
//! ships two implementations: `MemoryCatalog` (in-process, tests +
//! single-node use) and `RestCatalog` (Iceberg REST spec over HTTP).

use crate::error::Result;
use crate::namespace::{Namespace, NamespaceIdent};
use crate::table::{Table, TableIdent};
use crate::table_metadata::TableMetadata;
use async_trait::async_trait;

#[async_trait]
pub trait Catalog: Send + Sync {
    // ── namespace operations ─────────────────────────────────────────────────

    async fn create_namespace(&self, ns: &Namespace) -> Result<()>;
    async fn drop_namespace(&self, ident: &NamespaceIdent) -> Result<()>;
    async fn list_namespaces(&self, parent: Option<&NamespaceIdent>)
    -> Result<Vec<NamespaceIdent>>;
    async fn namespace_exists(&self, ident: &NamespaceIdent) -> Result<bool>;

    // ── table operations ─────────────────────────────────────────────────────

    async fn create_table(&self, ident: &TableIdent, metadata: TableMetadata) -> Result<Table>;
    async fn drop_table(&self, ident: &TableIdent) -> Result<()>;
    async fn load_table(&self, ident: &TableIdent) -> Result<Table>;
    async fn list_tables(&self, ns: &NamespaceIdent) -> Result<Vec<TableIdent>>;
    async fn table_exists(&self, ident: &TableIdent) -> Result<bool>;

    // ── rename ───────────────────────────────────────────────────────────────

    async fn rename_table(&self, from: &TableIdent, to: &TableIdent) -> Result<()>;

    // ── metadata refresh ─────────────────────────────────────────────────────

    /// Replace the table's metadata pointer. The MVP uses this as the
    /// commit primitive — the upstream `Catalog::commit_table` carries
    /// a `TableUpdate` enum sequence; we accept the post-update
    /// metadata directly. ACID guarantees are deferred to the impl.
    async fn replace_table_metadata(
        &self,
        ident: &TableIdent,
        new_metadata: TableMetadata,
    ) -> Result<Table>;
}
