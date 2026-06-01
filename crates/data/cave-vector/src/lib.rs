// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Vector — sovereign vector database.
//!
//! Clean-room Rust reimplementation of the in-scope feature surface of three
//! Apache-2.0 upstreams:
//!   * **qdrant/qdrant** — collection model, HNSW index, payload filtering,
//!     quantization, sparse vectors, the REST API shape.
//!   * **milvus-io/milvus** — multi/named vectors, hybrid search fusion,
//!     consistency/replica notions.
//!   * **pgvector/pgvector** — the SQL distance operators and ivfflat list
//!     assignment used by the Postgres integration surface.
//!
//! No upstream code is linked or vendored — semantics are re-expressed in
//! dependency-light Rust (only `serde`/`axum` for the wire surface).

/// Core domain types (collections, points, distance enum).
pub mod models;

/// Distance / similarity metrics.
pub mod distance;

pub use distance::Metric;
pub use models::{Distance, Point, PointId, ScoredPoint, VectorParams};

/// Module name constant (used by the runtime parity discovery + router mount).
pub const MODULE_NAME: &str = "vector";
