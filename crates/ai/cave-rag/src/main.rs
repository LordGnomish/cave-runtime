// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-rag` binary entry point — a thin shell over [`cave_rag::cli`].

use clap::Parser;

fn main() {
    let cli = cave_rag::cli::Cli::parse();
    match cave_rag::cli::run(cli) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
