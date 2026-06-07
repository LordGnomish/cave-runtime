// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-runtime-tracker` CLI — invoked by the daily LaunchAgent at
//! 06:30 Europe/Berlin (30 min after the cave-home tracker) and exposed
//! under `cavectl runtime-tracker report` for interactive use.
//!
//! Subcommands:
//! - `poll` — poll upstreams, print the summary JSON (no files). Honours
//!   `GITHUB_TOKEN`.
//! - `report [--measure]` — poll (+ optional LOC measure), write
//!   `daily-<date>.{md,json}`, `daily-progress-<date>.md`, `latest.json`
//!   (and `latest-measure.json` with `--measure`).
//! - `measure [--repo ...]` — shallow-clone + tokei LOC for the headline
//!   upstreams; print + cache `latest-measure.json`.
//! - `metrics [--output FILE]` — render the cached report as Prometheus
//!   text to stdout (or a node_exporter textfile).
//! - `serve [--port 9103]` — long-running `/metrics` daemon.
//! - `install-agent [--write] [--port 9103]` — render (or install) the
//!   LaunchAgent plists.
//! - `config` — print the default YAML config.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use cave_runtime_tracker::config::TrackerConfig;
use cave_runtime_tracker::error::{TrackerError, TrackerResult};
use cave_runtime_tracker::launchd::{daily_report_agent, metrics_serve_agent};
use cave_runtime_tracker::measure::{measure_subset, Measurement, TokeiLoc, DEFAULT_MEASURE_REPOS};
use cave_runtime_tracker::metrics::render_prometheus;
use cave_runtime_tracker::poll::{poll_all, PollSummary};
use cave_runtime_tracker::registry::GithubFetcher;
use cave_runtime_tracker::report::DailyReport;
use cave_runtime_tracker::serve;

const DEFAULT_METRICS_PORT: u16 = 9103;

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
    /// cave-runtime workspace root (holds `crates/`). Defaults to
    /// `$CAVE_RUNTIME_ROOT`, then a short list of known checkouts.
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Poll upstreams and print the summary JSON to stdout.
    Poll,
    /// Poll, assemble, and write the daily md+JSON + progress report.
    Report {
        /// Override the output directory (defaults to config output_dir).
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Also shallow-clone + tokei the headline upstreams for a LOC
        /// port-depth table and `latest-measure.json`.
        #[arg(long)]
        measure: bool,
    },
    /// Shallow-clone + tokei the headline upstreams; cache the result.
    Measure {
        /// Repos to measure (org/name). Repeatable. Defaults to the
        /// headline set.
        #[arg(long = "repo")]
        repos: Vec<String>,
        /// Override the output directory for `latest-measure.json`.
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
    /// Render the cached report as Prometheus text exposition.
    Metrics {
        /// Override the output directory the cache is read from.
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Write to this file (node_exporter textfile) instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Serve `/metrics` (+ `/healthz`) until signalled.
    Serve {
        #[arg(long, default_value_t = DEFAULT_METRICS_PORT)]
        port: u16,
    },
    /// Render (or, with --write, install) the LaunchAgent plists.
    InstallAgent {
        /// Write to ~/Library/LaunchAgents instead of printing to stdout.
        #[arg(long)]
        write: bool,
        #[arg(long, default_value_t = DEFAULT_METRICS_PORT)]
        port: u16,
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
        Cmd::Report { output_dir, measure } => {
            let summary = run_poll(&cfg).await;
            let report = DailyReport::assemble(summary);
            let dir = output_dir.unwrap_or_else(|| PathBuf::from(cfg.expanded_output_dir()));
            let stamp = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let (jp, mp) = report.write_to_dir(&dir, &stamp, cfg.emit_latest)?;
            println!("{}", jp.display());
            println!("{}", mp.display());

            let measurements = if measure {
                let ms = run_measure(&cfg, cli.workspace.as_deref(), DEFAULT_MEASURE_REPOS);
                cache_measurements(&dir, &ms)?;
                ms
            } else {
                Vec::new()
            };
            // The richer human digest, always written.
            let prog = dir.join(format!("daily-progress-{stamp}.md"));
            std::fs::write(&prog, report.to_progress_markdown(&measurements))?;
            println!("{}", prog.display());
            Ok(())
        }
        Cmd::Measure { repos, output_dir } => {
            let want: Vec<&str> = if repos.is_empty() {
                DEFAULT_MEASURE_REPOS.to_vec()
            } else {
                repos.iter().map(|s| s.as_str()).collect()
            };
            let ms = run_measure(&cfg, cli.workspace.as_deref(), &want);
            let dir = output_dir.unwrap_or_else(|| PathBuf::from(cfg.expanded_output_dir()));
            cache_measurements(&dir, &ms)?;
            println!("{}", serde_json::to_string_pretty(&ms)?);
            Ok(())
        }
        Cmd::Metrics { output_dir, output } => {
            let dir = output_dir.unwrap_or_else(|| PathBuf::from(cfg.expanded_output_dir()));
            let (report, measurements) = serve::load_latest(&cfg, &dir);
            let text = render_prometheus(&report, &measurements);
            match output {
                Some(path) => {
                    std::fs::write(&path, &text)?;
                    println!("{}", path.display());
                }
                None => print!("{text}"),
            }
            Ok(())
        }
        Cmd::Serve { port } => serve::serve(cfg, port).await,
        Cmd::InstallAgent { write, port } => install_agents(write, port),
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

/// Resolve the workspace root: explicit flag → `$CAVE_RUNTIME_ROOT` → the
/// first known checkout that actually has a `crates/` dir.
fn resolve_workspace(explicit: Option<&std::path::Path>) -> Option<PathBuf> {
    let has_crates = |p: &std::path::Path| p.join("crates").is_dir();
    if let Some(p) = explicit {
        return has_crates(p).then(|| p.to_path_buf());
    }
    if let Ok(env) = std::env::var("CAVE_RUNTIME_ROOT") {
        let p = PathBuf::from(env);
        if has_crates(&p) {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        std::env::current_dir().unwrap_or_default(),
        PathBuf::from(format!("{home}/Code/cave-runtime-main")),
        PathBuf::from(format!("{home}/Code/cave-runtime")),
    ];
    candidates.into_iter().find(|p| has_crates(p))
}

/// Run the LOC measure over `repos`, locating the workspace (cave side
/// degrades to `None` LOC if no checkout is found) and cloning into the
/// system temp dir.
fn run_measure(
    cfg: &TrackerConfig,
    workspace: Option<&std::path::Path>,
    repos: &[&str],
) -> Vec<Measurement> {
    let ws = resolve_workspace(workspace).unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let source = TokeiLoc::new(ws, std::env::temp_dir());
    measure_subset(&cfg.upstreams, &source, repos)
}

/// Persist measurements as `latest-measure.json` (read back by `metrics`
/// / `serve` and `report --measure`'s progress digest).
fn cache_measurements(dir: &std::path::Path, ms: &[Measurement]) -> TrackerResult<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join("latest-measure.json"),
        serde_json::to_string_pretty(ms)?,
    )?;
    Ok(())
}

/// Render both LaunchAgent plists; with `write`, install them into
/// `~/Library/LaunchAgents` and print the paths.
fn install_agents(write: bool, port: u16) -> TrackerResult<()> {
    let home = std::env::var("HOME").map_err(|_| TrackerError::Config("HOME unset".into()))?;
    let bin = format!("{home}/.local/bin/cave-runtime-tracker");
    let support = format!("{home}/Library/Application Support/cave-runtime");
    let agents = [
        daily_report_agent(&home, &bin, &support),
        metrics_serve_agent(&home, &bin, &support, port),
    ];
    for spec in agents {
        let xml = spec.render();
        if write {
            let dest = PathBuf::from(format!("{home}/Library/LaunchAgents/{}.plist", spec.label));
            std::fs::write(&dest, &xml)?;
            println!("{}", dest.display());
        } else {
            println!("{xml}");
        }
    }
    Ok(())
}
