// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Group-wise affine quantization — the cave-mlx analog of `mx.quantize` /
//! `mx.dequantize`.
//!
//! The last-axis (flattened) elements are partitioned into contiguous groups of
//! `group_size`. Each group is quantized independently with an affine map:
//! `code = round((x - bias) / scale)` where `bias = min(group)` and
//! `scale = (max - min) / (2^bits - 1)`. Dequantization inverts it:
//! `x ≈ code * scale + bias`. A constant group (zero range) uses `scale = 0`
//! and is reconstructed losslessly from its bias.

use crate::array::Array;

/// A quantized array: integer codes plus per-group affine `(scale, bias)`.
#[derive(Clone, Debug)]
pub struct Quantized {
    /// Bit width (e.g. 4 or 8).
    pub bits: u32,
    /// Number of elements per quantization group.
    pub group_size: usize,
    /// Original (logical) shape.
    pub shape: Vec<usize>,
    /// Quantized integer codes, one per original element, in `[0, 2^bits - 1]`.
    pub codes: Vec<u8>,
    /// Per-group scale (group index order).
    pub scales: Vec<f32>,
    /// Per-group bias / zero-point (group index order).
    pub biases: Vec<f32>,
}

/// Quantize an array group-wise to `bits` precision with the given `group_size`.
///
/// `group_size` should divide the element count; a trailing short group is
/// handled correctly. `bits` must be in `1..=8` (codes are stored as `u8`).
pub fn quantize(a: &Array, bits: u32, group_size: usize) -> Quantized {
    assert!((1..=8).contains(&bits), "bits must be in 1..=8");
    assert!(group_size > 0, "group_size must be positive");
    let data = a.data();
    let levels = ((1u32 << bits) - 1) as f32; // 2^bits - 1
    let mut codes = vec![0u8; data.len()];
    let mut scales = Vec::new();
    let mut biases = Vec::new();

    let mut start = 0usize;
    while start < data.len() {
        let end = (start + group_size).min(data.len());
        let group = &data[start..end];
        let min = group.iter().copied().fold(f32::INFINITY, f32::min);
        let max = group.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let range = max - min;
        let scale = if range > 0.0 { range / levels } else { 0.0 };
        for (i, &x) in group.iter().enumerate() {
            let code = if scale > 0.0 {
                ((x - min) / scale).round().clamp(0.0, levels) as u8
            } else {
                0
            };
            codes[start + i] = code;
        }
        scales.push(scale);
        biases.push(min);
        start = end;
    }

    Quantized {
        bits,
        group_size,
        shape: a.shape().to_vec(),
        codes,
        scales,
        biases,
    }
}

impl Quantized {
    /// Reconstruct an approximate [`Array`] from the codes and affine params.
    pub fn dequantize(&self) -> Array {
        let mut data = vec![0.0f32; self.codes.len()];
        for (i, &code) in self.codes.iter().enumerate() {
            let g = i / self.group_size;
            let scale = self.scales[g];
            let bias = self.biases[g];
            data[i] = code as f32 * scale + bias;
        }
        Array::from_parts(data, self.shape.clone())
    }

    /// Approximate compression ratio versus dense `f32` storage (codes +
    /// per-group params). Useful for reporting; not used in reconstruction.
    pub fn compression_ratio(&self) -> f32 {
        let dense_bits = self.codes.len() as f32 * 32.0;
        let code_bits = self.codes.len() as f32 * self.bits as f32;
        let param_bits = (self.scales.len() + self.biases.len()) as f32 * 32.0;
        dense_bits / (code_bits + param_bits)
    }
}
