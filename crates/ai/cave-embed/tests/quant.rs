// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 7 — embedding quantization (fp16 + int8).
//
// infinity can return quantized embeddings to cut payload size 2x (fp16) or 4x
// (int8). fp16 is lossy IEEE-754 half; int8 is per-vector scalar quantization
// (scale = max|x| / 127) that approximately preserves cosine ranking.

use cave_embed::pooling::cosine;
use cave_embed::quant::{f16_to_f32, f32_to_f16, Int8Vector};

#[test]
fn fp16_round_trips_within_tolerance() {
    for &x in &[0.0f32, 1.0, -1.0, 0.5, -0.25, 3.14159, 65504.0] {
        let back = f16_to_f32(f32_to_f16(x));
        let tol = (x.abs() * 1e-3).max(1e-3);
        assert!((back - x).abs() <= tol, "fp16 {x} -> {back}");
    }
}

#[test]
fn fp16_zero_is_exact() {
    assert_eq!(f16_to_f32(f32_to_f16(0.0)), 0.0);
}

#[test]
fn int8_quantize_dequantize_approximates() {
    let v = vec![0.1f32, -0.5, 0.9, -0.9, 0.3];
    let q = Int8Vector::quantize(&v);
    let d = q.dequantize();
    assert_eq!(d.len(), v.len());
    for (a, b) in v.iter().zip(d.iter()) {
        // within one quantization step (scale = 0.9/127 ≈ 0.0071).
        assert!((a - b).abs() <= q.scale + 1e-6, "{a} vs {b}");
    }
}

#[test]
fn int8_is_one_byte_per_dim() {
    let v = vec![0.1f32; 384];
    let q = Int8Vector::quantize(&v);
    assert_eq!(q.data.len(), 384); // i8 = 1 byte each vs 4 for f32
}

#[test]
fn int8_zero_vector_is_safe() {
    let v = vec![0.0f32; 4];
    let q = Int8Vector::quantize(&v);
    let d = q.dequantize();
    assert!(d.iter().all(|&x| x == 0.0));
}

#[test]
fn int8_preserves_cosine_ranking() {
    let q = vec![1.0f32, 0.2, 0.0, -0.3, 0.8];
    let near = vec![0.9f32, 0.3, 0.1, -0.2, 0.7];
    let far = vec![-0.8f32, 0.1, -0.9, 0.5, -0.2];
    let qn = Int8Vector::quantize(&q).dequantize();
    let nn = Int8Vector::quantize(&near).dequantize();
    let fn_ = Int8Vector::quantize(&far).dequantize();
    assert!(cosine(&qn, &nn) > cosine(&qn, &fn_));
}
