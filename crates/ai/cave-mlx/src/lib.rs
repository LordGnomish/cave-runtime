// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-mlx — a pure-Rust subset of Apple's MLX array framework.
//!
//! Parity target: ml-explore/mlx (MIT). This crate ports the *array
//! programming* core of MLX — an N-dimensional tensor type, elementwise and
//! reduction ops, matmul, activations, reverse-mode automatic
//! differentiation, neural-network modules, and optimizers — onto a
//! dependency-light CPU backend.
//!
//! MLX's lazy-evaluation graph and Metal/GPU kernels are deliberately *not*
//! ported: this is a sovereign, cross-platform CPU implementation. Eager
//! evaluation is used throughout; the autograd module records its own tape.
//!
//! Module map (built strict-TDD, lowest layer first):
//!   * [`array`]   — N-dim `Array<f32>` (shape/strides/contiguous storage).
//!   * [`ops`]     — elementwise, broadcasting, reductions, matmul, activations.
//!   * [`autograd`]— reverse-mode autodiff over a recorded tape.
//!   * [`nn`]      — `Linear` + activation layers.
//!   * [`optim`]   — `Sgd` / `Adam` / `AdamW`.

#![forbid(unsafe_code)]

/// N-dimensional dense array (`Array`) — the MLX `mx.array` analog.
pub mod array;

/// Eager array operations — elementwise, broadcast, reductions, matmul.
pub mod ops;

/// Reverse-mode automatic differentiation (`Tape` / `Var`).
pub mod autograd;

/// First-order optimizers — `Sgd` / `Adam` / `AdamW`.
pub mod optim;

/// Neural-network modules — `Linear` + `Activation`.
pub mod nn;

/// Group-wise affine quantization — `mx.quantize` / `mx.dequantize` analog.
pub mod quant;

/// Convolution + pooling — `mx.conv1d` / `mx.conv2d` and pooling layers.
pub mod conv;
