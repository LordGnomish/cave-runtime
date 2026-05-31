// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `pgvector` — the `vector` type and its distance operators.
//!
//! Pure-Rust port of pgvector's `src/vector.c`. A [`Vector`] is a fixed-length
//! array of `float4` components (max [`VECTOR_MAX_DIM`]). Distances are
//! accumulated in `f64` to match upstream, which uses `double`:
//!   * [`Vector::l2_distance`]   — `<->` Euclidean
//!   * [`Vector::inner_product`] / [`Vector::negative_inner_product`] — `<#>`
//!   * [`Vector::cosine_distance`] — `<=>` (`NaN` when either norm is zero)
//!   * [`Vector::l1_distance`]   — `<+>` taxicab
//! Binary ops require matching dimensions. [`nearest_neighbor`] is a
//! brute-force L2 scan standing in for an ivfflat/hnsw index probe.

/// `VECTOR_MAX_DIM` — the largest vector pgvector will store.
pub const VECTOR_MAX_DIM: usize = 16000;

/// Errors constructing or operating on vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorError {
    /// more than `VECTOR_MAX_DIM` components
    TooManyDimensions(usize),
    /// two operands of differing dimension `(left, right)`
    DimMismatch(usize, usize),
}

/// A pgvector `vector` value.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector {
    data: Vec<f32>,
}

impl Vector {
    /// Construct without bounds-checking the dimension (callers that already
    /// trust the source, e.g. literals built in tests).
    pub fn new(data: Vec<f32>) -> Self {
        Vector { data }
    }

    /// `CheckDim` — reject vectors exceeding `VECTOR_MAX_DIM`.
    pub fn checked(data: Vec<f32>) -> Result<Self, VectorError> {
        if data.len() > VECTOR_MAX_DIM {
            return Err(VectorError::TooManyDimensions(data.len()));
        }
        Ok(Vector { data })
    }

    pub fn dim(&self) -> usize {
        self.data.len()
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }

    fn check_dims(&self, other: &Vector) -> Result<(), VectorError> {
        if self.data.len() != other.data.len() {
            return Err(VectorError::DimMismatch(self.data.len(), other.data.len()));
        }
        Ok(())
    }

    /// `<->` — Euclidean (L2) distance, `sqrt(sum((a-b)^2))`.
    pub fn l2_distance(&self, other: &Vector) -> Result<f64, VectorError> {
        self.check_dims(other)?;
        let sum: f64 = self
            .data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| {
                let d = *a as f64 - *b as f64;
                d * d
            })
            .sum();
        Ok(sum.sqrt())
    }

    /// Dot product `sum(a*b)`.
    pub fn inner_product(&self, other: &Vector) -> Result<f64, VectorError> {
        self.check_dims(other)?;
        Ok(self
            .data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| *a as f64 * *b as f64)
            .sum())
    }

    /// `<#>` — the negative inner product pgvector exposes as an operator.
    pub fn negative_inner_product(&self, other: &Vector) -> Result<f64, VectorError> {
        self.inner_product(other).map(|d| -d)
    }

    /// `<=>` — cosine distance `1 - (a·b)/(||a|| ||b||)`. When either operand
    /// has zero magnitude the similarity is undefined and pgvector yields NaN.
    pub fn cosine_distance(&self, other: &Vector) -> Result<f64, VectorError> {
        self.check_dims(other)?;
        let mut dot = 0.0_f64;
        let mut na = 0.0_f64;
        let mut nb = 0.0_f64;
        for (a, b) in self.data.iter().zip(&other.data) {
            let (a, b) = (*a as f64, *b as f64);
            dot += a * b;
            na += a * a;
            nb += b * b;
        }
        let denom = (na * nb).sqrt();
        if denom == 0.0 {
            return Ok(f64::NAN);
        }
        let similarity = dot / denom;
        // pgvector clamps the cosine similarity into [-1, 1] before 1 - sim.
        let similarity = similarity.clamp(-1.0, 1.0);
        Ok(1.0 - similarity)
    }

    /// `<+>` — L1 (taxicab) distance `sum(|a-b|)`.
    pub fn l1_distance(&self, other: &Vector) -> Result<f64, VectorError> {
        self.check_dims(other)?;
        Ok(self
            .data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum())
    }
}

/// Brute-force nearest neighbour by L2 distance — returns the index of the
/// closest corpus vector to `query`, or `None` for an empty corpus or on a
/// dimension mismatch. Stands in for an ivfflat/hnsw index scan.
pub fn nearest_neighbor(query: &Vector, corpus: &[Vector]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, cand) in corpus.iter().enumerate() {
        let Ok(d) = query.l2_distance(cand) else {
            continue;
        };
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_clamps_to_unit_range() {
        // identical vectors → similarity exactly 1 → distance 0 (no negative
        // distance from float drift)
        let a = Vector::new(vec![3.0, 4.0]);
        let d = a.cosine_distance(&a).unwrap();
        assert!(d >= 0.0 && d < 1e-9);
    }

    #[test]
    fn nn_skips_mismatched_dims() {
        let q = Vector::new(vec![0.0, 0.0]);
        let corpus = vec![Vector::new(vec![1.0]), Vector::new(vec![2.0, 2.0])];
        // first candidate has wrong dim → skipped; index 1 wins
        assert_eq!(nearest_neighbor(&q, &corpus), Some(1));
    }
}
