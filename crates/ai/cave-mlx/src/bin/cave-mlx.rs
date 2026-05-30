// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-mlx` CLI — thin front-end over the cave-mlx array library.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cave-mlx",
    about = "Cave MLX — pure-Rust array/autograd/nn toolkit",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the pinned upstream ml-explore/mlx parity version.
    Version,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Version => {
            println!("cave-mlx {} (ml-explore/mlx v0.31.2 parity)", env!("CARGO_PKG_VERSION"));
        }
    }
}
