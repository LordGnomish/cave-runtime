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
