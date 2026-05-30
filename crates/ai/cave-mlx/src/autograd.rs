// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Reverse-mode automatic differentiation — the cave-mlx analog of
//! `mlx.core.grad` / `value_and_grad`.
//!
//! A [`Tape`] is an arena of nodes. Each forward op appends a node whose value
//! is computed eagerly and which records, per parent, a closure mapping the
//! upstream gradient to that parent's gradient contribution. Because a node's
//! parents are always created before it, the insertion order is already a
//! reverse-topological order: [`Var::backward`] seeds the root with ones and
//! walks node ids from high to low, accumulating gradients.
//!
//! Broadcasting is handled symmetrically: forward ops broadcast via
//! [`crate::ops`], and the backward pass reduces (`unbroadcast`) each gradient
//! back to its parent's shape.

use std::cell::RefCell;

use crate::array::Array;
use crate::ops;

/// A gradient closure: given the gradient flowing into a node's output, return
/// the gradient contribution to one specific parent.
type GradFn = Box<dyn Fn(&Array) -> Array>;

struct Node {
    value: Array,
    grad: Array,
    /// `(parent_id, d_output/d_parent applied to upstream grad)`.
    deps: Vec<(usize, GradFn)>,
}

/// An autodiff tape. Construct with [`Tape::new`], create leaves with
/// [`Tape::var`], compose with the op methods, then call [`Var::backward`].
pub struct Tape {
    nodes: RefCell<Vec<Node>>,
}

/// A handle to a value on a [`Tape`].
#[derive(Clone, Copy)]
pub struct Var<'t> {
    tape: &'t Tape,
    id: usize,
}

impl Default for Tape {
    fn default() -> Self {
        Self::new()
    }
}

impl Tape {
    /// Create an empty tape.
    pub fn new() -> Self {
        Self { nodes: RefCell::new(Vec::new()) }
    }

    fn push(&self, value: Array, deps: Vec<(usize, GradFn)>) -> usize {
        let grad = Array::zeros(value.shape());
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node { value, grad, deps });
        nodes.len() - 1
    }

    /// Wrap a concrete array as a differentiable leaf.
    pub fn var(&self, value: Array) -> Var<'_> {
        let id = self.push(value, Vec::new());
        Var { tape: self, id }
    }

    fn value_of(&self, id: usize) -> Array {
        self.nodes.borrow()[id].value.clone()
    }

    /// Elementwise addition (broadcasting).
    pub fn add(&self, x: &Var, y: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let yv = self.value_of(y.id);
        let out = ops::add(&xv, &yv).expect("add: incompatible shapes");
        let xs = xv.shape().to_vec();
        let ys = yv.shape().to_vec();
        let deps: Vec<(usize, GradFn)> = vec![
            (x.id, Box::new(move |g: &Array| unbroadcast(g, &xs))),
            (y.id, Box::new(move |g: &Array| unbroadcast(g, &ys))),
        ];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// Elementwise subtraction (broadcasting).
    pub fn sub(&self, x: &Var, y: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let yv = self.value_of(y.id);
        let out = ops::sub(&xv, &yv).expect("sub: incompatible shapes");
        let xs = xv.shape().to_vec();
        let ys = yv.shape().to_vec();
        let deps: Vec<(usize, GradFn)> = vec![
            (x.id, Box::new(move |g: &Array| unbroadcast(g, &xs))),
            (y.id, Box::new(move |g: &Array| unbroadcast(&ops::neg(g), &ys))),
        ];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// Elementwise multiplication (broadcasting).
    pub fn mul(&self, x: &Var, y: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let yv = self.value_of(y.id);
        let out = ops::mul(&xv, &yv).expect("mul: incompatible shapes");
        let xs = xv.shape().to_vec();
        let ys = yv.shape().to_vec();
        let yv_c = yv.clone();
        let xv_c = xv.clone();
        let deps: Vec<(usize, GradFn)> = vec![
            (x.id, Box::new(move |g: &Array| unbroadcast(&ops::mul(g, &yv_c).unwrap(), &xs))),
            (y.id, Box::new(move |g: &Array| unbroadcast(&ops::mul(g, &xv_c).unwrap(), &ys))),
        ];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// 2-D matrix multiply.
    pub fn matmul(&self, x: &Var, y: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let yv = self.value_of(y.id);
        let out = ops::matmul(&xv, &yv).expect("matmul: incompatible shapes");
        let a_for_b = xv.clone();
        let b_for_a = yv.clone();
        let deps: Vec<(usize, GradFn)> = vec![
            // grad_a = g @ b^T
            (x.id, Box::new(move |g: &Array| ops::matmul(g, &ops::transpose(&b_for_a)).unwrap())),
            // grad_b = a^T @ g
            (y.id, Box::new(move |g: &Array| ops::matmul(&ops::transpose(&a_for_b), g).unwrap())),
        ];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// ReLU activation.
    pub fn relu(&self, x: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let out = ops::relu(&xv);
        let mask = ops::map(&xv, |v| if v > 0.0 { 1.0 } else { 0.0 });
        let deps: Vec<(usize, GradFn)> =
            vec![(x.id, Box::new(move |g: &Array| ops::mul(g, &mask).unwrap()))];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// Logistic sigmoid activation.
    pub fn sigmoid(&self, x: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let out = ops::sigmoid(&xv);
        // s * (1 - s)
        let s = out.clone();
        let deriv = ops::mul(&s, &ops::scalar_add(&ops::neg(&s), 1.0)).unwrap();
        let deps: Vec<(usize, GradFn)> =
            vec![(x.id, Box::new(move |g: &Array| ops::mul(g, &deriv).unwrap()))];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// Hyperbolic tangent activation.
    pub fn tanh(&self, x: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let out = ops::tanh(&xv);
        // 1 - tanh^2
        let t2 = ops::mul(&out, &out).unwrap();
        let deriv = ops::scalar_add(&ops::neg(&t2), 1.0);
        let deps: Vec<(usize, GradFn)> =
            vec![(x.id, Box::new(move |g: &Array| ops::mul(g, &deriv).unwrap()))];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// Sum-reduce to a scalar.
    pub fn sum(&self, x: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let out = ops::sum(&xv, None);
        let shape = xv.shape().to_vec();
        let deps: Vec<(usize, GradFn)> = vec![(
            x.id,
            Box::new(move |g: &Array| Array::full(&shape, g.item())),
        )];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }

    /// Mean-reduce to a scalar.
    pub fn mean(&self, x: &Var) -> Var<'_> {
        let xv = self.value_of(x.id);
        let out = ops::mean(&xv, None);
        let shape = xv.shape().to_vec();
        let n = xv.size().max(1) as f32;
        let deps: Vec<(usize, GradFn)> = vec![(
            x.id,
            Box::new(move |g: &Array| Array::full(&shape, g.item() / n)),
        )];
        let id = self.push(out, deps);
        Var { tape: self, id }
    }
}

impl<'t> Var<'t> {
    /// The forward value of this node.
    pub fn value(&self) -> Array {
        self.tape.value_of(self.id)
    }

    /// The accumulated gradient (zeros until [`backward`](Self::backward)).
    pub fn grad(&self) -> Array {
        self.tape.nodes.borrow()[self.id].grad.clone()
    }

    /// Run reverse-mode accumulation from this node, seeding it with ones.
    ///
    /// Zeroes all gradients first so a tape can be re-differentiated.
    pub fn backward(&self) {
        let mut nodes = self.tape.nodes.borrow_mut();
        for node in nodes.iter_mut() {
            node.grad = Array::zeros(node.value.shape());
        }
        nodes[self.id].grad = Array::ones(nodes[self.id].value.shape());
        for id in (0..nodes.len()).rev() {
            // Take the current grad and dep list out to avoid overlapping
            // mutable borrows while we write into parents.
            let g = nodes[id].grad.clone();
            let deps = std::mem::take(&mut nodes[id].deps);
            for (parent, gfn) in deps.iter() {
                let contrib = gfn(&g);
                let acc = ops::add(&nodes[*parent].grad, &contrib)
                    .expect("gradient accumulation shape mismatch");
                nodes[*parent].grad = acc;
            }
            nodes[id].deps = deps;
        }
    }
}

/// Reduce `grad` (a broadcasted gradient) back down to `target_shape` by
/// summing over the broadcast (size-1 or missing-leading) axes.
fn unbroadcast(grad: &Array, target_shape: &[usize]) -> Array {
    if grad.shape() == target_shape {
        return grad.clone();
    }
    let mut g = grad.clone();
    // 1) Sum away extra leading dimensions until ranks match.
    while g.ndim() > target_shape.len() {
        g = ops::sum(&g, Some(0));
    }
    // 2) Sum (keep-dim) over axes where the target dim is 1 but grad's is > 1.
    for (axis, &tdim) in target_shape.iter().enumerate() {
        if tdim == 1 && g.shape()[axis] != 1 {
            let reduced = ops::sum(&g, Some(axis));
            // ops::sum drops the axis; reinsert as size 1 to preserve rank.
            let mut new_shape = reduced.shape().to_vec();
            new_shape.insert(axis, 1);
            g = reduced.reshape(&new_shape).unwrap();
        }
    }
    if g.shape() != target_shape {
        // Final safety reshape for scalar/degenerate cases.
        g = g.reshape(target_shape).unwrap_or(g);
    }
    g
}
