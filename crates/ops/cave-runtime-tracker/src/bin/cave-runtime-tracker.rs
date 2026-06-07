// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-runtime-tracker` CLI — invoked by the daily LaunchAgent at
//! 06:30 Europe/Berlin (30 min after the cave-home tracker) and exposed
//! under `cavectl runtime-tracker report` for interactive use.
//!
//! Subcommands:
//! - `poll` — poll upstreams, print the summary JSON to stdout (no files
//!   written). Honours `GITHUB_TOKEN`.
//! - `report` — poll + assemble + write `daily-<date>.{md,json}` into the
//!   configured output dir; prints the paths written.
//! - `config` — print the default YAML config to stdout (seed for
//!   `cave-runtime-tracker.yaml`).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use cave_runtime_tracker::config::TrackerConfig;
use cave_runtime_tracker::error::TrackerResult;
use cave_runtime_tracker::poll::{poll_all, PollSummary};
use cave_runtime_tracker::registry::GithubFetcher;
use cave_runtime_tracker::report::DailyReport;

#[derive(Debug, Parser)]
#[command(
    name = "cave-runtime-tracker",
    about = "Daily upstream drift tracker for the cave-runtime platform (Phase 0: report only)",
    version
)]
struct Cli {
    /// Optional YAML config override. Defaults to the built-in registry.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Poll upstreams and print the summary JSON to stdout.
    Poll,
    /// Poll, assemble, and write the daily md+JSON report.
    Report {
        /// Override the output directory (defaults to config output_dir).
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
    /// Print the default YAML config to stdout.
    Config,
}

#[tokio::main]
async fn main() -> TrackerResult<()> {
    let cli = Cli::parse();
    let cfg = match &cli.config {
        Some(p) => TrackerConfig::load(p)?,
        None => TrackerConfig::default_config(),
    };
    cfg.validate()?;

    match cli.cmd {
        Cmd::Config => {
            print!("{}", TrackerConfig::default_config().to_yaml()?);
            Ok(())
        }
        Cmd::Poll => {
            let summary = run_poll(&cfg).await;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Cmd::Report { output_dir } => {
            let summary = run_poll(&cfg).await;
            let report = DailyReport::assemble(summary);
            let dir = output_dir
                .unwrap_or_else(|| PathBuf::from(cfg.expanded_output_dir()));
            let stamp = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let (jp, mp) = report.write_to_dir(&dir, &stamp, cfg.emit_latest)?;
            println!("{}", jp.display());
            println!("{}", mp.display());
            Ok(())
        }
    }
}

/// Run a live poll, degrading to a registry-only summary if the whole
/// pass somehow fails. (Per-repo transport failures already degrade to
/// `unresolved` inside `poll_all`, so this is the belt-and-braces path.)
async fn run_poll(cfg: &TrackerConfig) -> PollSummary {
    let token = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty());
    let fetcher = GithubFetcher::new(cfg.github_api.clone(), cfg.request_timeout_secs, token);
    poll_all(cfg, &fetcher).await
}
