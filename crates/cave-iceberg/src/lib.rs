// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CAVE ICEBERG — Sovereign Apache Iceberg table format.
//!
//! Upstream: `apache/iceberg-rust` v0.9.1 (`source_sha = 96cde57d`).
//! Iceberg is the open table format that defines partition layout,
//! schema evolution, snapshot isolation, and time-travel on top of
//! object-store data files (Parquet/ORC/Avro).
//!
//! This MVP is **read-side** — the catalog API, table metadata parser,
//! manifest/manifest-list reader, snapshot navigation, and scan
//! planning. The write path (committers, transaction coordination,
//! data-file writer) is intentionally deferred to lakehouse-ray-2.
//!
//! ## Status
//! See `parity.manifest.toml` for the exhaustive enumeration. The
//! crate carries both an in-process `memory_catalog` and a `rest_catalog`
//! over an Iceberg REST-spec server. Predicate expressions are
//! evaluable but pushdown to Parquet row-groups is **not** wired in
//! the MVP (the `expr` module records the predicate; row-group
//! elimination is a v0.2 milestone).

pub mod catalog;
pub mod error;
pub mod expr;
pub mod file_io;
pub mod manifest;
pub mod manifest_list;
pub mod memory_catalog;
pub mod namespace;
pub mod rest_catalog;
pub mod scan;
pub mod schema;
pub mod snapshot;
pub mod sort_order;
pub mod table;
pub mod table_metadata;
pub mod transaction;
pub mod transform;
pub mod writer;

pub use catalog::Catalog;
pub use error::{Error, Result};
pub use expr::{Predicate, Reference, Term};
pub use file_io::{FileIo, MemFileIo};
pub use manifest::{
    DataFile, DataFileContent, FileFormat, Manifest, ManifestEntry, ManifestEntryStatus,
};
pub use manifest_list::{ManifestFile, ManifestList, ManifestListContent};
pub use memory_catalog::MemoryCatalog;
pub use namespace::{Namespace, NamespaceIdent};
pub use rest_catalog::RestCatalog;
pub use scan::{FileScanTask, ScanBuilder};
pub use schema::{NestedField, PrimitiveType, Schema, SchemaBuilder, Type};
pub use snapshot::{Snapshot, SnapshotRef};
pub use sort_order::{NullOrder, SortDirection, SortField, SortOrder};
pub use table::{Table, TableIdent};
pub use table_metadata::{FormatVersion, TableMetadata, TableMetadataBuilder};
pub use transaction::{rewrite_manifest, Transaction};
pub use transform::Transform;
pub use writer::{DataFileWriter, WriteOperation, WritePlan};

pub const UPSTREAM: &str = "apache/iceberg-rust";
pub const UPSTREAM_VERSION: &str = "v0.9.1";
