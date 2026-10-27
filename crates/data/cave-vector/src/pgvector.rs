// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! pgvector integration surface.
//!
//! Re-expresses the pgvector (PostgreSQL extension, PostgreSQL licence) SQL
//! distance operators and the ivfflat list index:
//!   * `<->` L2, `<=>` cosine distance, `<#>` negative inner product,
//!     `<+>` L1 — see `src/vector.c` operator definitions.
//!   * [`parse_vector`] / [`format_vector`] — the `'[1,2,3]'` text I/O.
//!   * [`IvfFlat`] — the inverted-file flat index (`src/ivfflat.c`): k-means
//!     list centroids + `probes`-limited scan.

use crate::distance::{cosine, dot};
use crate::error::VectorError;
use crate::models::{Distance, Payload, PointId, ScoredPoint};
use crate::quantization::{kmeans, l2_sq, nearest_centroid};

/// `<->` Euclidean (L2) distance.
pub fn l2_distance(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// `<=>` cosine distance = `1 - cosine_similarity`.
pub fn cosine_distance(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// `<#>` negative inner product (pgvector negates so ASC index scans work).
pub fn negative_inner_product(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// `<+>` taxicab / L1 distance (pgvector 0.7+).
pub fn l1_distance(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
}

/// Parse pgvector text form `[1,2,3]` (whitespace tolerant).
pub fn parse_vector(_s: &str) -> Result<Vec<f32>, VectorError> {
    Err(VectorError::Invalid("not implemented".into()))
}

/// Render as pgvector text form `[1,2,3]`.
pub fn format_vector(_v: &[f32]) -> String {
    String::new()
}

/// ivfflat inverted-file index: k-means list centroids + per-list postings.
pub struct IvfFlat {
    distance: Distance,
    centroids: Vec<Vec<f32>>,
    lists: Vec<Vec<(PointId, Vec<f32>)>>,
}

impl IvfFlat {
    /// Build `nlists` centroids over `data` and assign each point to its
    /// nearest list (k-means clustering, `seed` for reproducibility).
    pub fn train(
        _data: &[(PointId, Vec<f32>)],
        _nlists: usize,
        distance: Distance,
        _iters: usize,
        _seed: u64,
    ) -> Self {
        Self { distance, centroids: Vec::new(), lists: Vec::new() }
    }

    /// Number of lists.
    pub fn nlists(&self) -> usize {
        self.centroids.len()
    }

    /// Nearest list index for a vector.
    pub fn list_of(&self, _v: &[f32]) -> usize {
        usize::MAX
    }

    /// Search the `nprobe` nearest lists and return top-`k` by metric score.
    pub fn search(&self, _query: &[f32], _nprobe: usize, _k: usize) -> Vec<ScoredPoint> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distance::Metric;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn operators_match_definitions() {
        approx(l2_distance(&[3.0, 4.0], &[0.0, 0.0]), 5.0);
        approx(cosine_distance(&[1.0, 0.0], &[1.0, 0.0]), 0.0);
        approx(cosine_distance(&[1.0, 0.0], &[0.0, 1.0]), 1.0);
        approx(negative_inner_product(&[1.0, 2.0], &[3.0, 4.0]), -11.0);
        approx(l1_distance(&[1.0, 2.0, 3.0], &[4.0, 0.0, 3.0]), 5.0);
    }

    #[test]
    fn parse_and_format_round_trip() {
        let v = parse_vector("[1,2,3]").unwrap();
        assert_eq!(v, vec![1.0, 2.0, 3.0]);
        // whitespace tolerant
        let v2 = parse_vector(" [ 1.5 , -2 , 0 ] ").unwrap();
        assert_eq!(v2, vec![1.5, -2.0, 0.0]);
        assert_eq!(format_vector(&[1.0, 2.0, 3.0]), "[1,2,3]");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_vector("not a vector").is_err());
        assert!(parse_vector("[1,foo,3]").is_err());
    }

    fn clustered() -> Vec<(PointId, Vec<f32>)> {
        vec![
            (PointId::Num(1), vec![0.0, 0.0]),
            (PointId::Num(2), vec![0.5, 0.5]),
            (PointId::Num(3), vec![0.2, 0.1]),
            (PointId::Num(4), vec![10.0, 10.0]),
            (PointId::Num(5), vec![10.5, 9.5]),
            (PointId::Num(6), vec![9.8, 10.2]),
        ]
    }

    #[test]
    fn ivfflat_assigns_clusters() {
        let idx = IvfFlat::train(&clustered(), 2, Distance::Euclid, 10, 3);
        assert_eq!(idx.nlists(), 2);
        // points 1,2,3 share a list; 4,5,6 share the other.
        let l1 = idx.list_of(&[0.0, 0.0]);
        let l2 = idx.list_of(&[10.0, 10.0]);
        assert_ne!(l1, l2);
        assert_eq!(idx.list_of(&[0.3, 0.3]), l1);
    }

    #[test]
    fn ivfflat_probe_one_finds_local_nearest() {
        let idx = IvfFlat::train(&clustered(), 2, Distance::Euclid, 10, 3);
        let hits = idx.search(&[0.1, 0.1], 1, 2);
        assert_eq!(hits.len(), 2);
        // nearest to [0.1,0.1] within its list is point 1 ([0,0]).
        assert_eq!(hits[0].id, PointId::Num(1));
        assert!(hits.iter().all(|h| matches!(h.id, PointId::Num(n) if n <= 3)));
    }

    #[test]
    fn ivfflat_full_probe_equals_brute_force() {
        let data = clustered();
        let idx = IvfFlat::train(&data, 2, Distance::Euclid, 10, 3);
        let hits = idx.search(&[9.9, 9.9], 2, 1);
        // scanning all lists must find the global nearest (point 6 ~[9.8,10.2]
        // or 4 ~[10,10]); confirm it matches a brute-force pass.
        let metric = Metric(Distance::Euclid);
        let best = data
            .iter()
            .max_by(|a, b| {
                metric.score(&[9.9, 9.9], &a.1).total_cmp(&metric.score(&[9.9, 9.9], &b.1))
            })
            .unwrap()
            .0
            .clone();
        assert_eq!(hits[0].id, best);
    }
}
