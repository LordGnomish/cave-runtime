// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Eager array operations: elementwise (with NumPy/MLX broadcasting),
//! reductions, matrix multiply, transpose, and elementwise activations.
//!
//! Every function here is a pure value-to-value transform with no autograd
//! bookkeeping; the [`crate::autograd`] layer composes these primitives and
//! records its own tape.

use crate::array::{row_major_strides, shape_numel, Array, MlxError};

/// Broadcast two shapes per NumPy rules, returning the result shape.
///
/// Trailing dimensions are aligned; each pair must be equal or one must be 1.
pub fn broadcast_shapes(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let rank = a.len().max(b.len());
    let mut out = vec![0usize; rank];
    for i in 0..rank {
        let da = if i < rank - a.len() { 1 } else { a[i - (rank - a.len())] };
        let db = if i < rank - b.len() { 1 } else { b[i - (rank - b.len())] };
        if da == db || da == 1 || db == 1 {
            out[i] = da.max(db);
        } else {
            return None;
        }
    }
    Some(out)
}

/// Map a flat output index (into `out_shape`) back to the flat source index of
/// an operand with `src_shape`, honoring broadcasting (size-1 axes repeat).
fn broadcast_src_index(out_index: &[usize], src_shape: &[usize], src_strides: &[usize]) -> usize {
    let rank = out_index.len();
    let offset = rank - src_shape.len();
    let mut flat = 0usize;
    for (axis, dim) in src_shape.iter().enumerate() {
        let coord = if *dim == 1 { 0 } else { out_index[offset + axis] };
        flat += coord * src_strides[axis];
    }
    flat
}

/// Iterate every multi-index of `shape` in row-major order.
fn for_each_index(shape: &[usize], mut f: impl FnMut(&[usize], usize)) {
    let n = shape_numel(shape);
    let mut index = vec![0usize; shape.len()];
    for flat in 0..n {
        f(&index, flat);
        // increment row-major odometer
        for axis in (0..shape.len()).rev() {
            index[axis] += 1;
            if index[axis] < shape[axis] {
                break;
            }
            index[axis] = 0;
        }
    }
}

/// Generic broadcasting elementwise binary op.
fn binary(a: &Array, b: &Array, op: &'static str, f: impl Fn(f32, f32) -> f32) -> Result<Array, MlxError> {
    let out_shape = broadcast_shapes(a.shape(), b.shape()).ok_or_else(|| MlxError::Incompatible {
        op,
        lhs: a.shape().to_vec(),
        rhs: b.shape().to_vec(),
    })?;
    let a_strides = row_major_strides(a.shape());
    let b_strides = row_major_strides(b.shape());
    let mut data = vec![0.0f32; shape_numel(&out_shape)];
    for_each_index(&out_shape, |index, flat| {
        let av = a.data()[broadcast_src_index(index, a.shape(), &a_strides)];
        let bv = b.data()[broadcast_src_index(index, b.shape(), &b_strides)];
        data[flat] = f(av, bv);
    });
    Ok(Array::from_parts(data, out_shape))
}

/// Elementwise addition with broadcasting.
pub fn add(a: &Array, b: &Array) -> Result<Array, MlxError> {
    binary(a, b, "add", |x, y| x + y)
}

/// Elementwise subtraction with broadcasting.
pub fn sub(a: &Array, b: &Array) -> Result<Array, MlxError> {
    binary(a, b, "sub", |x, y| x - y)
}

/// Elementwise multiplication with broadcasting.
pub fn mul(a: &Array, b: &Array) -> Result<Array, MlxError> {
    binary(a, b, "mul", |x, y| x * y)
}

/// Elementwise division with broadcasting.
pub fn div(a: &Array, b: &Array) -> Result<Array, MlxError> {
    binary(a, b, "div", |x, y| x / y)
}

/// Negate every element.
pub fn neg(a: &Array) -> Array {
    map(a, |x| -x)
}

/// Scale every element by a scalar.
pub fn scalar_mul(a: &Array, s: f32) -> Array {
    map(a, |x| x * s)
}

/// Add a scalar to every element.
pub fn scalar_add(a: &Array, s: f32) -> Array {
    map(a, |x| x + s)
}

/// Apply a unary function elementwise.
pub fn map(a: &Array, f: impl Fn(f32) -> f32) -> Array {
    Array::from_parts(a.data().iter().copied().map(f).collect(), a.shape().to_vec())
}

/// Elementwise exponential.
pub fn exp(a: &Array) -> Array {
    map(a, f32::exp)
}

/// Elementwise natural logarithm.
pub fn log(a: &Array) -> Array {
    map(a, f32::ln)
}

/// Elementwise square root.
pub fn sqrt(a: &Array) -> Array {
    map(a, f32::sqrt)
}

/// ReLU activation: `max(0, x)`.
pub fn relu(a: &Array) -> Array {
    map(a, |x| x.max(0.0))
}

/// Logistic sigmoid: `1 / (1 + e^-x)`.
pub fn sigmoid(a: &Array) -> Array {
    map(a, |x| 1.0 / (1.0 + (-x).exp()))
}

/// Hyperbolic-tangent activation.
pub fn tanh(a: &Array) -> Array {
    map(a, f32::tanh)
}

/// 2-D matrix multiply `(m,k) x (k,n) -> (m,n)`.
pub fn matmul(a: &Array, b: &Array) -> Result<Array, MlxError> {
    if a.ndim() != 2 || b.ndim() != 2 {
        return Err(MlxError::Incompatible {
            op: "matmul (only 2-D supported)",
            lhs: a.shape().to_vec(),
            rhs: b.shape().to_vec(),
        });
    }
    let (m, k) = (a.shape()[0], a.shape()[1]);
    let (k2, n) = (b.shape()[0], b.shape()[1]);
    if k != k2 {
        return Err(MlxError::Incompatible {
            op: "matmul",
            lhs: a.shape().to_vec(),
            rhs: b.shape().to_vec(),
        });
    }
    let ad = a.data();
    let bd = b.data();
    let mut out = vec![0.0f32; m * n];
    for i in 0..m {
        for p in 0..k {
            let aip = ad[i * k + p];
            if aip == 0.0 {
                continue;
            }
            let brow = p * n;
            let orow = i * n;
            for j in 0..n {
                out[orow + j] += aip * bd[brow + j];
            }
        }
    }
    Ok(Array::from_parts(out, vec![m, n]))
}

/// Transpose a 2-D array (swap the two axes). Higher ranks reverse all axes.
pub fn transpose(a: &Array) -> Array {
    let shape = a.shape();
    if shape.len() < 2 {
        return a.clone();
    }
    let rev: Vec<usize> = shape.iter().rev().copied().collect();
    let in_strides = row_major_strides(shape);
    let mut data = vec![0.0f32; a.size()];
    // out axis j corresponds to input axis (rank-1-j)
    for_each_index(&rev, |out_index, flat| {
        let mut src = 0usize;
        for (j, &coord) in out_index.iter().enumerate() {
            let in_axis = shape.len() - 1 - j;
            src += coord * in_strides[in_axis];
        }
        data[flat] = a.data()[src];
    });
    Array::from_parts(data, rev)
}

/// Reduce along `axis` (or the whole array when `axis` is `None`) with an
/// associative accumulator seeded by `init`, then optionally post-process
/// (e.g. divide by count for mean).
fn reduce(
    a: &Array,
    axis: Option<usize>,
    init: f32,
    acc: impl Fn(f32, f32) -> f32,
    finish: impl Fn(f32, usize) -> f32,
) -> Array {
    match axis {
        None => {
            let mut r = init;
            for &x in a.data() {
                r = acc(r, x);
            }
            Array::from_scalar(finish(r, a.size().max(1)))
        }
        Some(ax) => {
            let shape = a.shape();
            let in_strides = row_major_strides(shape);
            let axis_len = shape[ax];
            let mut out_shape: Vec<usize> = shape.to_vec();
            out_shape.remove(ax);
            let out_n = shape_numel(&out_shape);
            let mut data = vec![init; out_n];
            // For each output cell, walk the reduced axis.
            for_each_index(&out_shape, |out_index, flat| {
                let mut full = Vec::with_capacity(shape.len());
                full.extend_from_slice(&out_index[..ax]);
                full.push(0);
                full.extend_from_slice(&out_index[ax..]);
                let mut r = init;
                for t in 0..axis_len {
                    full[ax] = t;
                    let mut src = 0usize;
                    for (d, &c) in full.iter().enumerate() {
                        src += c * in_strides[d];
                    }
                    r = acc(r, a.data()[src]);
                }
                data[flat] = finish(r, axis_len);
            });
            Array::from_parts(data, out_shape)
        }
    }
}

/// Sum reduction.
pub fn sum(a: &Array, axis: Option<usize>) -> Array {
    reduce(a, axis, 0.0, |r, x| r + x, |r, _| r)
}

/// Mean reduction.
pub fn mean(a: &Array, axis: Option<usize>) -> Array {
    reduce(a, axis, 0.0, |r, x| r + x, |r, n| r / n as f32)
}

/// Maximum reduction.
pub fn max(a: &Array, axis: Option<usize>) -> Array {
    reduce(a, axis, f32::NEG_INFINITY, f32::max, |r, _| r)
}

/// Numerically-stable softmax along `axis`.
pub fn softmax(a: &Array, axis: usize) -> Array {
    let shape = a.shape();
    let in_strides = row_major_strides(shape);
    let axis_len = shape[axis];
    let mut out = vec![0.0f32; a.size()];
    let mut out_shape: Vec<usize> = shape.to_vec();
    out_shape.remove(axis);
    for_each_index(&out_shape, |out_index, _| {
        let mut full = Vec::with_capacity(shape.len());
        full.extend_from_slice(&out_index[..axis]);
        full.push(0);
        full.extend_from_slice(&out_index[axis..]);
        // pass 1: max
        let mut m = f32::NEG_INFINITY;
        for t in 0..axis_len {
            full[axis] = t;
            m = m.max(a.data()[flat_index(&full, &in_strides)]);
        }
        // pass 2: exp + sum
        let mut s = 0.0f32;
        let mut exps = vec![0.0f32; axis_len];
        for t in 0..axis_len {
            full[axis] = t;
            let e = (a.data()[flat_index(&full, &in_strides)] - m).exp();
            exps[t] = e;
            s += e;
        }
        // pass 3: normalize into the contiguous output buffer
        for t in 0..axis_len {
            full[axis] = t;
            out[flat_index(&full, &in_strides)] = exps[t] / s;
        }
    });
    Array::from_parts(out, shape.to_vec())
}

/// Flatten a full multi-index against the supplied strides.
fn flat_index(index: &[usize], strides: &[usize]) -> usize {
    index.iter().zip(strides).map(|(c, s)| c * s).sum()
}
