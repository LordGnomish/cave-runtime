//! Vector distance and similarity metrics.
//!
//! All functions operate on dense `f32` vectors and return a scalar value
//! where **lower is more similar** (to allow uniform min-heap ordering).

use crate::models::Distance;

// ─────────────────────────────────────────────────────────────────────────────
// Core metric dispatch
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the distance between two vectors according to `metric`.
///
/// Returns a non-negative value where 0.0 means identical.
/// All metrics are normalised to this convention:
/// - Cosine   → 1 − similarity (range [0, 2])
/// - Euclid   → L2 distance    (range [0, ∞))
/// - Dot      → −dot_product   (range (−∞, 0] flipped to [0, ∞))
/// - Manhattan→ L1 distance    (range [0, ∞))
#[inline]
pub fn distance(a: &[f32], b: &[f32], metric: Distance) -> f32 {
    match metric {
        Distance::Cosine => cosine_distance(a, b),
        Distance::Euclid => euclidean_distance(a, b),
        Distance::Dot => dot_distance(a, b),
        Distance::Manhattan => manhattan_distance(a, b),
    }
}

/// Convert a distance value into the Qdrant-compatible similarity score
/// that users see in responses (`score` field).
///
/// For Cosine and Dot, higher scores are better.
/// For Euclid and Manhattan, scores are negated so lower distance = higher score.
#[inline]
pub fn distance_to_score(dist: f32, metric: Distance) -> f32 {
    match metric {
        Distance::Cosine => 1.0 - dist,         // [−1, 1]
        Distance::Euclid => -dist,              // (−∞, 0]
        Distance::Dot => -dist,                 // (−∞, 0]
        Distance::Manhattan => -dist,           // (−∞, 0]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Individual metrics
// ─────────────────────────────────────────────────────────────────────────────

/// 1 − cosine_similarity.
///
/// Normalises both vectors before computing the dot product, so the result
/// is independent of vector magnitude.  Returns values in [0, 2].
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vector dimension mismatch");
    if a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0; // undefined → max distance
    }

    let similarity = dot / (norm_a * norm_b);
    // Clamp to [−1, 1] to handle floating-point rounding.
    1.0 - similarity.clamp(-1.0, 1.0)
}

/// Euclidean (L2) distance: √Σ(aᵢ − bᵢ)².
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vector dimension mismatch");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum::<f32>()
        .sqrt()
}

/// Squared Euclidean distance (avoids the square root for comparison-only use).
#[inline]
pub fn euclidean_distance_sq(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vector dimension mismatch");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// Dot-product distance: −(a · b).
///
/// The negation makes the convention consistent (lower = more similar).
/// This is only well-defined for normalised vectors.
pub fn dot_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vector dimension mismatch");
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    -dot
}

/// Raw dot product (without negation).
#[inline]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Manhattan (L1) distance: Σ|aᵢ − bᵢ|.
pub fn manhattan_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vector dimension mismatch");
    a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum()
}

// ─────────────────────────────────────────────────────────────────────────────
// Vector normalisation utilities
// ─────────────────────────────────────────────────────────────────────────────

/// Return the L2 norm of a vector.
#[inline]
pub fn norm_l2(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Normalise a vector in-place to unit L2 norm.
pub fn normalise_inplace(v: &mut Vec<f32>) {
    let n = norm_l2(v);
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

/// Return a new normalised copy of the vector.
pub fn normalise(v: &[f32]) -> Vec<f32> {
    let n = norm_l2(v);
    if n == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / n).collect()
}

/// Add two vectors element-wise.
pub fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
}

/// Subtract vector `b` from `a` element-wise.
pub fn sub(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b.iter()).map(|(x, y)| x - y).collect()
}

/// Multiply all components of a vector by a scalar.
pub fn scale(v: &[f32], s: f32) -> Vec<f32> {
    v.iter().map(|x| x * s).collect()
}

/// Compute the centroid (mean) of a set of vectors.
pub fn centroid(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    if vectors.is_empty() {
        return None;
    }
    let dim = vectors[0].len();
    let mut sum = vec![0.0f32; dim];
    for v in vectors {
        for (i, x) in v.iter().enumerate() {
            sum[i] += x;
        }
    }
    let n = vectors.len() as f32;
    Some(sum.iter().map(|x| x / n).collect())
}

/// Generate a random unit vector of the given dimension.
pub fn random_unit_vector(dim: usize, rng: &mut impl rand::Rng) -> Vec<f32> {
    use rand::distributions::Standard;
    use rand::Rng;
    let v: Vec<f32> = rng.sample_iter::<f32, _>(Standard).take(dim).collect();
    normalise(&v)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 0.0, 0.0];
        assert!(approx_eq(cosine_distance(&v, &v), 0.0));
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(approx_eq(cosine_distance(&a, &b), 1.0));
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!(approx_eq(cosine_distance(&a, &b), 2.0));
    }

    #[test]
    fn euclidean_distance_3d() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 6.0, 3.0];
        // sqrt((3^2) + (4^2) + 0) = sqrt(9 + 16) = 5
        assert!(approx_eq(euclidean_distance(&a, &b), 5.0));
    }

    #[test]
    fn euclidean_identical() {
        let v = vec![0.5, 0.5, 0.5];
        assert!(approx_eq(euclidean_distance(&v, &v), 0.0));
    }

    #[test]
    fn dot_distance_perpendicular() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(approx_eq(dot_distance(&a, &b), 0.0));
    }

    #[test]
    fn dot_distance_parallel() {
        let a = vec![1.0, 0.0];
        assert!(approx_eq(dot_distance(&a, &a), -1.0));
    }

    #[test]
    fn manhattan_distance_basic() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 0.0, 5.0];
        // |1-4| + |2-0| + |3-5| = 3 + 2 + 2 = 7
        assert!(approx_eq(manhattan_distance(&a, &b), 7.0));
    }

    #[test]
    fn normalise_unit_vector() {
        let v = vec![3.0, 4.0];
        let n = normalise(&v);
        assert!(approx_eq(norm_l2(&n), 1.0));
        assert!(approx_eq(n[0], 0.6));
        assert!(approx_eq(n[1], 0.8));
    }

    #[test]
    fn centroid_of_two_vectors() {
        let a = vec![1.0, 2.0];
        let b = vec![3.0, 4.0];
        let c = centroid(&[a, b]).unwrap();
        assert!(approx_eq(c[0], 2.0));
        assert!(approx_eq(c[1], 3.0));
    }

    #[test]
    fn centroid_empty_returns_none() {
        assert!(centroid(&[]).is_none());
    }

    #[test]
    fn add_vectors() {
        let a = vec![1.0, 2.0];
        let b = vec![3.0, 4.0];
        let c = add(&a, &b);
        assert_eq!(c, vec![4.0, 6.0]);
    }

    #[test]
    fn distance_to_score_cosine() {
        // cosine_distance = 0.0 → score = 1.0 (perfect match)
        let score = distance_to_score(0.0, Distance::Cosine);
        assert!(approx_eq(score, 1.0));
    }

    #[test]
    fn distance_to_score_euclid() {
        let score = distance_to_score(3.0, Distance::Euclid);
        assert!(approx_eq(score, -3.0));
    }

    #[test]
    fn distance_dispatch() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(approx_eq(distance(&a, &b, Distance::Cosine), 1.0));
        assert!(approx_eq(distance(&a, &b, Distance::Euclid), 2.0f32.sqrt()));
        assert!(approx_eq(distance(&a, &b, Distance::Dot), 0.0));
        assert!(approx_eq(distance(&a, &b, Distance::Manhattan), 2.0));
    }
}
