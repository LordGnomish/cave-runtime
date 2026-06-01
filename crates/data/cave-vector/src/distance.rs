// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Distance / similarity metrics.
//!
//! Port of Qdrant `lib/segment/src/spaces/{simple,metric}.rs`. Every metric
//! exposes:
//!   * [`Metric::distance`] — the raw geometric distance (lower = closer).
//!   * [`Metric::score`]    — a unified similarity where **higher = closer**,
//!     so the top-k selection is always a max-heap.

use crate::models::Distance;

/// A metric bound to a [`Distance`] kind.
#[derive(Debug, Clone, Copy)]
pub struct Metric(pub Distance);

impl Metric {
    /// Raw geometric distance. For cosine/dot this is `1 - similarity` /
    /// `-dot` so that "lower = closer" holds uniformly.
    pub fn distance(&self, _a: &[f32], _b: &[f32]) -> f32 {
        0.0
    }

    /// Unified similarity score (higher = closer).
    pub fn score(&self, _a: &[f32], _b: &[f32]) -> f32 {
        0.0
    }
}

/// Cosine similarity `dot(a,b) / (|a||b|)`.
pub fn cosine(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// Dot product.
pub fn dot(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// Euclidean (L2) distance.
pub fn euclid(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// Manhattan (L1) distance.
pub fn manhattan(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn dot_product_basic() {
        approx(dot(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]), 32.0);
    }

    #[test]
    fn euclid_distance_basic() {
        // sqrt((3-0)^2 + (4-0)^2) = 5
        approx(euclid(&[3.0, 4.0], &[0.0, 0.0]), 5.0);
        approx(euclid(&[1.0, 1.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn manhattan_distance_basic() {
        approx(manhattan(&[1.0, 2.0, 3.0], &[4.0, 0.0, 3.0]), 3.0 + 2.0 + 0.0);
    }

    #[test]
    fn cosine_identical_is_one() {
        approx(cosine(&[1.0, 0.0], &[2.0, 0.0]), 1.0);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        approx(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    }

    #[test]
    fn cosine_zero_vector_is_zero() {
        // Guard against NaN from a zero-norm divisor.
        approx(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn score_higher_is_closer_for_euclid() {
        let m = Metric(Distance::Euclid);
        let near = m.score(&[1.0, 1.0], &[1.0, 1.1]);
        let far = m.score(&[1.0, 1.0], &[5.0, 5.0]);
        assert!(near > far, "near {near} should outscore far {far}");
    }

    #[test]
    fn score_higher_is_closer_for_cosine() {
        let m = Metric(Distance::Cosine);
        let aligned = m.score(&[1.0, 0.0], &[1.0, 0.0]);
        let orthogonal = m.score(&[1.0, 0.0], &[0.0, 1.0]);
        assert!(aligned > orthogonal);
        approx(aligned, 1.0);
    }

    #[test]
    fn score_dot_matches_dot() {
        let m = Metric(Distance::Dot);
        approx(m.score(&[1.0, 2.0], &[3.0, 4.0]), 11.0);
    }

    #[test]
    fn euclid_score_is_negative_distance() {
        let m = Metric(Distance::Euclid);
        approx(m.score(&[3.0, 4.0], &[0.0, 0.0]), -5.0);
        approx(m.distance(&[3.0, 4.0], &[0.0, 0.0]), 5.0);
    }

    #[test]
    fn manhattan_score_is_negative_distance() {
        let m = Metric(Distance::Manhattan);
        approx(m.score(&[1.0, 2.0], &[4.0, 2.0]), -3.0);
    }
}
