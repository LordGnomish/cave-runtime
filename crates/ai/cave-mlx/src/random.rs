// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pseudo-random sampling — the cave-mlx analog of `mx.random`.
//!
//! Upstream MLX builds `mx.random` on a counter-based Threefry2x32 generator
//! (the Random123 algorithm) keyed by an explicit `mx.random.key`. This module
//! ports that primitive faithfully — [`threefry2x32`] reproduces the canonical
//! Random123 known-answer vectors — and layers the user-facing distribution
//! suite on top: [`uniform`], [`normal`], [`bernoulli`], [`randint`],
//! [`truncated_normal`], and [`categorical`].
//!
//! A [`Key`] holds the 2×u32 counter-PRNG key. [`Key::split`] derives
//! independent sub-streams deterministically, mirroring `mx.random.split`.
//! Sampling is therefore pure and reproducible: the same key always yields the
//! same array.
//!
//! Divergence note (kept honest): the *element fill order* — i.e. how array
//! positions map onto Threefry counter blocks — is cave-mlx's own scheme, so a
//! whole-array draw is not byte-for-byte identical to an equivalent upstream
//! `mx.random` call. The PRNG primitive and the distribution semantics match;
//! the per-element counter layout is implementation-local.

use crate::array::Array;

/// A counter-PRNG key: the 2×u32 Threefry key state (`mx.random.key` analog).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    state: [u32; 2],
}

impl Key {
    /// Build a key from a 64-bit seed (`mx.random.key(seed)` analog).
    ///
    /// The seed is split into the two 32-bit key words.
    pub fn new(seed: u64) -> Self {
        Self { state: [seed as u32, (seed >> 32) as u32] }
    }

    /// Derive `num` independent sub-keys (`mx.random.split` analog).
    ///
    /// Each sub-key is produced by running Threefry over a dedicated counter
    /// domain, so the resulting streams are decorrelated from each other and
    /// from the parent.
    pub fn split(&self, num: usize) -> Vec<Key> {
        (0..num)
            .map(|i| Key { state: threefry2x32(self.state, [0xDEAD_BEEF, i as u32]) })
            .collect()
    }
}

/// Threefry2x32 with 20 rounds — the Random123 counter-based PRNG primitive.
///
/// Verified against the canonical `kat_vectors.txt` known-answer vectors.
/// `key` and `ctr` are each a pair of 32-bit words; the output is a pair of
/// statistically-independent 32-bit words.
pub fn threefry2x32(key: [u32; 2], ctr: [u32; 2]) -> [u32; 2] {
    // Per-round left-rotation amounts for the 2×32 variant (R_32x2_*).
    const ROT: [u32; 8] = [13, 15, 26, 6, 17, 29, 16, 24];
    // Skein key-schedule parity constant.
    const PARITY: u32 = 0x1BD1_1BDA;

    let ks = [key[0], key[1], PARITY ^ key[0] ^ key[1]];
    let mut x0 = ctr[0].wrapping_add(ks[0]);
    let mut x1 = ctr[1].wrapping_add(ks[1]);

    for round in 0..20u32 {
        x0 = x0.wrapping_add(x1);
        x1 = x1.rotate_left(ROT[(round % 8) as usize]);
        x1 ^= x0;
        // Inject the key schedule after every 4 rounds.
        if (round + 1) % 4 == 0 {
            let j = (round + 1) / 4; // 1..=5
            x0 = x0.wrapping_add(ks[(j % 3) as usize]);
            x1 = x1.wrapping_add(ks[((j + 1) % 3) as usize]).wrapping_add(j);
        }
    }
    [x0, x1]
}

/// Produce `n` raw 32-bit words from the key by running Threefry over the
/// counter sequence `0, 1, 2, …`, two words per block.
fn keystream(key: &Key, n: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(n);
    let mut block: u64 = 0;
    while out.len() < n {
        let ctr = [block as u32, (block >> 32) as u32];
        let pair = threefry2x32(key.state, ctr);
        out.push(pair[0]);
        if out.len() < n {
            out.push(pair[1]);
        }
        block += 1;
    }
    out
}

/// Map a 32-bit word to a float in `[0, 1)` using its top 24 bits (the f32
/// mantissa width), avoiding the rounding bias of a naive `bits / 2^32`.
#[inline]
fn to_unit(bits: u32) -> f32 {
    (bits >> 8) as f32 / (1u32 << 24) as f32
}

/// Number of elements implied by a shape (empty shape ⇒ scalar, 1 element).
fn numel(shape: &[usize]) -> usize {
    shape.iter().product::<usize>().max(if shape.is_empty() { 1 } else { 0 })
}

/// Uniform samples over `[low, high)` with the given shape (`mx.random.uniform`).
pub fn uniform(key: &Key, low: f32, high: f32, shape: &[usize]) -> Array {
    let n = numel(shape);
    let span = high - low;
    let data: Vec<f32> = keystream(key, n).into_iter().map(|b| low + span * to_unit(b)).collect();
    Array::from_parts(data, shape.to_vec())
}

/// Map a unit float in `[0, 1)` into the open interval `(0, 1)` so that
/// `ln(u)` and `ln(1 - u)` are always finite (needed by Box-Muller / Gumbel).
#[inline]
fn open_unit(u: f32) -> f32 {
    // Nudge endpoints to half an ULP-scale interior; `to_unit` produces values
    // in [0, 1) on a 2^-24 grid, so this keeps every draw strictly interior.
    const EPS: f32 = 1.0 / (1u32 << 25) as f32;
    u.clamp(EPS, 1.0 - EPS)
}

/// Normal (Gaussian) samples with mean `loc` and standard deviation `scale`
/// (`mx.random.normal`). Implemented with the Box-Muller transform.
pub fn normal(key: &Key, loc: f32, scale: f32, shape: &[usize]) -> Array {
    let n = numel(shape);
    // Two uniforms per Gaussian; round the request up to an even count.
    let bits = keystream(key, n.div_ceil(2) * 2);
    let two_pi = std::f32::consts::TAU;
    let mut data = Vec::with_capacity(n);
    let mut i = 0;
    while data.len() < n {
        let u1 = open_unit(to_unit(bits[i]));
        let u2 = to_unit(bits[i + 1]);
        let r = (-2.0 * u1.ln()).sqrt();
        let z0 = r * (two_pi * u2).cos();
        let z1 = r * (two_pi * u2).sin();
        data.push(loc + scale * z0);
        if data.len() < n {
            data.push(loc + scale * z1);
        }
        i += 2;
    }
    Array::from_parts(data, shape.to_vec())
}

/// Bernoulli samples: `1.0` with probability `p`, else `0.0`
/// (`mx.random.bernoulli`).
pub fn bernoulli(key: &Key, p: f32, shape: &[usize]) -> Array {
    let n = numel(shape);
    let data: Vec<f32> = keystream(key, n)
        .into_iter()
        .map(|b| if to_unit(b) < p { 1.0 } else { 0.0 })
        .collect();
    Array::from_parts(data, shape.to_vec())
}

/// Integer samples over the half-open range `[low, high)`, returned as `f32`
/// (`mx.random.randint`; cave-mlx arrays are f32-typed).
pub fn randint(key: &Key, low: i64, high: i64, shape: &[usize]) -> Array {
    assert!(high > low, "randint requires high > low");
    let span = (high - low) as u64;
    let n = numel(shape);
    let data: Vec<f32> = keystream(key, n)
        .into_iter()
        .map(|b| (low + (b as u64 % span) as i64) as f32)
        .collect();
    Array::from_parts(data, shape.to_vec())
}

/// Gauss error function `erf(x)` — Abramowitz & Stegun 7.1.26 approximation
/// (max abs error ≈ 1.5e-7, well within f32 precision).
#[allow(clippy::excessive_precision)] // canonical A&S coefficients kept verbatim
fn erf(x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

/// Inverse error function `erfinv(x)` for `x ∈ (-1, 1)` — Giles (2010)
/// single-precision polynomial approximation.
#[allow(clippy::excessive_precision)] // canonical Giles coefficients kept verbatim
fn erfinv(x: f32) -> f32 {
    let w = -((1.0 - x) * (1.0 + x)).ln();
    let p = if w < 5.0 {
        let w = w - 2.5;
        let mut p = 2.810_226_36e-08;
        p = 3.432_739_39e-07 + p * w;
        p = -3.523_387_7e-06 + p * w;
        p = -4.391_506_54e-06 + p * w;
        p = 0.000_218_580_87 + p * w;
        p = -0.001_253_725_03 + p * w;
        p = -0.004_177_681_64 + p * w;
        p = 0.246_640_727 + p * w;
        1.501_409_41 + p * w
    } else {
        let w = w.sqrt() - 3.0;
        let mut p = -0.000_200_214_257;
        p = 0.000_100_950_558 + p * w;
        p = 0.001_349_343_22 + p * w;
        p = -0.003_673_428_44 + p * w;
        p = 0.005_739_507_73 + p * w;
        p = -0.007_622_461_3 + p * w;
        p = 0.009_438_870_47 + p * w;
        p = 1.001_674_06 + p * w;
        2.832_976_82 + p * w
    };
    p * x
}

/// Standard-normal CDF: `Φ(x) = ½(1 + erf(x/√2))`.
#[inline]
fn std_normal_cdf(x: f32) -> f32 {
    0.5 * (1.0 + erf(x * std::f32::consts::FRAC_1_SQRT_2))
}

/// Samples from a standard normal truncated to `[lower, upper]`
/// (`mx.random.truncated_normal`). Uses the exact inverse-CDF method (no
/// rejection): draw `u` uniformly in `[Φ(lower), Φ(upper)]`, return `Φ⁻¹(u)`.
pub fn truncated_normal(key: &Key, lower: f32, upper: f32, shape: &[usize]) -> Array {
    assert!(upper > lower, "truncated_normal requires upper > lower");
    let lo = std_normal_cdf(lower);
    let hi = std_normal_cdf(upper);
    let span = hi - lo;
    let sqrt2 = std::f32::consts::SQRT_2;
    let n = numel(shape);
    let data: Vec<f32> = keystream(key, n)
        .into_iter()
        .map(|b| {
            let u = lo + span * to_unit(b);
            // Φ⁻¹(u) = √2 · erfinv(2u − 1); clamp to the requested bounds to
            // absorb the approximation's tail error.
            (sqrt2 * erfinv((2.0 * u - 1.0).clamp(-0.999_999, 0.999_999)))
                .clamp(lower, upper)
        })
        .collect();
    Array::from_parts(data, shape.to_vec())
}

/// Samples class indices from unnormalized `logits` along the last axis via the
/// Gumbel-max trick (`mx.random.categorical`). The result drops the last axis:
/// `(…, num_classes)` logits → `(…)` integer indices (as `f32`).
pub fn categorical(key: &Key, logits: &Array) -> Array {
    let shape = logits.shape();
    assert!(!shape.is_empty(), "categorical requires at least a 1-D logits array");
    let classes = *shape.last().unwrap();
    assert!(classes > 0, "categorical requires a non-empty class axis");
    let rows: usize = shape[..shape.len() - 1].iter().product::<usize>().max(1);
    let data = logits.data();

    // One Gumbel sample per logit element.
    let bits = keystream(key, rows * classes);
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let base = r * classes;
        let mut best = f32::NEG_INFINITY;
        let mut best_idx = 0usize;
        for c in 0..classes {
            let u = open_unit(to_unit(bits[base + c]));
            // Gumbel(0,1) noise: g = -ln(-ln(u)).
            let g = -(-u.ln()).ln();
            let score = data[base + c] + g;
            if score > best {
                best = score;
                best_idx = c;
            }
        }
        out.push(best_idx as f32);
    }
    Array::from_parts(out, shape[..shape.len() - 1].to_vec())
}
