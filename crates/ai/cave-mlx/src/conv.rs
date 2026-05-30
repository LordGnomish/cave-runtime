// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Convolution and pooling — the cave-mlx analog of `mx.conv1d` / `mx.conv2d`
//! and the `mlx.nn` pooling layers.
//!
//! Layouts mirror upstream MLX exactly. MLX is **channel-last**:
//!   * conv1d  input `(N, L, C_in)`, weight `(C_out, K, C_in)`,
//!     output `(N, L_out, C_out)`.
//!   * conv2d  input `(N, H, W, C_in)`, weight `(C_out, KH, KW, C_in)`,
//!     output `(N, H_out, W_out, C_out)`.
//!
//! Spatial output size follows `out = (in + 2*pad - kernel)/stride + 1` with
//! dilation fixed at 1. Zero-padding is implicit (reads outside the input read
//! as `0.0`). Evaluation is eager and direct (no im2col/winograd) — this is the
//! sovereign CPU reference, not a performance kernel.

use crate::array::{Array, MlxError};

/// 1-D cross-correlation (the operation deep-learning frameworks call
/// "convolution"). See the [module docs](self) for layouts.
///
/// Returns [`MlxError::Incompatible`] when the ranks are wrong or the weight's
/// input-channel count does not match the input's.
pub fn conv1d(
    input: &Array,
    weight: &Array,
    stride: usize,
    padding: usize,
) -> Result<Array, MlxError> {
    if input.ndim() != 3 || weight.ndim() != 3 {
        return Err(MlxError::Incompatible {
            op: "conv1d (expects rank-3 (N,L,C_in) and (C_out,K,C_in))",
            lhs: input.shape().to_vec(),
            rhs: weight.shape().to_vec(),
        });
    }
    let (n, l, c_in) = (input.shape()[0], input.shape()[1], input.shape()[2]);
    let (c_out, k, w_c_in) = (weight.shape()[0], weight.shape()[1], weight.shape()[2]);
    if w_c_in != c_in {
        return Err(MlxError::Incompatible {
            op: "conv1d (channel mismatch)",
            lhs: input.shape().to_vec(),
            rhs: weight.shape().to_vec(),
        });
    }
    let l_out = match out_dim(l, k, stride, padding) {
        Some(v) => v,
        None => {
            return Err(MlxError::Incompatible {
                op: "conv1d (kernel larger than padded input)",
                lhs: input.shape().to_vec(),
                rhs: weight.shape().to_vec(),
            })
        }
    };

    let xd = input.data();
    let wd = weight.data();
    let mut out = vec![0.0f32; n * l_out * c_out];
    for ni in 0..n {
        for lo in 0..l_out {
            for co in 0..c_out {
                let mut acc = 0.0f32;
                for kk in 0..k {
                    // input position (may be negative under padding -> skip).
                    let li = lo * stride + kk;
                    if li < padding || li - padding >= l {
                        continue;
                    }
                    let li = li - padding;
                    for ci in 0..c_in {
                        let x = xd[(ni * l + li) * c_in + ci];
                        let wv = wd[(co * k + kk) * c_in + ci];
                        acc += x * wv;
                    }
                }
                out[(ni * l_out + lo) * c_out + co] = acc;
            }
        }
    }
    Ok(Array::from_parts(out, vec![n, l_out, c_out]))
}

/// 2-D cross-correlation. See the [module docs](self) for layouts.
pub fn conv2d(
    input: &Array,
    weight: &Array,
    stride: (usize, usize),
    padding: (usize, usize),
) -> Result<Array, MlxError> {
    if input.ndim() != 4 || weight.ndim() != 4 {
        return Err(MlxError::Incompatible {
            op: "conv2d (expects rank-4 (N,H,W,C_in) and (C_out,KH,KW,C_in))",
            lhs: input.shape().to_vec(),
            rhs: weight.shape().to_vec(),
        });
    }
    let (n, h, w, c_in) = (
        input.shape()[0],
        input.shape()[1],
        input.shape()[2],
        input.shape()[3],
    );
    let (c_out, kh, kw, w_c_in) = (
        weight.shape()[0],
        weight.shape()[1],
        weight.shape()[2],
        weight.shape()[3],
    );
    if w_c_in != c_in {
        return Err(MlxError::Incompatible {
            op: "conv2d (channel mismatch)",
            lhs: input.shape().to_vec(),
            rhs: weight.shape().to_vec(),
        });
    }
    let (sh, sw) = stride;
    let (ph, pw) = padding;
    let (h_out, w_out) = match (out_dim(h, kh, sh, ph), out_dim(w, kw, sw, pw)) {
        (Some(ho), Some(wo)) => (ho, wo),
        _ => {
            return Err(MlxError::Incompatible {
                op: "conv2d (kernel larger than padded input)",
                lhs: input.shape().to_vec(),
                rhs: weight.shape().to_vec(),
            })
        }
    };

    let xd = input.data();
    let wd = weight.data();
    let mut out = vec![0.0f32; n * h_out * w_out * c_out];
    for ni in 0..n {
        for ho in 0..h_out {
            for wo in 0..w_out {
                for co in 0..c_out {
                    let mut acc = 0.0f32;
                    for ki in 0..kh {
                        let hi = ho * sh + ki;
                        if hi < ph || hi - ph >= h {
                            continue;
                        }
                        let hi = hi - ph;
                        for kj in 0..kw {
                            let wi = wo * sw + kj;
                            if wi < pw || wi - pw >= w {
                                continue;
                            }
                            let wi = wi - pw;
                            for ci in 0..c_in {
                                let x = xd[((ni * h + hi) * w + wi) * c_in + ci];
                                let wv = wd[((co * kh + ki) * kw + kj) * c_in + ci];
                                acc += x * wv;
                            }
                        }
                    }
                    out[((ni * h_out + ho) * w_out + wo) * c_out + co] = acc;
                }
            }
        }
    }
    Ok(Array::from_parts(out, vec![n, h_out, w_out, c_out]))
}

/// 2-D max pooling over `(N, H, W, C)` input. No padding; each channel is
/// pooled independently.
pub fn max_pool2d(input: &Array, kernel: (usize, usize), stride: (usize, usize)) -> Array {
    pool2d(input, kernel, stride, f32::NEG_INFINITY, f32::max, |acc, _| acc)
}

/// 2-D average pooling over `(N, H, W, C)` input. No padding.
pub fn avg_pool2d(input: &Array, kernel: (usize, usize), stride: (usize, usize)) -> Array {
    pool2d(input, kernel, stride, 0.0, |a, b| a + b, |acc, n| acc / n as f32)
}

/// Shared spatial-window reducer for pooling.
fn pool2d(
    input: &Array,
    kernel: (usize, usize),
    stride: (usize, usize),
    init: f32,
    acc_fn: impl Fn(f32, f32) -> f32,
    finish: impl Fn(f32, usize) -> f32,
) -> Array {
    assert_eq!(input.ndim(), 4, "pool2d expects rank-4 (N,H,W,C)");
    let (n, h, w, c) = (
        input.shape()[0],
        input.shape()[1],
        input.shape()[2],
        input.shape()[3],
    );
    let (kh, kw) = kernel;
    let (sh, sw) = stride;
    let h_out = out_dim(h, kh, sh, 0).expect("pool kernel larger than input");
    let w_out = out_dim(w, kw, sw, 0).expect("pool kernel larger than input");

    let xd = input.data();
    let mut out = vec![0.0f32; n * h_out * w_out * c];
    for ni in 0..n {
        for ho in 0..h_out {
            for wo in 0..w_out {
                for ci in 0..c {
                    let mut r = init;
                    for ki in 0..kh {
                        let hi = ho * sh + ki;
                        for kj in 0..kw {
                            let wi = wo * sw + kj;
                            let x = xd[((ni * h + hi) * w + wi) * c + ci];
                            r = acc_fn(r, x);
                        }
                    }
                    out[((ni * h_out + ho) * w_out + wo) * c + ci] = finish(r, kh * kw);
                }
            }
        }
    }
    Array::from_parts(out, vec![n, h_out, w_out, c])
}

/// Output spatial extent `(in + 2*pad - kernel)/stride + 1`, or `None` when the
/// (padded) kernel does not fit or `stride == 0`.
fn out_dim(input: usize, kernel: usize, stride: usize, pad: usize) -> Option<usize> {
    if stride == 0 {
        return None;
    }
    let padded = input + 2 * pad;
    if kernel == 0 || kernel > padded {
        return None;
    }
    Some((padded - kernel) / stride + 1)
}
