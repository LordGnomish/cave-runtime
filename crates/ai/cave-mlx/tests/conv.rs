// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: convolution + pooling (MLX channel-last `mx.conv1d` / `mx.conv2d`).
//!
//! Layouts mirror upstream MLX exactly:
//!   * conv1d  input  `(N, L, C_in)`   weight `(C_out, K, C_in)`   -> `(N, L_out, C_out)`
//!   * conv2d  input  `(N, H, W, C_in)` weight `(C_out, KH, KW, C_in)` -> `(N, H_out, W_out, C_out)`
//!
//! with `L_out = (L + 2*pad - K)/stride + 1` (dilation = 1).

use cave_mlx::array::Array;
use cave_mlx::conv;

fn arr(data: &[f32], shape: &[usize]) -> Array {
    Array::new(data.to_vec(), shape).unwrap()
}

// ---- conv1d -------------------------------------------------------------

#[test]
fn conv1d_single_channel_no_pad() {
    // input (1,4,1) = [1,2,3,4], weight (1,2,1) = [1,1], stride 1, pad 0.
    let x = arr(&[1.0, 2.0, 3.0, 4.0], &[1, 4, 1]);
    let w = arr(&[1.0, 1.0], &[1, 2, 1]);
    let y = conv::conv1d(&x, &w, 1, 0).unwrap();
    assert_eq!(y.shape(), &[1, 3, 1]);
    assert_eq!(y.data(), &[3.0, 5.0, 7.0]); // sliding sum of adjacent pairs
}

#[test]
fn conv1d_padding_one() {
    // input (1,3,1) = [1,2,3], weight (1,2,1) = [1,1], stride 1, pad 1.
    // padded -> [0,1,2,3,0]; windows -> [0+1,1+2,2+3,3+0] = [1,3,5,3].
    let x = arr(&[1.0, 2.0, 3.0], &[1, 3, 1]);
    let w = arr(&[1.0, 1.0], &[1, 2, 1]);
    let y = conv::conv1d(&x, &w, 1, 1).unwrap();
    assert_eq!(y.shape(), &[1, 4, 1]);
    assert_eq!(y.data(), &[1.0, 3.0, 5.0, 3.0]);
}

#[test]
fn conv1d_multi_in_channel() {
    // input (1,2,2): pos0 channels [1,2], pos1 channels [3,4] -> [1,2,3,4].
    // weight (C_out=1, K=1, C_in=2) = [1,1] sums the two channels.
    let x = arr(&[1.0, 2.0, 3.0, 4.0], &[1, 2, 2]);
    let w = arr(&[1.0, 1.0], &[1, 1, 2]);
    let y = conv::conv1d(&x, &w, 1, 0).unwrap();
    assert_eq!(y.shape(), &[1, 2, 1]);
    assert_eq!(y.data(), &[3.0, 7.0]);
}

#[test]
fn conv1d_multi_out_channel_and_stride() {
    // input (1,4,1) = [1,2,3,4]; weight (C_out=2, K=2, C_in=1):
    //   out-ch0 = [1,1] (sum), out-ch1 = [1,-1] (diff). stride 2.
    // positions at stride 2: window0 [1,2], window2 [3,4].
    //   ch0: 3, 7 ; ch1: -1, -1  -> NLC interleave [3,-1, 7,-1].
    let x = arr(&[1.0, 2.0, 3.0, 4.0], &[1, 4, 1]);
    let w = arr(&[1.0, 1.0, 1.0, -1.0], &[2, 2, 1]);
    let y = conv::conv1d(&x, &w, 2, 0).unwrap();
    assert_eq!(y.shape(), &[1, 2, 2]);
    assert_eq!(y.data(), &[3.0, -1.0, 7.0, -1.0]);
}

#[test]
fn conv1d_channel_mismatch_errs() {
    let x = arr(&[1.0, 2.0], &[1, 2, 1]);
    let w = arr(&[1.0, 1.0], &[1, 1, 2]); // C_in=2 != input C_in=1
    assert!(conv::conv1d(&x, &w, 1, 0).is_err());
}

// ---- conv2d -------------------------------------------------------------

#[test]
fn conv2d_single_channel_identity_kernel() {
    // input (1,3,3,1) row-major; weight (1,2,2,1) all-ones; stride 1, pad 0.
    // H_out = W_out = 2. Each output = sum of a 2x2 patch.
    let x = arr(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        &[1, 3, 3, 1],
    );
    let w = arr(&[1.0, 1.0, 1.0, 1.0], &[1, 2, 2, 1]);
    let y = conv::conv2d(&x, &w, (1, 1), (0, 0)).unwrap();
    assert_eq!(y.shape(), &[1, 2, 2, 1]);
    // patches: [1,2,4,5]=12, [2,3,5,6]=16, [4,5,7,8]=24, [5,6,8,9]=28
    assert_eq!(y.data(), &[12.0, 16.0, 24.0, 28.0]);
}

#[test]
fn conv2d_stride_two() {
    // input (1,4,4,1) = 1..=16; weight (1,2,2,1) ones; stride 2, pad 0.
    // H_out = W_out = 2. Top-left patches at (0,0),(0,2),(2,0),(2,2).
    let data: Vec<f32> = (1..=16).map(|v| v as f32).collect();
    let x = arr(&data, &[1, 4, 4, 1]);
    let w = arr(&[1.0, 1.0, 1.0, 1.0], &[1, 2, 2, 1]);
    let y = conv::conv2d(&x, &w, (2, 2), (0, 0)).unwrap();
    assert_eq!(y.shape(), &[1, 2, 2, 1]);
    // (0,0):1+2+5+6=14  (0,2):3+4+7+8=22  (2,0):9+10+13+14=46  (2,2):11+12+15+16=54
    assert_eq!(y.data(), &[14.0, 22.0, 46.0, 54.0]);
}

#[test]
fn conv2d_padding_one_keeps_size() {
    // input (1,2,2,1) = [1,2,3,4]; weight (1,3,3,1) ones; stride 1, pad 1.
    // H_out = (2 + 2 - 3)/1 + 1 = 2. Each output sums the 3x3 zero-padded patch
    // centred on the pixel -> sum of all overlapping = full window minus borders.
    let x = arr(&[1.0, 2.0, 3.0, 4.0], &[1, 2, 2, 1]);
    let w = arr(&[1.0; 9], &[1, 3, 3, 1]);
    let y = conv::conv2d(&x, &w, (1, 1), (1, 1)).unwrap();
    assert_eq!(y.shape(), &[1, 2, 2, 1]);
    // With pad=1 every 3x3 window over a 2x2 image covers all 4 pixels -> 10 each.
    assert_eq!(y.data(), &[10.0, 10.0, 10.0, 10.0]);
}

#[test]
fn conv2d_multi_channel() {
    // input (1,2,2,2): NHWC. pixel(0,0)=[1,1] (0,1)=[2,2] (1,0)=[3,3] (1,1)=[4,4].
    // weight (C_out=1, KH=2, KW=2, C_in=2) all-ones -> sums all 8 values = 20.
    let x = arr(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0], &[1, 2, 2, 2]);
    let w = arr(&[1.0; 8], &[1, 2, 2, 2]);
    let y = conv::conv2d(&x, &w, (1, 1), (0, 0)).unwrap();
    assert_eq!(y.shape(), &[1, 1, 1, 1]);
    assert_eq!(y.data(), &[20.0]);
}

// ---- pooling ------------------------------------------------------------

#[test]
fn max_pool2d_basic() {
    // input (1,4,4,1) = 1..=16; 2x2 pool, stride 2 -> (1,2,2,1).
    let data: Vec<f32> = (1..=16).map(|v| v as f32).collect();
    let x = arr(&data, &[1, 4, 4, 1]);
    let y = conv::max_pool2d(&x, (2, 2), (2, 2));
    assert_eq!(y.shape(), &[1, 2, 2, 1]);
    // max of each 2x2 block: 6,8,14,16
    assert_eq!(y.data(), &[6.0, 8.0, 14.0, 16.0]);
}

#[test]
fn avg_pool2d_basic() {
    let data: Vec<f32> = (1..=16).map(|v| v as f32).collect();
    let x = arr(&data, &[1, 4, 4, 1]);
    let y = conv::avg_pool2d(&x, (2, 2), (2, 2));
    assert_eq!(y.shape(), &[1, 2, 2, 1]);
    // mean of each 2x2 block: (1+2+5+6)/4=3.5, 5.5, 11.5, 13.5
    assert_eq!(y.data(), &[3.5, 5.5, 11.5, 13.5]);
}

#[test]
fn max_pool2d_preserves_channels() {
    // input (1,2,2,2) -> 1x1 output keeps both channels.
    let x = arr(&[1.0, 5.0, 2.0, 6.0, 3.0, 7.0, 4.0, 8.0], &[1, 2, 2, 2]);
    let y = conv::max_pool2d(&x, (2, 2), (2, 2));
    assert_eq!(y.shape(), &[1, 1, 1, 2]);
    // channel0 max(1,2,3,4)=4, channel1 max(5,6,7,8)=8
    assert_eq!(y.data(), &[4.0, 8.0]);
}
