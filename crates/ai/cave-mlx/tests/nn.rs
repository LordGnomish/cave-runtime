// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: neural-network modules (`Linear`, `Activation`) + training.

use cave_mlx::array::Array;
use cave_mlx::autograd::Tape;
use cave_mlx::nn::{Activation, Conv2d, Linear};
use cave_mlx::optim::{Adam, Optimizer};

fn arr(data: &[f32], shape: &[usize]) -> Array {
    Array::new(data.to_vec(), shape).unwrap()
}

#[test]
fn linear_forward_identity_weight() {
    // weight is (in, out); identity -> output == input.
    let lin = Linear::from_parts(arr(&[1.0, 0.0, 0.0, 1.0], &[2, 2]), arr(&[0.0, 0.0], &[2]));
    let t = Tape::new();
    let x = t.var(arr(&[1.0, 2.0], &[1, 2]));
    let f = lin.forward(&t, &x);
    assert_eq!(f.output.value().shape(), &[1, 2]);
    assert_eq!(f.output.value().data(), &[1.0, 2.0]);
}

#[test]
fn linear_forward_adds_bias() {
    let lin = Linear::from_parts(arr(&[1.0, 0.0, 0.0, 1.0], &[2, 2]), arr(&[10.0, 20.0], &[2]));
    let t = Tape::new();
    let x = t.var(arr(&[1.0, 2.0, 3.0, 4.0], &[2, 2]));
    let f = lin.forward(&t, &x);
    assert_eq!(f.output.value().shape(), &[2, 2]);
    assert_eq!(f.output.value().data(), &[11.0, 22.0, 13.0, 24.0]);
}

#[test]
fn linear_exposes_two_param_vars_with_grads() {
    let lin = Linear::from_parts(arr(&[1.0, 0.0, 0.0, 1.0], &[2, 2]), arr(&[0.0, 0.0], &[2]));
    let t = Tape::new();
    let x = t.var(arr(&[1.0, 1.0], &[1, 2]));
    let f = lin.forward(&t, &x);
    let loss = t.sum(&f.output);
    loss.backward();
    assert_eq!(f.params.len(), 2);
    // bias grad = ones(2); weight grad = x^T @ ones = each row [1,1]
    assert_eq!(f.params[1].grad().data(), &[1.0, 1.0]);
    assert_eq!(f.params[0].grad().shape(), &[2, 2]);
}

#[test]
fn parameters_roundtrip() {
    let mut lin = Linear::from_parts(arr(&[1.0, 2.0, 3.0, 4.0], &[2, 2]), arr(&[5.0, 6.0], &[2]));
    let p = lin.parameters();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].data(), &[1.0, 2.0, 3.0, 4.0]);
    lin.set_parameters(&[arr(&[0.0, 0.0, 0.0, 0.0], &[2, 2]), arr(&[1.0, 1.0], &[2])]);
    assert_eq!(lin.parameters()[0].data(), &[0.0, 0.0, 0.0, 0.0]);
    assert_eq!(lin.parameters()[1].data(), &[1.0, 1.0]);
}

#[test]
fn new_initializes_finite_shapes() {
    let lin = Linear::new(4, 3, 42);
    assert_eq!(lin.weight.shape(), &[4, 3]);
    assert_eq!(lin.bias.shape(), &[3]);
    assert!(lin.weight.data().iter().all(|v| v.is_finite()));
    // Two seeds differ; same seed reproduces.
    let a = Linear::new(4, 3, 1);
    let b = Linear::new(4, 3, 1);
    let c = Linear::new(4, 3, 2);
    assert_eq!(a.weight.data(), b.weight.data());
    assert_ne!(a.weight.data(), c.weight.data());
}

#[test]
fn activation_apply_relu() {
    let t = Tape::new();
    let x = t.var(arr(&[-1.0, 2.0, -3.0], &[3]));
    let y = Activation::Relu.apply(&t, &x);
    assert_eq!(y.value().data(), &[0.0, 2.0, 0.0]);
}

#[test]
fn conv2d_layer_forward_adds_bias() {
    // 1 output channel, 2x2 kernel of ones over a 3x3 single-channel image,
    // bias = 100. Output (1,2,2,1) = patch sums + 100.
    let weight = arr(&[1.0, 1.0, 1.0, 1.0], &[1, 2, 2, 1]);
    let bias = arr(&[100.0], &[1]);
    let layer = Conv2d::from_parts(weight, bias, (1, 1), (0, 0));
    let x = arr(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        &[1, 3, 3, 1],
    );
    let y = layer.forward(&x).unwrap();
    assert_eq!(y.shape(), &[1, 2, 2, 1]);
    // patch sums 12,16,24,28 each + 100.
    assert_eq!(y.data(), &[112.0, 116.0, 124.0, 128.0]);
}

#[test]
fn conv2d_layer_per_output_channel_bias() {
    // 2 output channels; weights pick channel sums; distinct bias per channel.
    // input (1,1,1,1)=[5]; weight (2,1,1,1)=[1, 2]; bias=[10, 20].
    let weight = arr(&[1.0, 2.0], &[2, 1, 1, 1]);
    let bias = arr(&[10.0, 20.0], &[2]);
    let layer = Conv2d::from_parts(weight, bias, (1, 1), (0, 0));
    let x = arr(&[5.0], &[1, 1, 1, 1]);
    let y = layer.forward(&x).unwrap();
    assert_eq!(y.shape(), &[1, 1, 1, 2]);
    // ch0: 5*1+10=15 ; ch1: 5*2+20=30
    assert_eq!(y.data(), &[15.0, 30.0]);
}

#[test]
fn conv2d_layer_new_has_correct_shapes() {
    // Conv2d::new(c_in, c_out, kernel, stride, pad, seed).
    let layer = Conv2d::new(3, 8, (3, 3), (1, 1), (1, 1), 42);
    assert_eq!(layer.weight.shape(), &[8, 3, 3, 3]);
    assert_eq!(layer.bias.shape(), &[8]);
    assert!(layer.weight.data().iter().all(|v| v.is_finite()));
    // Same seed reproduces; different seed differs.
    let a = Conv2d::new(1, 4, (2, 2), (1, 1), (0, 0), 1);
    let b = Conv2d::new(1, 4, (2, 2), (1, 1), (0, 0), 1);
    let c = Conv2d::new(1, 4, (2, 2), (1, 1), (0, 0), 2);
    assert_eq!(a.weight.data(), b.weight.data());
    assert_ne!(a.weight.data(), c.weight.data());
}

#[test]
fn train_linear_with_adam_converges() {
    // Target: y = 2*f0 + (-1)*f1 + 0.5
    let x = arr(&[1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0], &[4, 2]);
    let y = arr(&[2.5, -0.5, 1.5, 3.5], &[4, 1]);
    let mut lin = Linear::new(2, 1, 7);
    let mut opt = Adam::new(0.1);

    let mut first = None;
    let mut last = 0.0;
    for _ in 0..400 {
        let t = Tape::new();
        let xv = t.var(x.clone());
        let yv = t.var(y.clone());
        let f = lin.forward(&t, &xv);
        let diff = t.sub(&f.output, &yv);
        let loss = t.mean(&t.mul(&diff, &diff));
        loss.backward();
        last = loss.value().item();
        first.get_or_insert(last);

        let mut params = lin.parameters();
        let grads: Vec<Array> = f.params.iter().map(|p| p.grad()).collect();
        opt.step(&mut params, &grads);
        lin.set_parameters(&params);
    }
    assert!(last < first.unwrap());
    assert!(last < 0.05, "Adam should fit the affine target, loss={last}");
}
