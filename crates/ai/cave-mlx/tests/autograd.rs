// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: reverse-mode automatic differentiation over a recorded tape.

use cave_mlx::array::Array;
use cave_mlx::autograd::Tape;

fn arr(data: &[f32], shape: &[usize]) -> Array {
    Array::new(data.to_vec(), shape).unwrap()
}

#[test]
fn mul_then_sum_grads_are_the_other_operand() {
    let t = Tape::new();
    let x = t.var(arr(&[1.0, 2.0, 3.0], &[3]));
    let y = t.var(arr(&[4.0, 5.0, 6.0], &[3]));
    let z = t.sum(&t.mul(&x, &y));
    assert_eq!(z.value().item(), 32.0);
    z.backward();
    assert_eq!(x.grad().data(), &[4.0, 5.0, 6.0]);
    assert_eq!(y.grad().data(), &[1.0, 2.0, 3.0]);
}

#[test]
fn square_via_reused_leaf_accumulates_grad() {
    // z = sum(x * x) ; dz/dx = 2x
    let t = Tape::new();
    let x = t.var(arr(&[1.0, 2.0, 3.0], &[3]));
    let z = t.sum(&t.mul(&x, &x));
    assert_eq!(z.value().item(), 14.0);
    z.backward();
    assert_eq!(x.grad().data(), &[2.0, 4.0, 6.0]);
}

#[test]
fn broadcast_add_reduces_bias_gradient() {
    // z = sum(x + b), x:(2,3), b:(3,) ; dz/dx = ones, dz/db = [2,2,2]
    let t = Tape::new();
    let x = t.var(arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2, 3]));
    let b = t.var(arr(&[10.0, 20.0, 30.0], &[3]));
    let z = t.sum(&t.add(&x, &b));
    z.backward();
    assert_eq!(x.grad().data(), &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(b.grad().shape(), &[3]);
    assert_eq!(b.grad().data(), &[2.0, 2.0, 2.0]);
}

#[test]
fn matmul_backward_shapes_and_values() {
    // a:(2,2), w = I(2,2); loss = sum(a @ w)
    let t = Tape::new();
    let a = t.var(arr(&[1.0, 2.0, 3.0, 4.0], &[2, 2]));
    let w = t.var(arr(&[1.0, 0.0, 0.0, 1.0], &[2, 2]));
    let z = t.sum(&t.matmul(&a, &w));
    assert_eq!(z.value().item(), 10.0);
    z.backward();
    // grad_a = ones(2,2) @ w^T = ones
    assert_eq!(a.grad().data(), &[1.0, 1.0, 1.0, 1.0]);
    // grad_w = a^T @ ones(2,2) = [[1+3,1+3],[2+4,2+4]]
    assert_eq!(w.grad().data(), &[4.0, 4.0, 6.0, 6.0]);
}

#[test]
fn relu_backward_gates_negatives() {
    let t = Tape::new();
    let x = t.var(arr(&[-1.0, 2.0, -3.0, 4.0], &[4]));
    let z = t.sum(&t.relu(&x));
    assert_eq!(z.value().item(), 6.0);
    z.backward();
    assert_eq!(x.grad().data(), &[0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn sigmoid_backward_matches_analytic() {
    // z = sum(sigmoid(x)); grad = s*(1-s). At x=0 -> 0.25.
    let t = Tape::new();
    let x = t.var(arr(&[0.0], &[1]));
    let z = t.sum(&t.sigmoid(&x));
    z.backward();
    assert!((x.grad().data()[0] - 0.25).abs() < 1e-6);
}

#[test]
fn sub_and_mean_compose() {
    // z = mean(x - y); dz/dx = 1/n, dz/dy = -1/n
    let t = Tape::new();
    let x = t.var(arr(&[2.0, 4.0, 6.0, 8.0], &[4]));
    let y = t.var(arr(&[1.0, 1.0, 1.0, 1.0], &[4]));
    let z = t.mean(&t.sub(&x, &y));
    z.backward();
    for &g in x.grad().data() {
        assert!((g - 0.25).abs() < 1e-6);
    }
    for &g in y.grad().data() {
        assert!((g + 0.25).abs() < 1e-6);
    }
}

#[test]
fn linear_regression_gradient_descends() {
    // One step of GD on MSE should reduce the loss for a tiny linear model.
    let t = Tape::new();
    let w = t.var(arr(&[0.0, 0.0], &[2, 1]));
    let xb = t.var(arr(&[1.0, 2.0, 3.0, 4.0], &[2, 2])); // 2 samples, 2 feats
    let target = t.var(arr(&[1.0, 2.0], &[2, 1]));
    let pred = t.matmul(&xb, &w);
    let diff = t.sub(&pred, &target);
    let loss = t.mean(&t.mul(&diff, &diff));
    let loss0 = loss.value().item();
    loss.backward();
    let g = w.grad();
    assert_eq!(g.shape(), &[2, 1]);
    // Gradient is non-zero so a descent step is possible.
    assert!(g.data().iter().any(|&v| v.abs() > 1e-6));
    assert!(loss0 > 0.0);
}
