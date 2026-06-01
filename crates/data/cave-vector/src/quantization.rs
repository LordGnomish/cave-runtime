// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vector quantization.
//!
//! Port of Qdrant `lib/quantization` (scalar + binary + product). Three
//! lossy codecs that shrink stored vectors and speed up scoring:
//!   * [`ScalarQuantizer`] — global int8 with a quantile clamp.
//!   * [`binary_encode`] / [`hamming`] — 1-bit-per-dim sign quantization.
//!   * [`ProductQuantizer`] — split into `m` subspaces, k-means codebook per
//!     subspace, asymmetric distance via a precomputed lookup table (Jégou
//!     et al. 2011, "Product Quantization for NN Search").

// ── Scalar (int8) ────────────────────────────────────────────────────────────

/// Global scalar int8 quantizer with quantile clamping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalarQuantizer {
    /// Clamp lower bound.
    pub lo: f32,
    /// Clamp upper bound.
    pub hi: f32,
}

impl ScalarQuantizer {
    /// Fit `lo`/`hi` from `vectors`, trimming `quantile` of the value mass off
    /// each tail (`quantile = 0.0` → plain min/max).
    pub fn train(_vectors: &[Vec<f32>], _quantile: f32) -> Self {
        Self { lo: 0.0, hi: 0.0 }
    }

    /// Encode to one byte per dimension.
    pub fn encode(&self, _v: &[f32]) -> Vec<u8> {
        Vec::new()
    }

    /// Approximate reconstruction.
    pub fn decode(&self, _codes: &[u8]) -> Vec<f32> {
        Vec::new()
    }
}

// ── Binary (1-bit) ─────────────────────────────────────────────────────────

/// Pack each dimension into a sign bit (`v[i] > 0 → 1`), LSB-first.
pub fn binary_encode(_v: &[f32]) -> Vec<u8> {
    Vec::new()
}

/// Hamming distance between two packed bit strings.
pub fn hamming(_a: &[u8], _b: &[u8]) -> u32 {
    u32::MAX
}

// ── Product quantization ─────────────────────────────────────────────────────

/// Product quantizer: `m` subspaces, `k = 2^nbits` centroids each.
#[derive(Debug, Clone)]
pub struct ProductQuantizer {
    /// Number of subspaces.
    pub m: usize,
    /// Centroids per subspace (`2^nbits`).
    pub k: usize,
    /// Dimension of each subvector.
    pub sub_dim: usize,
    /// `codebooks[subspace][centroid]` → centroid subvector.
    pub codebooks: Vec<Vec<Vec<f32>>>,
}

impl ProductQuantizer {
    /// Train codebooks via per-subspace k-means (k-means++ init, `iters`
    /// Lloyd steps, `seed` for reproducibility).
    pub fn train(_vectors: &[Vec<f32>], m: usize, nbits: u8, _iters: usize, _seed: u64) -> Self {
        Self { m, k: 1 << nbits, sub_dim: 0, codebooks: Vec::new() }
    }

    /// Encode to `m` centroid indices.
    pub fn encode(&self, _v: &[f32]) -> Vec<u8> {
        Vec::new()
    }

    /// Reconstruct from codes (concatenate chosen centroids).
    pub fn reconstruct(&self, _codes: &[u8]) -> Vec<f32> {
        Vec::new()
    }

    /// Asymmetric squared-L2 distance: full-precision `query` vs PQ codes.
    pub fn asymmetric_distance(&self, _query: &[f32], _codes: &[u8]) -> f32 {
        f32::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "{a} != {b} (tol {tol})");
    }

    #[test]
    fn scalar_round_trips_within_step() {
        let data = vec![vec![0.0, 2.0, 10.0], vec![-1.0, 5.0, 7.0]];
        let q = ScalarQuantizer::train(&data, 0.0);
        approx(q.lo, -1.0, 1e-6);
        approx(q.hi, 10.0, 1e-6);
        let codes = q.encode(&[0.0, 5.0, 10.0]);
        assert_eq!(codes.len(), 3);
        let dec = q.decode(&codes);
        let step = (q.hi - q.lo) / 255.0;
        approx(dec[0], 0.0, step);
        approx(dec[1], 5.0, step);
        approx(dec[2], 10.0, step);
    }

    #[test]
    fn scalar_encoding_is_monotonic() {
        let q = ScalarQuantizer { lo: 0.0, hi: 100.0 };
        let lo = q.encode(&[10.0])[0];
        let hi = q.encode(&[90.0])[0];
        assert!(hi > lo);
        // clamps outside the range.
        assert_eq!(q.encode(&[-5.0])[0], 0);
        assert_eq!(q.encode(&[200.0])[0], 255);
    }

    #[test]
    fn binary_sign_bits_and_hamming() {
        let a = binary_encode(&[1.0, -1.0, 2.0, -3.0, 0.5, -0.5, 7.0, -8.0]);
        let b = binary_encode(&[1.0, -1.0, 2.0, -3.0, 0.5, -0.5, 7.0, -8.0]);
        assert_eq!(hamming(&a, &b), 0);
        // fully opposite signs → all 8 bits differ.
        let c = binary_encode(&[-1.0, 1.0, -2.0, 3.0, -0.5, 0.5, -7.0, 8.0]);
        assert_eq!(hamming(&a, &c), 8);
    }

    fn pq_data() -> Vec<Vec<f32>> {
        vec![
            vec![0.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 5.0, 5.0],
            vec![10.0, 10.0, 0.0, 0.0],
            vec![10.0, 10.0, 5.0, 5.0],
        ]
    }

    #[test]
    fn pq_reconstructs_clustered_data() {
        let pq = ProductQuantizer::train(&pq_data(), 2, 1, 10, 7);
        assert_eq!(pq.m, 2);
        assert_eq!(pq.k, 2);
        assert_eq!(pq.sub_dim, 2);
        for v in pq_data() {
            let codes = pq.encode(&v);
            assert_eq!(codes.len(), 2);
            let r = pq.reconstruct(&codes);
            for (a, b) in r.iter().zip(&v) {
                approx(*a, *b, 0.5);
            }
        }
    }

    #[test]
    fn pq_asymmetric_distance_ranks_correctly() {
        let data = pq_data();
        let pq = ProductQuantizer::train(&data, 2, 1, 10, 7);
        let query = data[0].clone(); // [0,0,0,0]
        let d_self = pq.asymmetric_distance(&query, &pq.encode(&data[0]));
        let d_far = pq.asymmetric_distance(&query, &pq.encode(&data[3])); // [10,10,5,5]
        assert!(d_self < d_far, "self {d_self} should be < far {d_far}");
        approx(d_self, 0.0, 0.5);
    }
}
