// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core domain types for cave-vector.
//!
//! Mirrors the Qdrant collection/point model (`lib/segment/src/types.rs`,
//! `lib/collection/src/operations/types.rs`) re-expressed in Rust, plus the
//! Milvus schema notions of named/multi vectors and the pgvector column types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Vector distance / similarity metric.
///
/// Qdrant `Distance` (`lib/segment/src/types.rs`). For every metric we expose a
/// unified `score()` where **higher is better** (closer), so the top-k heap is
/// always a max-heap regardless of metric. Raw geometric distance is available
/// via [`Distance::distance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Distance {
    /// Cosine similarity (`Cosine`). Score in `[-1, 1]`, higher = closer.
    Cosine,
    /// Squared/Euclidean L2. Score = `-distance`, higher = closer.
    Euclid,
    /// Dot product. Score = dot, higher = closer.
    Dot,
    /// Manhattan L1. Score = `-distance`, higher = closer.
    Manhattan,
}

/// HNSW index tuning parameters (Qdrant `HnswConfig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Edges per node on layers > 0 (and `2*m` on layer 0).
    pub m: usize,
    /// Candidate-list size used while *building* the graph.
    pub ef_construct: usize,
    /// Default candidate-list size used while *searching*.
    pub ef: usize,
    /// Collections below this point-count are brute-force scanned.
    pub full_scan_threshold: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        // Qdrant defaults: m=16, ef_construct=100.
        Self { m: 16, ef_construct: 100, ef: 64, full_scan_threshold: 10_000 }
    }
}

/// Per-vector configuration in a collection schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorParams {
    /// Dimensionality.
    pub size: usize,
    /// Distance metric.
    pub distance: Distance,
    /// HNSW config (None → defaults).
    #[serde(default)]
    pub hnsw_config: Option<HnswConfig>,
    /// Optional on-disk-style quantization config.
    #[serde(default)]
    pub quantization: Option<QuantizationConfig>,
}

/// Quantization mode for a vector field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantizationConfig {
    /// Scalar int8 quantization with a quantile-based clamp.
    Scalar { quantile: f32 },
    /// 1-bit-per-dimension binary quantization.
    Binary,
    /// Product quantization: split into `m` subvectors, `nbits` per code.
    Product { m: usize, nbits: u8 },
}

/// A point identifier — Qdrant allows both u64 and UUID-string ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PointId {
    /// Numeric id.
    Num(u64),
    /// UUID / string id.
    Uuid(String),
}

impl From<u64> for PointId {
    fn from(n: u64) -> Self {
        PointId::Num(n)
    }
}
impl From<&str> for PointId {
    fn from(s: &str) -> Self {
        PointId::Uuid(s.to_string())
    }
}

/// JSON payload attached to a point.
pub type Payload = BTreeMap<String, serde_json::Value>;

/// A stored point: id, dense vector, optional payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Identifier.
    pub id: PointId,
    /// Dense vector.
    pub vector: Vec<f32>,
    /// Optional payload (defaults to empty).
    #[serde(default)]
    pub payload: Payload,
}

/// A scored search hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredPoint {
    /// Identifier.
    pub id: PointId,
    /// Unified score (higher = closer).
    pub score: f32,
    /// Payload (echoed when `with_payload`).
    #[serde(default)]
    pub payload: Payload,
}
