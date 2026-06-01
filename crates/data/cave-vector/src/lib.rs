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

/// Error type.
pub mod error;

/// Collection schema + point storage.
pub mod collection;

/// HNSW proximity-graph index.
pub mod hnsw;

/// Payload filtering.
pub mod filter;

/// Search composition (filtered / MMR / hybrid fusion).
pub mod search;

/// Vector quantization (scalar / binary / product).
pub mod quantization;

/// Sharding + replication.
pub mod sharding;

/// Collection snapshots.
pub mod snapshot;

/// Sparse vectors, BM25, ColBERT MaxSim.
pub mod sparse;

pub use collection::{Collection, CollectionStore};
pub use filter::{Condition, Filter};
pub use hnsw::HnswIndex;
pub use distance::Metric;
pub use error::VectorError;
pub use models::{Distance, Point, PointId, ScoredPoint, VectorParams};

/// Module name constant (used by the runtime parity discovery + router mount).
pub const MODULE_NAME: &str = "vector";
