//! cave-iceberg — Apache Iceberg table format primitives.
//!
//! Models the on-disk JSON metadata documented in the
//! [Apache Iceberg spec v2](https://iceberg.apache.org/spec/) and mirrored
//! in `apache/iceberg-rust` v0.x:
//!
//!   * `Schema`        — typed column list + identifier (apache/iceberg-rust crates/iceberg/src/spec/schema.rs)
//!   * `PartitionSpec` — partition transform list bound to source-id (spec/partition.rs)
//!   * `Manifest`      — list of data-file entries (spec/manifest.rs)
//!   * `Snapshot`      — atomic table version (spec/snapshot.rs)
//!   * `TableMetadata` — top-level metadata.json (spec/table_metadata.rs)
//!
//! Every metadata object carries a `tenant_id` invariant matching the cave-cri
//! tenant model so that tables are scoped per tenant in the catalog.

pub mod schema;
pub mod partition;
pub mod manifest;
pub mod snapshot;
pub mod table_metadata;
pub mod tenant;
pub mod error;
