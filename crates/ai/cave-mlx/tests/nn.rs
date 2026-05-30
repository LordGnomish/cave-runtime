// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: neural-network modules (`Linear`, `Activation`) + training.

use cave_mlx::array::Array;
use cave_mlx::autograd::Tape;
use cave_mlx::nn::{Activation, Linear};
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
