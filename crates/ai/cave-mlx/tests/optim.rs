// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict-TDD: SGD / Adam / AdamW optimizers + an end-to-end training loop.

use cave_mlx::array::Array;
use cave_mlx::autograd::Tape;
use cave_mlx::optim::{Adam, Optimizer, Sgd};

fn arr(data: &[f32], shape: &[usize]) -> Array {
    Array::new(data.to_vec(), shape).unwrap()
}

#[test]
fn sgd_vanilla_step() {
    let mut opt = Sgd::new(0.1);
    let mut params = vec![arr(&[1.0, 1.0], &[2])];
    let grads = vec![arr(&[1.0, 0.5], &[2])];
    opt.step(&mut params, &grads);
    assert_eq!(params[0].data(), &[0.9, 0.95]);
}

#[test]
fn sgd_momentum_accumulates() {
    let mut opt = Sgd::new(0.1).with_momentum(0.9);
    let mut params = vec![arr(&[1.0], &[1])];
    let grad = vec![arr(&[1.0], &[1])];
    // step 1: v=1.0, param -= 0.1*1.0 = 0.9
    opt.step(&mut params, &grad);
    assert!((params[0].item() - 0.9).abs() < 1e-6);
    // step 2: v = 0.9*1.0 + 1.0 = 1.9, param -= 0.1*1.9 => 0.9 - 0.19 = 0.71
    opt.step(&mut params, &grad);
    assert!((params[0].item() - 0.71).abs() < 1e-6);
}

#[test]
fn sgd_weight_decay_pulls_toward_zero() {
    // With grad=0 and weight_decay, the parameter shrinks: p -= lr*wd*p.
    let mut opt = Sgd::new(0.1).with_weight_decay(0.5);
    let mut params = vec![arr(&[2.0], &[1])];
    let grad = vec![arr(&[0.0], &[1])];
    opt.step(&mut params, &grad);
    // p -= 0.1 * (0 + 0.5*2.0) = 2.0 - 0.1 = 1.9
    assert!((params[0].item() - 1.9).abs() < 1e-6);
}

#[test]
fn adam_first_step_is_signed_lr() {
    // At t=1 with bias correction, the update magnitude ~ lr*sign(g).
    let mut opt = Adam::new(0.1);
    let mut params = vec![arr(&[1.0, 1.0], &[2])];
    let grads = vec![arr(&[2.0, -3.0], &[2])];
    opt.step(&mut params, &grads);
    // First element: 1.0 - ~0.1, second: 1.0 + ~0.1
    assert!((params[0].data()[0] - 0.9).abs() < 1e-3);
    assert!((params[0].data()[1] - 1.1).abs() < 1e-3);
}

#[test]
fn adamw_applies_decoupled_decay() {
    // AdamW: with grad=0, only decoupled decay applies: p -= lr*wd*p.
    let mut opt = Adam::new(0.1).adamw().with_weight_decay(0.1);
    let mut params = vec![arr(&[2.0], &[1])];
    let grad = vec![arr(&[0.0], &[1])];
    opt.step(&mut params, &grad);
    // m=v=0 -> adam update 0; decoupled decay: 2.0 - 0.1*0.1*2.0 = 1.98
    assert!((params[0].item() - 1.98).abs() < 1e-6);
}

#[test]
fn training_loop_reduces_mse_loss() {
    // Fit w in y = X w with SGD over the cave-mlx autograd tape.
    let x = arr(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[3, 2]); // 3 samples, 2 feats
    let y = arr(&[3.0, 7.0, 11.0], &[3, 1]); // y = 1*f0 + 1*f1 ... ish
    let mut w = arr(&[0.0, 0.0], &[2, 1]);
    let mut opt = Sgd::new(0.01);

    let mut first_loss = None;
    let mut last_loss = 0.0;
    for _ in 0..200 {
        let t = Tape::new();
        let wv = t.var(w.clone());
        let xv = t.var(x.clone());
        let yv = t.var(y.clone());
        let pred = t.matmul(&xv, &wv);
        let diff = t.sub(&pred, &yv);
        let loss = t.mean(&t.mul(&diff, &diff));
        loss.backward();
        last_loss = loss.value().item();
        if first_loss.is_none() {
            first_loss = Some(last_loss);
        }
        let mut params = vec![w.clone()];
        opt.step(&mut params, &[wv.grad()]);
        w = params.into_iter().next().unwrap();
    }
    assert!(last_loss < first_loss.unwrap());
    assert!(last_loss < 0.5, "loss should converge near zero, got {last_loss}");
}
