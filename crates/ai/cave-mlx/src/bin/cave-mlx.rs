// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-mlx` CLI — thin front-end over the cave-mlx array library.
//!
//! The subcommands exercise the real library so the binary doubles as a smoke
//! test for the array/autograd/nn/optim stack on the host.

use clap::{Parser, Subcommand};

use cave_mlx::array::Array;
use cave_mlx::autograd::Tape;
use cave_mlx::conv;
use cave_mlx::nn::Linear;
use cave_mlx::optim::{Adam, Optimizer};

const MLX_PARITY_VERSION: &str = "v0.31.2";

#[derive(Parser)]
#[command(
    name = "cave-mlx",
    about = "Cave MLX — pure-Rust array/autograd/nn toolkit (ml-explore/mlx parity)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the cave-mlx version and the pinned upstream MLX parity tag.
    Version,
    /// List the implemented capability surface.
    Info,
    /// Train a tiny affine model with autograd + Adam and print the loss curve.
    Demo {
        /// Number of optimization steps.
        #[arg(long, default_value_t = 100)]
        steps: usize,
        /// Learning rate.
        #[arg(long, default_value_t = 0.1)]
        lr: f32,
    },
    /// Run a small channel-last conv2d + 2x2 max-pool over a sample image.
    Conv,
}

fn main() {
    match Cli::parse().command {
        Command::Version => {
            println!(
                "cave-mlx {} (ml-explore/mlx {MLX_PARITY_VERSION} parity, CPU backend)",
                env!("CARGO_PKG_VERSION")
            );
        }
        Command::Info => {
            println!("cave-mlx — pure-Rust subset of ml-explore/mlx {MLX_PARITY_VERSION}");
            println!("  array   : N-dim f32 Array (shape/strides/reshape/arange/get)");
            println!("  ops     : add/sub/mul/div (broadcast), matmul, transpose,");
            println!("            sum/mean/max, exp/log/sqrt, relu/sigmoid/tanh, softmax");
            println!("  autograd: reverse-mode Tape/Var (add/sub/mul/matmul/relu/sigmoid/tanh/sum/mean)");
            println!("  conv    : conv1d/conv2d (channel-last), max_pool2d/avg_pool2d");
            println!("  nn      : Linear, Conv2d, Activation");
            println!("  optim   : Sgd (momentum/decay), Adam, AdamW");
        }
        Command::Demo { steps, lr } => run_demo(steps, lr),
        Command::Conv => run_conv(),
    }
}

/// Convolve a 4x4 single-channel image with a 2x2 box kernel, then 2x2 max-pool.
fn run_conv() {
    // NHWC input (1,4,4,1) = 1..=16.
    let data: Vec<f32> = (1..=16).map(|v| v as f32).collect();
    let x = Array::new(data, &[1, 4, 4, 1]).unwrap();
    // Box filter (C_out=1, KH=2, KW=2, C_in=1).
    let w = Array::new(vec![1.0; 4], &[1, 2, 2, 1]).unwrap();
    let y = conv::conv2d(&x, &w, (1, 1), (0, 0)).unwrap();
    println!("conv2d 2x2-box over 4x4 image -> shape {:?}", y.shape());
    print_hw(y.data(), 3, 3);
    let p = conv::max_pool2d(&x, (2, 2), (2, 2));
    println!("max_pool2d 2x2 stride 2     -> shape {:?}", p.shape());
    print_hw(p.data(), 2, 2);
}

/// Print a flat single-channel buffer as an HxW grid.
fn print_hw(data: &[f32], h: usize, w: usize) {
    for r in 0..h {
        let row: Vec<String> = (0..w).map(|c| format!("{:>5.1}", data[r * w + c])).collect();
        println!("  [{}]", row.join(" "));
    }
}

/// Fit `y = 3*f0 - 2*f1 + 1` from four samples and report the loss curve.
fn run_demo(steps: usize, lr: f32) {
    let x = Array::new(
        vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0],
        &[4, 2],
    )
    .unwrap();
    let y = Array::new(vec![4.0, -1.0, 2.0, 5.0], &[4, 1]).unwrap();
    let mut lin = Linear::new(2, 1, 0);
    let mut opt = Adam::new(lr);

    println!("step  loss");
    for step in 0..steps {
        let t = Tape::new();
        let xv = t.var(x.clone());
        let yv = t.var(y.clone());
        let f = lin.forward(&t, &xv);
        let diff = t.sub(&f.output, &yv);
        let loss = t.mean(&t.mul(&diff, &diff));
        loss.backward();
        if step % (steps.max(10) / 10).max(1) == 0 || step == steps - 1 {
            println!("{step:>4}  {:.6}", loss.value().item());
        }
        let mut params = lin.parameters();
        let grads: Vec<Array> = f.params.iter().map(|p| p.grad()).collect();
        opt.step(&mut params, &grads);
        lin.set_parameters(&params);
    }
    let w = lin.weight.data();
    println!(
        "learned: w0={:.3} w1={:.3} b={:.3}  (target 3.000 -2.000 1.000)",
        w[0],
        w[1],
        lin.bias.data()[0]
    );
}
