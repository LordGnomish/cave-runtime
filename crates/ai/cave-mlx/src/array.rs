// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! N-dimensional dense array — the cave-mlx analog of MLX's `mx.array`.
//!
//! Storage is always row-major (C-contiguous) `f32`. Unlike upstream MLX,
//! evaluation is eager: there is no lazy graph and no GPU backend. Strides are
//! retained because broadcasting and reductions in [`crate::ops`] reason about
//! them, but every public constructor returns contiguous data.

use std::fmt;

/// Error type for array construction and shape manipulation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MlxError {
    /// Data length does not match the product of the requested shape.
    #[error("shape mismatch: {data_len} elements cannot fill shape {shape:?} ({expected} expected)")]
    ShapeMismatch {
        /// Number of elements supplied.
        data_len: usize,
        /// Number of elements the shape requires.
        expected: usize,
        /// The offending shape.
        shape: Vec<usize>,
    },
    /// A reductive/elementwise op received incompatible shapes.
    #[error("incompatible shapes for {op}: {lhs:?} vs {rhs:?}")]
    Incompatible {
        /// Operation name.
        op: &'static str,
        /// Left shape.
        lhs: Vec<usize>,
        /// Right shape.
        rhs: Vec<usize>,
    },
    /// An axis argument was out of range for the array rank.
    #[error("axis {axis} out of range for rank {rank}")]
    AxisOutOfRange {
        /// The offending axis.
        axis: usize,
        /// The array rank.
        rank: usize,
    },
}

/// Row-major contiguous N-dimensional `f32` array.
#[derive(Clone, PartialEq)]
pub struct Array {
    data: Vec<f32>,
    shape: Vec<usize>,
    strides: Vec<usize>,
}

/// Compute row-major strides for a shape: `stride[i] = prod(shape[i+1..])`.
pub(crate) fn row_major_strides(shape: &[usize]) -> Vec<usize> {
    let mut strides = vec![1usize; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1];
    }
    strides
}

/// Number of elements implied by a shape (the empty shape is a scalar => 1).
pub(crate) fn shape_numel(shape: &[usize]) -> usize {
    shape.iter().product::<usize>().max(if shape.is_empty() { 1 } else { 0 })
}

impl Array {
    /// Construct an array from contiguous row-major data and an explicit shape.
    ///
    /// Returns [`MlxError::ShapeMismatch`] when `data.len()` differs from the
    /// product of `shape`.
    pub fn new(data: Vec<f32>, shape: &[usize]) -> Result<Self, MlxError> {
        let expected = shape_numel(shape);
        if data.len() != expected {
            return Err(MlxError::ShapeMismatch {
                data_len: data.len(),
                expected,
                shape: shape.to_vec(),
            });
        }
        Ok(Self {
            data,
            strides: row_major_strides(shape),
            shape: shape.to_vec(),
        })
    }

    /// Construct directly from validated parts (internal — skips re-checking).
    pub(crate) fn from_parts(data: Vec<f32>, shape: Vec<usize>) -> Self {
        let strides = row_major_strides(&shape);
        Self { data, shape, strides }
    }

    /// A rank-0 scalar array holding a single value.
    pub fn from_scalar(v: f32) -> Self {
        Self { data: vec![v], shape: vec![], strides: vec![] }
    }

    /// All-zeros array of the given shape.
    pub fn zeros(shape: &[usize]) -> Self {
        Self::from_parts(vec![0.0; shape_numel(shape)], shape.to_vec())
    }

    /// All-ones array of the given shape.
    pub fn ones(shape: &[usize]) -> Self {
        Self::from_parts(vec![1.0; shape_numel(shape)], shape.to_vec())
    }

    /// A constant-filled array of the given shape.
    pub fn full(shape: &[usize], value: f32) -> Self {
        Self::from_parts(vec![value; shape_numel(shape)], shape.to_vec())
    }

    /// A 1-D ramp `[start, start+step, ...)` stopping before `stop`.
    pub fn arange(start: f32, stop: f32, step: f32) -> Self {
        let mut data = Vec::new();
        let mut x = start;
        if step > 0.0 {
            while x < stop {
                data.push(x);
                x += step;
            }
        } else if step < 0.0 {
            while x > stop {
                data.push(x);
                x += step;
            }
        }
        let n = data.len();
        Self::from_parts(data, vec![n])
    }

    /// The array shape.
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// The row-major strides.
    pub fn strides(&self) -> &[usize] {
        &self.strides
    }

    /// The number of dimensions (rank). Scalars are rank 0.
    pub fn ndim(&self) -> usize {
        self.shape.len()
    }

    /// The total element count.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Borrow the underlying contiguous row-major buffer.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Mutable view of the underlying buffer (internal use by ops/autograd).
    pub(crate) fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// Read a single element addressed by a full multi-index.
    ///
    /// Panics if the index rank differs from the array rank — callers in this
    /// crate always supply a complete index.
    pub fn get(&self, index: &[usize]) -> f32 {
        assert_eq!(index.len(), self.shape.len(), "index rank must match array rank");
        let mut flat = 0usize;
        for (i, &ix) in index.iter().enumerate() {
            flat += ix * self.strides[i];
        }
        self.data[flat]
    }

    /// Extract the sole element of a size-1 array.
    ///
    /// Panics if the array holds more than one element.
    pub fn item(&self) -> f32 {
        assert_eq!(self.data.len(), 1, "item() requires a single-element array");
        self.data[0]
    }

    /// Return a reshaped view of the data. A single `-1` axis (encoded as
    /// `usize::MAX`) infers the remaining dimension. The element count must be
    /// preserved.
    pub fn reshape(&self, new_shape: &[usize]) -> Result<Self, MlxError> {
        let resolved = resolve_shape(new_shape, self.data.len())?;
        let expected = shape_numel(&resolved);
        if expected != self.data.len() {
            return Err(MlxError::ShapeMismatch {
                data_len: self.data.len(),
                expected,
                shape: resolved,
            });
        }
        Ok(Self::from_parts(self.data.clone(), resolved))
    }
}

/// Resolve a shape that may contain one inferred (`usize::MAX`) axis.
fn resolve_shape(shape: &[usize], numel: usize) -> Result<Vec<usize>, MlxError> {
    let infer_pos = shape.iter().position(|&d| d == usize::MAX);
    match infer_pos {
        None => Ok(shape.to_vec()),
        Some(pos) => {
            let known: usize = shape.iter().filter(|&&d| d != usize::MAX).product();
            if known == 0 || numel % known != 0 {
                return Err(MlxError::ShapeMismatch {
                    data_len: numel,
                    expected: known,
                    shape: shape.to_vec(),
                });
            }
            let mut out = shape.to_vec();
            out[pos] = numel / known;
            Ok(out)
        }
    }
}

impl fmt::Debug for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Array(shape={:?}, data={:?})", self.shape, self.data)
    }
}
