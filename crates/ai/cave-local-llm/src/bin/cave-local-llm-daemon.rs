// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-local-llm-daemon — 24/7 scheduler daemon for tier-1 draft generation.
//!
//! Usage:
//!   cave-local-llm-daemon start  [--workspace-root <path>]
//!   cave-local-llm-daemon stop   [--workspace-root <path>]
//!   cave-local-llm-daemon status [--workspace-root <path>]

use anyhow::{Context, Result};
use cave_local_llm::{
    daemon::{Daemon, DaemonConfig},
    metrics::DaemonMetrics,
};
use clap::{Parser, Subcommand};
use prometheus_client::registry::Registry;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cave-local-llm-daemon",
    about = "Cave Local LLM 24/7 scheduler daemon (Qwen / Ollama)",
    version
)]
struct Cli {
    /// Workspace root (parent of Cargo.toml)
    #[arg(long, default_value = ".", env = "CAVE_WORKSPACE_ROOT")]
    workspace_root: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon in the foreground
    Start {
        /// Tick interval in seconds (default 5)
        #[arg(long, default_value = "5")]
        tick_secs: u64,
        /// Cargo test timeout in seconds (default 600)
        #[arg(long, default_value = "600")]
        test_timeout_secs: u64,
    },
    /// Signal a running daemon to stop gracefully
    Stop,
    /// Show whether the daemon stop-signal file exists
    Status,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("cave_local_llm=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let workspace_root = cli.workspace_root.canonicalize().with_context(|| {
        format!(
            "workspace root '{}' not found",
            cli.workspace_root.display()
        )
    })?;

    let mut cfg = DaemonConfig::new(&workspace_root);

    match cli.command {
        Commands::Start {
            tick_secs,
            test_timeout_secs,
        } => {
            cfg.tick_interval = std::time::Duration::from_secs(tick_secs);
            cfg.cargo_test_timeout = std::time::Duration::from_secs(test_timeout_secs);

            let mut registry = Registry::default();
            let metrics = DaemonMetrics::new(&mut registry);
            let daemon = Daemon::new(cfg, metrics);
            daemon.run().await?;
        }
        Commands::Stop => {
            std::fs::write(&cfg.stop_signal_path, b"stop").with_context(|| {
                format!("writing stop signal to {}", cfg.stop_signal_path.display())
            })?;
            println!("stop signal written → {}", cfg.stop_signal_path.display());
        }
        Commands::Status => {
            if cfg.stop_signal_path.exists() {
                println!("status: stop-signal file present — daemon will stop at next tick");
                println!("  signal file: {}", cfg.stop_signal_path.display());
            } else {
                println!("status: no stop-signal file — daemon should be running (or not started)");
            }
        }
    }

    Ok(())
}
