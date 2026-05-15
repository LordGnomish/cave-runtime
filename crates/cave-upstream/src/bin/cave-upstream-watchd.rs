// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-upstream-watchd` — daemon binary for the upstream watch loop.
//!
//! ```text
//! cave-upstream-watchd                       # run forever, default config path
//! cave-upstream-watchd --config /path.toml   # explicit config path
//! cave-upstream-watchd --once                # do one tick and exit (for cron)
//! cave-upstream-watchd --print-config        # dump effective config and exit
//! ```
//!
//! Default config path is
//! `~/Library/Application Support/cave-runtime/upstream-watch.toml`
//! (or `$CAVE_UPSTREAM_CONFIG` if set).
//!
//! GitHub token is read from `$GITHUB_TOKEN` if not set in the TOML.

use cave_upstream::{
    daemon::{install_signal_handler, Config, Daemon},
    projects::TRACKED_PROJECTS,
};
use clap::Parser;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "cave-upstream-watchd")]
#[command(about = "Watch upstream OSS releases and feed the Qwen pump", long_about = None)]
struct Cli {
    /// Path to the daemon config TOML.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Run a single tick and exit. Use this from cron / GitHub Actions.
    #[arg(long)]
    once: bool,

    /// Print the resolved config and exit.
    #[arg(long)]
    print_config: bool,

    /// Override the GitHub token (also read from $GITHUB_TOKEN).
    #[arg(long)]
    github_token: Option<String>,

    /// Limit which projects we track in this run. Match by `cave_module`
    /// substring (`--module cave-etcd`). Repeatable.
    #[arg(long = "module")]
    only_modules: Vec<String>,
}

fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("CAVE_UPSTREAM_CONFIG") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("cave-runtime").join("upstream-watch.toml")
}

fn load_config(path: &std::path::Path) -> anyhow::Result<Config> {
    if !path.exists() {
        info!(path = %path.display(), "no config file; using defaults");
        return Ok(Config::default());
    }
    let body = std::fs::read_to_string(path)?;
    let cfg: Config = toml::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;
    Ok(cfg)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let cfg_path = cli.config.clone().unwrap_or_else(default_config_path);
    let mut cfg = load_config(&cfg_path)?;

    // CLI overrides
    if let Some(t) = cli.github_token.clone() {
        cfg.github_token = Some(t);
    } else if cfg.github_token.is_none() {
        if let Ok(t) = std::env::var("GITHUB_TOKEN") {
            if !t.is_empty() {
                cfg.github_token = Some(t);
            }
        }
    }

    if cli.print_config {
        // Redact token before printing
        let mut redacted = cfg.clone();
        if redacted.github_token.is_some() {
            redacted.github_token = Some("<redacted>".to_string());
        }
        println!("{}", toml::to_string_pretty(&redacted)?);
        return Ok(());
    }

    if cfg.github_token.is_none() {
        warn!(
            "no GITHUB_TOKEN configured — falling back to anonymous limits \
             (60 req/h, NOT enough for 66 repos × 4 ticks/h)"
        );
    }

    let projects: Vec<_> = TRACKED_PROJECTS
        .iter()
        .filter(|p| {
            cli.only_modules.is_empty()
                || cli.only_modules.iter().any(|m| p.cave_module.contains(m))
        })
        .cloned()
        .collect();

    info!(
        config = %cfg_path.display(),
        projects = projects.len(),
        once = cli.once,
        "starting cave-upstream-watchd"
    );

    let daemon = Daemon::new(cfg, projects);

    if cli.once {
        let report = daemon.tick_once().await?;
        info!(?report, "one-shot tick complete");
        // Print a one-line summary on stdout for cron consumers.
        println!(
            "tick run_id={} considered={} due={} polled={} new={} unchanged={} no_releases={} rate_limited={} errors={} payloads={}",
            report.run_id,
            report.considered,
            report.due,
            report.polled,
            report.new_releases,
            report.unchanged,
            report.no_releases,
            report.rate_limited,
            report.errors,
            report.payloads_written.len(),
        );
        return Ok(());
    }

    let cancel = CancellationToken::new();
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move { install_signal_handler(cancel_for_signal).await });

    daemon.run(cancel).await
}
