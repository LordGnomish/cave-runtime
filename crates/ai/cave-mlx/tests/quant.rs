// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: group-wise affine quantization (MLX mx.quantize/dequantize).

use cave_mlx::array::Array;
use cave_mlx::quant::{quantize, Quantized};

fn arr(data: &[f32], shape: &[usize]) -> Array {
    Array::new(data.to_vec(), shape).unwrap()
}

#[test]
fn eight_bit_roundtrip_is_near_exact() {
    let a = arr(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], &[8]);
    let q = quantize(&a, 8, 8);
    let back = q.dequantize();
    assert_eq!(back.shape(), &[8]);
    for (x, y) in a.data().iter().zip(back.data()) {
        assert!((x - y).abs() < 0.05, "8-bit recon off: {x} vs {y}");
    }
}

#[test]
fn codes_fit_in_bit_width() {
    let a = arr(&[0.0, 1.0, 2.0, 3.0, 10.0, 20.0, 30.0, 40.0], &[8]);
    let q4 = quantize(&a, 4, 8);
    assert_eq!(q4.bits, 4);
    // 4-bit codes are in [0, 15].
    assert!(q4.codes.iter().all(|&c| c <= 15));
    // 8-bit must exploit more levels than 4-bit on this spread-out data.
    let q8 = quantize(&a, 8, 8);
    assert!(q8.codes.iter().any(|&c| c > 15));
}

#[test]
fn grouping_produces_per_group_scales() {
    // Two groups of 4 with very different ranges quantize independently.
    let a = arr(&[0.0, 1.0, 2.0, 3.0, 100.0, 200.0, 300.0, 400.0], &[8]);
    let q = quantize(&a, 8, 4);
    assert_eq!(q.scales.len(), 2);
    assert_eq!(q.biases.len(), 2);
    // The second group's scale must be much larger than the first's.
    assert!(q.scales[1] > q.scales[0] * 10.0);
    let back = q.dequantize();
    for (x, y) in a.data().iter().zip(back.data()) {
        let tol = (x.abs() * 0.01).max(0.5);
        assert!((x - y).abs() <= tol, "grouped recon off: {x} vs {y}");
    }
}

#[test]
fn constant_group_is_lossless() {
    // A flat group has zero range; dequantize must return the constant.
    let a = arr(&[5.0, 5.0, 5.0, 5.0], &[4]);
    let q = quantize(&a, 4, 4);
    let back = q.dequantize();
    for &y in back.data() {
        assert!((y - 5.0).abs() < 1e-6);
    }
}

#[test]
fn quantized_struct_records_shape() {
    let a = arr(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], &[2, 3]);
    let q: Quantized = quantize(&a, 8, 3);
    assert_eq!(q.shape, vec![2, 3]);
    assert_eq!(q.group_size, 3);
    assert_eq!(q.dequantize().shape(), &[2, 3]);
}
