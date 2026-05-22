// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-llm-tracker` CLI — invoked by the daily LaunchAgent at 03:00
//! Europe/Berlin and exposed under `cavectl llm-tracker report` for
//! interactive use.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use tracing::info;

use cave_llm_tracker::bench::{run_bench, synth_snapshot};
use cave_llm_tracker::config::TrackerConfig;
use cave_llm_tracker::error::TrackerResult;
use cave_llm_tracker::poll::{poll_all, PollSummary};
use cave_llm_tracker::report::{shortlist, DailyReport};
use cave_llm_tracker::selection::{baseline_verdict, evaluate};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    /// Default — poll, optional bench (if Ollama reachable), emit report.
    Report,
    /// Poll only; print summary to stdout, do not bench, do not write.
    Poll,
    /// Bench only against the baseline; print snapshot JSON.
    Bench,
    /// Selection only against the precomputed seed candidates; useful
    /// to validate verdicts logic without touching the network.
    Select,
}

#[derive(Debug, Parser)]
#[command(
    name = "cave-llm-tracker",
    about = "Daily always-latest tracker for the local-LLM seat (Phase 0: report only)",
    version
)]
struct Cli {
    #[arg(long, value_enum, default_value_t = Mode::Report)]
    mode: Mode,
    /// Path to the JSON report file. Sibling `.md` is written next to it.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Optional TOML config override. If omitted, [`TrackerConfig::default_config`].
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override the baseline model id without writing a config file.
    #[arg(long)]
    baseline: Option<String>,
    /// Cap on candidates included in the live bench (each prompt costs
    /// real Ollama time). Default 8.
    #[arg(long, default_value_t = 8)]
    bench_cap: usize,
}

#[tokio::main]
async fn main() -> TrackerResult<()> {
    tracing_subscriber_init();
    let cli = Cli::parse();
    let mut cfg = match &cli.config {
        Some(p) => TrackerConfig::load(p)?,
        None => TrackerConfig::default_config(),
    };
    if let Some(b) = cli.baseline.clone() {
        cfg.baseline.model = b;
    }
    cfg.validate()?;
    match cli.mode {
        Mode::Report => mode_report(cli, cfg).await,
        Mode::Poll => mode_poll(cfg).await,
        Mode::Bench => mode_bench(cfg).await,
        Mode::Select => mode_select(cfg).await,
    }
}

async fn mode_report(cli: Cli, cfg: TrackerConfig) -> TrackerResult<()> {
    info!(baseline = %cfg.baseline.model, "starting daily report");
    let poll = poll_all(&cfg).await.unwrap_or_else(|e| {
        info!(error = %e, "poll failed; falling back to seed-only summary");
        PollSummary::from_seed_only()
    });
    let baseline_bench = match run_bench(&cfg, &cfg.baseline.model).await {
        Ok(s) => s,
        Err(_) => synth_snapshot(&cfg.baseline.model),
    };
    let mut candidate_benches = Vec::new();
    let mut verdicts = vec![baseline_verdict(&cfg.baseline.model)];
    for cand in shortlist(&poll, &cfg.baseline.model, cli.bench_cap) {
        let snap = match run_bench(&cfg, &cand.model_id).await {
            Ok(s) => s,
            Err(_) => synth_snapshot(&cand.model_id),
        };
        let verdict = evaluate(&cfg, cand, &snap, &baseline_bench);
        verdicts.push(verdict);
        candidate_benches.push(snap);
    }
    let report = DailyReport::assemble(&cfg, poll, baseline_bench, candidate_benches, verdicts);
    write_report(&cli, &report, &cfg)?;
    Ok(())
}

fn write_report(cli: &Cli, report: &DailyReport, cfg: &TrackerConfig) -> TrackerResult<()> {
    let json = report.to_json()?;
    let md = report.to_markdown();
    if let Some(out) = &cli.output {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out, &json)?;
        let md_path = out.with_extension("md");
        std::fs::write(&md_path, &md)?;
        println!("{}", out.display());
        println!("{}", md_path.display());
        return Ok(());
    }
    let dir = std::path::PathBuf::from(cfg.expanded_output_dir());
    let stamp = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let (jp, mp) = report.write_to_dir(&dir, &stamp)?;
    if cfg.report.emit_latest {
        let latest = dir.join("latest.json");
        std::fs::write(&latest, &json)?;
    }
    println!("{}", jp.display());
    println!("{}", mp.display());
    Ok(())
}

async fn mode_poll(cfg: TrackerConfig) -> TrackerResult<()> {
    let p = poll_all(&cfg).await.unwrap_or_else(|_| PollSummary::from_seed_only());
    println!("{}", serde_json::to_string_pretty(&p)?);
    Ok(())
}

async fn mode_bench(cfg: TrackerConfig) -> TrackerResult<()> {
    let s = run_bench(&cfg, &cfg.baseline.model).await?;
    println!("{}", serde_json::to_string_pretty(&s)?);
    Ok(())
}

async fn mode_select(cfg: TrackerConfig) -> TrackerResult<()> {
    let poll = PollSummary::from_seed_only();
    let base = synth_snapshot(&cfg.baseline.model);
    let mut verdicts = vec![baseline_verdict(&cfg.baseline.model)];
    for cand in shortlist(&poll, &cfg.baseline.model, 32) {
        let snap = synth_snapshot(&cand.model_id);
        verdicts.push(evaluate(&cfg, cand, &snap, &base));
    }
    println!("{}", serde_json::to_string_pretty(&verdicts)?);
    Ok(())
}

fn tracing_subscriber_init() {
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber_stub::default_subscriber(),
    );
}

/// Tiny shim — we don't want `tracing-subscriber` as a workspace dep
/// just for one binary. Re-uses `tracing`'s default no-op subscriber.
mod tracing_subscriber_stub {
    pub fn default_subscriber() -> tracing::subscriber::NoSubscriber {
        tracing::subscriber::NoSubscriber::default()
    }
}
