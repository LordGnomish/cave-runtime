// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-lakehouse — consolidated lakehouse stack per ADR-147.
//!
//! Same multi-upstream pattern as cave-streams (Kafka + Pulsar) and
//! cave-gateway (Kong + Gravitee).
//!
//! Layout (per ADR-147 §3.2):
//!
//!   * `table_format::iceberg` — Apache Iceberg spec v2 metadata: Schema,
//!     PartitionSpec, Manifest, Snapshot, TableMetadata.
//!   * `engine::datafusion`    — Logical and physical query plans operating
//!     over the table format above.
//!
//! Future sub-upstreams documented in ADR-147 (Delta/Hudi optional table
//! formats, Arrow/Parquet IO, MinIO substrate) live as future submodules.

pub mod table_format;
pub mod engine;
