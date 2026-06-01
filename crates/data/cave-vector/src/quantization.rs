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
    pub fn train(vectors: &[Vec<f32>], quantile: f32) -> Self {
        let mut all: Vec<f32> = vectors.iter().flatten().copied().collect();
        if all.is_empty() {
            return Self { lo: 0.0, hi: 0.0 };
        }
        all.sort_by(f32::total_cmp);
        let n = all.len();
        let trim = ((quantile.clamp(0.0, 0.49)) * n as f32).floor() as usize;
        let lo = all[trim.min(n - 1)];
        let hi = all[(n - 1 - trim).max(0)];
        Self { lo, hi }
    }

    /// Encode to one byte per dimension.
    pub fn encode(&self, v: &[f32]) -> Vec<u8> {
        let span = (self.hi - self.lo).max(f32::EPSILON);
        v.iter()
            .map(|&x| {
                let t = ((x - self.lo) / span * 255.0).round();
                t.clamp(0.0, 255.0) as u8
            })
            .collect()
    }

    /// Approximate reconstruction.
    pub fn decode(&self, codes: &[u8]) -> Vec<f32> {
        let span = self.hi - self.lo;
        codes.iter().map(|&c| self.lo + (c as f32 / 255.0) * span).collect()
    }
}

// ── Binary (1-bit) ─────────────────────────────────────────────────────────

/// Pack each dimension into a sign bit (`v[i] > 0 → 1`), LSB-first.
pub fn binary_encode(v: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; v.len().div_ceil(8)];
    for (i, &x) in v.iter().enumerate() {
        if x > 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// Hamming distance between two packed bit strings.
pub fn hamming(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
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
    pub fn train(vectors: &[Vec<f32>], m: usize, nbits: u8, iters: usize, seed: u64) -> Self {
        let dim = vectors.first().map(|v| v.len()).unwrap_or(0);
        let sub_dim = dim / m;
        let k = 1usize << nbits;
        let mut codebooks = Vec::with_capacity(m);
        let mut rng = seed;
        for s in 0..m {
            let subs: Vec<Vec<f32>> = vectors
                .iter()
                .map(|v| v[s * sub_dim..(s + 1) * sub_dim].to_vec())
                .collect();
            codebooks.push(kmeans(&subs, k, iters, &mut rng));
        }
        Self { m, k, sub_dim, codebooks }
    }

    fn subspace(&self, v: &[f32], s: usize) -> Vec<f32> {
        v[s * self.sub_dim..(s + 1) * self.sub_dim].to_vec()
    }

    /// Encode to `m` centroid indices.
    pub fn encode(&self, v: &[f32]) -> Vec<u8> {
        (0..self.m)
            .map(|s| {
                let sub = self.subspace(v, s);
                nearest_centroid(&sub, &self.codebooks[s]) as u8
            })
            .collect()
    }

    /// Reconstruct from codes (concatenate chosen centroids).
    pub fn reconstruct(&self, codes: &[u8]) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.m * self.sub_dim);
        for (s, &c) in codes.iter().enumerate() {
            out.extend_from_slice(&self.codebooks[s][c as usize]);
        }
        out
    }

    /// Asymmetric squared-L2 distance: full-precision `query` vs PQ codes.
    pub fn asymmetric_distance(&self, query: &[f32], codes: &[u8]) -> f32 {
        let mut sum = 0.0;
        for (s, &c) in codes.iter().enumerate() {
            let q = self.subspace(query, s);
            sum += l2_sq(&q, &self.codebooks[s][c as usize]);
        }
        sum
    }
}

fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

fn nearest_centroid(v: &[f32], centroids: &[Vec<f32>]) -> usize {
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, c) in centroids.iter().enumerate() {
        let d = l2_sq(v, c);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

fn next_unit(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// k-means with k-means++ seeding. Empty clusters keep their prior centroid.
fn kmeans(points: &[Vec<f32>], k: usize, iters: usize, rng: &mut u64) -> Vec<Vec<f32>> {
    let n = points.len();
    let dim = points.first().map(|p| p.len()).unwrap_or(0);
    if n == 0 {
        return vec![vec![0.0; dim]; k];
    }
    // ── k-means++ init ──
    let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(k);
    let first = (next_unit(rng) * n as f64) as usize % n;
    centroids.push(points[first].clone());
    while centroids.len() < k {
        let d2: Vec<f32> = points
            .iter()
            .map(|p| {
                centroids
                    .iter()
                    .map(|c| l2_sq(p, c))
                    .fold(f32::MAX, f32::min)
            })
            .collect();
        let total: f32 = d2.iter().sum();
        if total <= 0.0 {
            // all points coincide with chosen centroids — pad with a repeat.
            centroids.push(points[centroids.len() % n].clone());
            continue;
        }
        let target = (next_unit(rng) as f32) * total;
        let mut acc = 0.0;
        let mut chosen = n - 1;
        for (i, &w) in d2.iter().enumerate() {
            acc += w;
            if acc >= target {
                chosen = i;
                break;
            }
        }
        centroids.push(points[chosen].clone());
    }
    // ── Lloyd iterations ──
    for _ in 0..iters {
        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for p in points {
            let c = nearest_centroid(p, &centroids);
            counts[c] += 1;
            for (s, &x) in p.iter().enumerate() {
                sums[c][s] += x;
            }
        }
        for c in 0..k {
            if counts[c] > 0 {
                for s in 0..dim {
                    centroids[c][s] = sums[c][s] / counts[c] as f32;
                }
            }
        }
    }
    centroids
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
