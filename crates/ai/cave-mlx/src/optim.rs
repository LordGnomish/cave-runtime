// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! First-order optimizers — the cave-mlx analog of `mlx.optimizers`.
//!
//! Each optimizer is stateful and updates a slice of parameter [`Array`]s in
//! place given their gradients (typically obtained from
//! [`crate::autograd::Var::grad`]). Per-parameter state (momentum buffers,
//! Adam moments) is allocated lazily on the first [`Optimizer::step`] and
//! indexed positionally, so the parameter slice must keep a stable order.

use crate::array::Array;
use crate::ops;

/// Common optimizer interface: apply one update to `params` using `grads`.
pub trait Optimizer {
    /// Update every parameter in place. `params` and `grads` are parallel.
    fn step(&mut self, params: &mut [Array], grads: &[Array]);
}

/// Stochastic gradient descent with optional momentum and (coupled) L2
/// weight decay.
pub struct Sgd {
    lr: f32,
    momentum: f32,
    weight_decay: f32,
    velocity: Vec<Array>,
}

impl Sgd {
    /// Construct plain SGD with the given learning rate.
    pub fn new(lr: f32) -> Self {
        Self { lr, momentum: 0.0, weight_decay: 0.0, velocity: Vec::new() }
    }

    /// Enable classical momentum (`v <- momentum*v + g`).
    pub fn with_momentum(mut self, m: f32) -> Self {
        self.momentum = m;
        self
    }

    /// Enable coupled L2 weight decay (`g <- g + wd*p`).
    pub fn with_weight_decay(mut self, wd: f32) -> Self {
        self.weight_decay = wd;
        self
    }
}

impl Optimizer for Sgd {
    fn step(&mut self, params: &mut [Array], grads: &[Array]) {
        if self.velocity.len() != params.len() {
            self.velocity = params.iter().map(|p| Array::zeros(p.shape())).collect();
        }
        for (i, (p, g)) in params.iter_mut().zip(grads).enumerate() {
            // coupled weight decay: g_eff = g + wd*p
            let g_eff = if self.weight_decay != 0.0 {
                ops::add(g, &ops::scalar_mul(p, self.weight_decay)).unwrap()
            } else {
                g.clone()
            };
            let update = if self.momentum != 0.0 {
                let v = &self.velocity[i];
                let new_v = ops::add(&ops::scalar_mul(v, self.momentum), &g_eff).unwrap();
                self.velocity[i] = new_v.clone();
                new_v
            } else {
                g_eff
            };
            *p = ops::sub(p, &ops::scalar_mul(&update, self.lr)).unwrap();
        }
    }
}

/// Adam / AdamW optimizer.
///
/// Construct with [`Adam::new`]; call [`Adam::adamw`] to switch to decoupled
/// weight decay (AdamW). Weight decay defaults to zero.
pub struct Adam {
    lr: f32,
    beta1: f32,
    beta2: f32,
    eps: f32,
    weight_decay: f32,
    decoupled: bool,
    t: i32,
    m: Vec<Array>,
    v: Vec<Array>,
}

impl Adam {
    /// Construct Adam with the given learning rate and standard betas/eps.
    pub fn new(lr: f32) -> Self {
        Self {
            lr,
            beta1: 0.9,
            beta2: 0.999,
            eps: 1e-8,
            weight_decay: 0.0,
            decoupled: false,
            t: 0,
            m: Vec::new(),
            v: Vec::new(),
        }
    }

    /// Override the `(beta1, beta2)` exponential-decay rates.
    pub fn with_betas(mut self, b1: f32, b2: f32) -> Self {
        self.beta1 = b1;
        self.beta2 = b2;
        self
    }

    /// Switch to the AdamW variant (decoupled weight decay).
    pub fn adamw(mut self) -> Self {
        self.decoupled = true;
        self
    }

    /// Set the weight-decay coefficient (coupled for Adam, decoupled for AdamW).
    pub fn with_weight_decay(mut self, wd: f32) -> Self {
        self.weight_decay = wd;
        self
    }
}

impl Optimizer for Adam {
    fn step(&mut self, params: &mut [Array], grads: &[Array]) {
        if self.m.len() != params.len() {
            self.m = params.iter().map(|p| Array::zeros(p.shape())).collect();
            self.v = params.iter().map(|p| Array::zeros(p.shape())).collect();
        }
        self.t += 1;
        let bc1 = 1.0 - self.beta1.powi(self.t);
        let bc2 = 1.0 - self.beta2.powi(self.t);
        for (i, (p, g)) in params.iter_mut().zip(grads).enumerate() {
            // Coupled decay folds into the gradient; decoupled is applied to p.
            let g_eff = if self.weight_decay != 0.0 && !self.decoupled {
                ops::add(g, &ops::scalar_mul(p, self.weight_decay)).unwrap()
            } else {
                g.clone()
            };
            // m <- b1*m + (1-b1)*g
            self.m[i] = ops::add(
                &ops::scalar_mul(&self.m[i], self.beta1),
                &ops::scalar_mul(&g_eff, 1.0 - self.beta1),
            )
            .unwrap();
            // v <- b2*v + (1-b2)*g^2
            let g2 = ops::mul(&g_eff, &g_eff).unwrap();
            self.v[i] = ops::add(
                &ops::scalar_mul(&self.v[i], self.beta2),
                &ops::scalar_mul(&g2, 1.0 - self.beta2),
            )
            .unwrap();
            // mhat / vhat, then update = lr * mhat / (sqrt(vhat) + eps)
            let mhat = ops::scalar_mul(&self.m[i], 1.0 / bc1);
            let vhat = ops::scalar_mul(&self.v[i], 1.0 / bc2);
            let denom = ops::scalar_add(&ops::sqrt(&vhat), self.eps);
            let step = ops::scalar_mul(&ops::div(&mhat, &denom).unwrap(), self.lr);
            *p = ops::sub(p, &step).unwrap();
            // Decoupled (AdamW) weight decay applied directly to the parameter.
            if self.weight_decay != 0.0 && self.decoupled {
                *p = ops::sub(p, &ops::scalar_mul(p, self.lr * self.weight_decay)).unwrap();
            }
        }
    }
}
