// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: elementwise + broadcasting + reductions + matmul + activations.

use cave_mlx::array::Array;
use cave_mlx::ops;

fn arr(data: &[f32], shape: &[usize]) -> Array {
    Array::new(data.to_vec(), shape).unwrap()
}

#[test]
fn elementwise_same_shape() {
    let a = arr(&[1.0, 2.0, 3.0, 4.0], &[2, 2]);
    let b = arr(&[10.0, 20.0, 30.0, 40.0], &[2, 2]);
    assert_eq!(ops::add(&a, &b).unwrap().data(), &[11.0, 22.0, 33.0, 44.0]);
    assert_eq!(ops::sub(&b, &a).unwrap().data(), &[9.0, 18.0, 27.0, 36.0]);
    assert_eq!(ops::mul(&a, &b).unwrap().data(), &[10.0, 40.0, 90.0, 160.0]);
    assert_eq!(ops::div(&b, &a).unwrap().data(), &[10.0, 10.0, 10.0, 10.0]);
}

#[test]
fn broadcasting_row_vector_over_matrix() {
    // (2,3) + (3,) -> (2,3)
    let a = arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]);
    let bias = arr(&[10.0, 20.0, 30.0], &[3]);
    let out = ops::add(&a, &bias).unwrap();
    assert_eq!(out.shape(), &[2, 3]);
    assert_eq!(out.data(), &[11.0, 22.0, 33.0, 14.0, 25.0, 36.0]);
}

#[test]
fn broadcasting_scalar() {
    let a = arr(&[1.0, 2.0, 3.0], &[3]);
    let s = Array::from_scalar(2.0);
    assert_eq!(ops::mul(&a, &s).unwrap().data(), &[2.0, 4.0, 6.0]);
}

#[test]
fn broadcasting_incompatible_errs() {
    let a = arr(&[1.0, 2.0, 3.0], &[3]);
    let b = arr(&[1.0, 2.0], &[2]);
    assert!(ops::add(&a, &b).is_err());
}

#[test]
fn matmul_2d() {
    // (2,3) x (3,2) = (2,2)
    let a = arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]);
    let b = arr(&[7.0, 8.0, 9.0, 10.0, 11.0, 12.0], &[3, 2]);
    let c = ops::matmul(&a, &b).unwrap();
    assert_eq!(c.shape(), &[2, 2]);
    // [1*7+2*9+3*11, 1*8+2*10+3*12; 4*7+5*9+6*11, 4*8+5*10+6*12]
    assert_eq!(c.data(), &[58.0, 64.0, 139.0, 154.0]);
}

#[test]
fn matmul_inner_dim_mismatch_errs() {
    let a = arr(&[1.0, 2.0], &[1, 2]);
    let b = arr(&[1.0, 2.0, 3.0], &[3, 1]);
    assert!(ops::matmul(&a, &b).is_err());
}

#[test]
fn transpose_2d() {
    let a = arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]);
    let t = ops::transpose(&a);
    assert_eq!(t.shape(), &[3, 2]);
    assert_eq!(t.data(), &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
}

#[test]
fn sum_full_and_axis() {
    let a = arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]);
    assert_eq!(ops::sum(&a, None).item(), 21.0);
    // axis 0 -> (3,): column sums
    assert_eq!(ops::sum(&a, Some(0)).data(), &[5.0, 7.0, 9.0]);
    // axis 1 -> (2,): row sums
    assert_eq!(ops::sum(&a, Some(1)).data(), &[6.0, 15.0]);
}

#[test]
fn mean_and_max_axis() {
    let a = arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]);
    assert_eq!(ops::mean(&a, None).item(), 3.5);
    assert_eq!(ops::mean(&a, Some(1)).data(), &[2.0, 5.0]);
    assert_eq!(ops::max(&a, Some(0)).data(), &[4.0, 5.0, 6.0]);
}

#[test]
fn relu_and_sigmoid() {
    let a = arr(&[-1.0, 0.0, 2.0], &[3]);
    assert_eq!(ops::relu(&a).data(), &[0.0, 0.0, 2.0]);
    let s = ops::sigmoid(&arr(&[0.0], &[1]));
    assert!((s.data()[0] - 0.5).abs() < 1e-6);
}

#[test]
fn softmax_rows_sum_to_one() {
    let a = arr(&[1.0, 2.0, 3.0, 1.0, 1.0, 1.0], &[2, 3]);
    let sm = ops::softmax(&a, 1);
    assert_eq!(sm.shape(), &[2, 3]);
    let row0: f32 = sm.data()[0..3].iter().sum();
    let row1: f32 = sm.data()[3..6].iter().sum();
    assert!((row0 - 1.0).abs() < 1e-6);
    assert!((row1 - 1.0).abs() < 1e-6);
    // Uniform row -> uniform probabilities.
    for &p in &sm.data()[3..6] {
        assert!((p - 1.0 / 3.0).abs() < 1e-6);
    }
}

#[test]
fn exp_and_log_roundtrip() {
    let a = arr(&[1.0, 2.0, 3.0], &[3]);
    let back = ops::log(&ops::exp(&a));
    for (x, y) in a.data().iter().zip(back.data()) {
        assert!((x - y).abs() < 1e-5);
    }
}
