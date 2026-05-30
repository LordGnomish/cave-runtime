// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: N-dimensional `Array` core (MLX `mx.array` analog).

use cave_mlx::array::{Array, MlxError};

#[test]
fn new_validates_shape_against_data_len() {
    let a = Array::new(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]).unwrap();
    assert_eq!(a.shape(), &[2, 3]);
    assert_eq!(a.ndim(), 2);
    assert_eq!(a.size(), 6);
    assert_eq!(a.data(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);

    // Mismatched length is a hard error.
    let err = Array::new(vec![1.0, 2.0], &[2, 3]).unwrap_err();
    assert!(matches!(err, MlxError::ShapeMismatch { .. }));
}

#[test]
fn row_major_strides_are_computed() {
    let a = Array::new(vec![0.0; 24], &[2, 3, 4]).unwrap();
    // Row-major: stride[i] = product of dims after i.
    assert_eq!(a.strides(), &[12, 4, 1]);
}

#[test]
fn scalar_array_has_rank_zero() {
    let s = Array::from_scalar(3.5);
    assert_eq!(s.shape(), &[] as &[usize]);
    assert_eq!(s.ndim(), 0);
    assert_eq!(s.size(), 1);
    assert_eq!(s.item(), 3.5);
}

#[test]
fn zeros_and_ones_fill() {
    let z = Array::zeros(&[2, 2]);
    assert_eq!(z.data(), &[0.0, 0.0, 0.0, 0.0]);
    let o = Array::ones(&[3]);
    assert_eq!(o.data(), &[1.0, 1.0, 1.0]);
}

#[test]
fn arange_is_a_ramp() {
    let r = Array::arange(0.0, 5.0, 1.0);
    assert_eq!(r.shape(), &[5]);
    assert_eq!(r.data(), &[0.0, 1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn get_indexes_via_strides() {
    let a = Array::new(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]).unwrap();
    assert_eq!(a.get(&[0, 0]), 1.0);
    assert_eq!(a.get(&[0, 2]), 3.0);
    assert_eq!(a.get(&[1, 0]), 4.0);
    assert_eq!(a.get(&[1, 2]), 6.0);
}

#[test]
fn reshape_preserves_data_and_recomputes_strides() {
    let a = Array::new((0..12).map(|x| x as f32).collect(), &[3, 4]).unwrap();
    let b = a.reshape(&[2, 6]).unwrap();
    assert_eq!(b.shape(), &[2, 6]);
    assert_eq!(b.strides(), &[6, 1]);
    assert_eq!(b.data(), a.data());

    // -1 infers the remaining dimension.
    let c = a.reshape(&[4, 3]).unwrap();
    assert_eq!(c.shape(), &[4, 3]);

    // Non-conforming reshape errors.
    assert!(a.reshape(&[5, 5]).is_err());
}

#[test]
fn item_requires_single_element() {
    let a = Array::new(vec![42.0], &[1, 1]).unwrap();
    assert_eq!(a.item(), 42.0);
}
