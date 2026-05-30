// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Neural-network building blocks — the cave-mlx analog of `mlx.nn`.
//!
//! A [`Linear`] layer owns its weight/bias as plain [`Array`]s. [`Linear::forward`]
//! registers those parameters as leaves on a [`Tape`] and returns the output
//! together with the parameter [`Var`]s, so a training loop can read their
//! gradients after [`Var::backward`], hand them to an optimizer, and write the
//! updated values back with [`Linear::set_parameters`].

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::array::Array;
use crate::autograd::{Tape, Var};

/// The result of a module forward pass on a tape: the output node plus the
/// parameter leaves (in `parameters()` order) for gradient collection.
pub struct Forward<'t> {
    /// Output activation node.
    pub output: Var<'t>,
    /// Parameter leaves: `[weight, bias]` for [`Linear`].
    pub params: Vec<Var<'t>>,
}

/// A fully-connected affine layer: `y = x @ W + b`.
///
/// Weight shape is `(in_features, out_features)`; bias shape is
/// `(out_features,)` and broadcasts over the batch dimension.
pub struct Linear {
    /// Weight matrix, shape `(in_features, out_features)`.
    pub weight: Array,
    /// Bias vector, shape `(out_features,)`.
    pub bias: Array,
}

impl Linear {
    /// Construct from explicit weight/bias arrays.
    pub fn from_parts(weight: Array, bias: Array) -> Self {
        Self { weight, bias }
    }

    /// Construct with Kaiming-uniform weights and zero bias from a fixed seed
    /// (deterministic for a given seed — reproducible training).
    pub fn new(in_features: usize, out_features: usize, seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        // Kaiming-uniform bound: sqrt(6 / fan_in).
        let bound = (6.0f32 / in_features as f32).sqrt();
        let weight: Vec<f32> = (0..in_features * out_features)
            .map(|_| rng.gen_range(-bound..bound))
            .collect();
        Self {
            weight: Array::from_parts(weight, vec![in_features, out_features]),
            bias: Array::zeros(&[out_features]),
        }
    }

    /// Run the affine transform on a tape, returning the output and the
    /// weight/bias parameter vars.
    pub fn forward<'t>(&self, tape: &'t Tape, x: &Var<'t>) -> Forward<'t> {
        let w = tape.var(self.weight.clone());
        let b = tape.var(self.bias.clone());
        let xw = tape.matmul(x, &w);
        let output = tape.add(&xw, &b);
        Forward { output, params: vec![w, b] }
    }

    /// Current parameter values in `[weight, bias]` order.
    pub fn parameters(&self) -> Vec<Array> {
        vec![self.weight.clone(), self.bias.clone()]
    }

    /// Write back updated `[weight, bias]` values (as produced by an optimizer).
    pub fn set_parameters(&mut self, params: &[Array]) {
        assert_eq!(params.len(), 2, "Linear expects [weight, bias]");
        self.weight = params[0].clone();
        self.bias = params[1].clone();
    }
}

/// Parameter-free activation functions usable as standalone modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// `max(0, x)`.
    Relu,
    /// Logistic sigmoid.
    Sigmoid,
    /// Hyperbolic tangent.
    Tanh,
    /// Pass-through.
    Identity,
}

impl Activation {
    /// Apply the activation on a tape.
    pub fn apply<'t>(&self, tape: &'t Tape, x: &Var<'t>) -> Var<'t> {
        match self {
            Activation::Relu => tape.relu(x),
            Activation::Sigmoid => tape.sigmoid(x),
            Activation::Tanh => tape.tanh(x),
            Activation::Identity => *x,
        }
    }
}
