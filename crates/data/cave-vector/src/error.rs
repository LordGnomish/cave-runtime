// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type for cave-vector operations.

/// Errors surfaced by the collection store / index / search layers.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VectorError {
    /// `create_collection` on a name that already exists.
    #[error("collection {0:?} already exists")]
    CollectionExists(String),
    /// Lookup on a missing collection.
    #[error("collection {0:?} not found")]
    CollectionNotFound(String),
    /// Upserted vector length differs from the schema dimension.
    #[error("vector dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch {
        /// Schema dimension.
        expected: usize,
        /// Provided dimension.
        got: usize,
    },
    /// Point id not present.
    #[error("point not found")]
    PointNotFound,
    /// Malformed request (filters, quantization params, …).
    #[error("invalid request: {0}")]
    Invalid(String),
}
