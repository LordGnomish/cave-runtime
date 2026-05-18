// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shamir's Secret Sharing over GF(256).
//!
//! Uses the primitive polynomial x^8 + x^4 + x^3 + x + 1.

use rand::RngCore;

// ── GF(256) arithmetic ──────────────────────────────────────────────────────

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut result = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            result ^= a;
        }
        b >>= 1;
        let carry = a & 0x80;
        a <<= 1;
        if carry != 0 {
            a ^= 0x1b; // x^8 + x^4 + x^3 + x + 1 reduced
        }
    }
    result
}

fn gf_pow(mut base: u8, mut exp: u8) -> u8 {
    let mut result = 1u8;
    while exp > 0 {
        if exp & 1 != 0 {
            result = gf_mul(result, base);
        }
        base = gf_mul(base, base);
        exp >>= 1;
    }
    result
}

fn gf_inv(x: u8) -> u8 {
    assert!(x != 0, "no inverse of 0 in GF(256)");
    gf_pow(x, 254)
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Split `secret` into `shares` shares where any `threshold` can reconstruct it.
///
/// Each returned share is `[x_i, y_0, y_1, …, y_{n-1}]` where the first byte
/// is the x-coordinate (1-indexed) and the rest are evaluations of each secret
/// byte's polynomial at that x.
pub fn split(secret: &[u8], threshold: u8, shares: u8) -> Vec<Vec<u8>> {
    assert!(threshold >= 2, "threshold must be >= 2");
    assert!(shares >= threshold, "shares must be >= threshold");
    assert!(shares <= 255, "maximum 255 shares");
    assert!(!secret.is_empty(), "secret must not be empty");

    let mut rng = rand::thread_rng();
    // result[i] = [x=i+1, y_0, y_1, ...]
    let mut result: Vec<Vec<u8>> = (0..shares as usize)
        .map(|i| vec![i as u8 + 1])
        .collect();

    for &byte in secret {
        // Polynomial f(x) with f(0) = byte and random coefficients for degree 1..threshold-1
        let mut coeffs = vec![byte];
        for _ in 1..threshold {
            coeffs.push(rng.next_u32() as u8);
        }

        for (i, share) in result.iter_mut().enumerate() {
            let x = i as u8 + 1;
            // Horner's method: f(x) = c0 + c1*x + c2*x^2 + ...
            let mut y = 0u8;
            for &c in coeffs.iter().rev() {
                y = gf_mul(y, x) ^ c;
            }
            share.push(y);
        }
    }

    result
}

/// Reconstruct the secret from a subset of shares (length >= threshold).
pub fn combine(shares: &[Vec<u8>]) -> Vec<u8> {
    assert!(!shares.is_empty(), "must provide at least one share");
    let secret_len = shares[0].len() - 1; // minus the x-coordinate byte
    let mut secret = Vec::with_capacity(secret_len);

    let xs: Vec<u8> = shares.iter().map(|s| s[0]).collect();

    for byte_idx in 0..secret_len {
        let ys: Vec<u8> = shares.iter().map(|s| s[byte_idx + 1]).collect();
        // Lagrange interpolation at x=0 over GF(256)
        let mut result = 0u8;
        for (j, &yj) in ys.iter().enumerate() {
            // numerator and denominator of Lagrange basis polynomial λ_j(0)
            let mut num = yj;
            let mut den = 1u8;
            for (k, &xk) in xs.iter().enumerate() {
                if k != j {
                    num = gf_mul(num, xk);          // multiply numerator by x_k
                    den = gf_mul(den, xs[j] ^ xk);  // multiply denom by (x_j - x_k) = x_j XOR x_k
                }
            }
            result ^= gf_mul(num, gf_inv(den));
        }
        secret.push(result);
    }

    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_combine_roundtrip() {
        let secret = b"my-master-key-32bytes-pad-000000";
        let shares = split(secret, 3, 5);
        assert_eq!(shares.len(), 5);

        // Any 3 should reconstruct
        let reconstructed = combine(&shares[0..3]);
        assert_eq!(reconstructed, secret);

        // Different subset of 3
        let reconstructed2 = combine(&[shares[1].clone(), shares[3].clone(), shares[4].clone()]);
        assert_eq!(reconstructed2, secret);
    }

    #[test]
    fn test_split_combine_threshold_2() {
        let secret = b"short";
        let shares = split(secret, 2, 3);
        let rec = combine(&shares[0..2]);
        assert_eq!(rec, secret);
    }

    #[test]
    fn test_gf_mul_identity() {
        for x in 1..=255u8 {
            assert_eq!(gf_mul(x, 1), x);
            assert_eq!(gf_mul(1, x), x);
        }
    }

    #[test]
    fn test_gf_inv() {
        for x in 1..=255u8 {
            assert_eq!(gf_mul(x, gf_inv(x)), 1);
        }
    }
}
