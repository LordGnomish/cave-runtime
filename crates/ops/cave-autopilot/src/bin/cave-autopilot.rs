// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-autopilot` — single-binary entry point for the autonomous code-gen
//! orchestrator. Sub-command driven (single-binary mandate; no per-crate Helm).

use cave_autopilot::config::AutopilotConfig;
use cave_autopilot::daemon::Daemon;
use cave_autopilot::{launchagent, report};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "cave-autopilot",
    about = "7/24 autonomous code-gen orchestrator (tiered local LLM + Claude escalation)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the 7/24 daemon (this is what the LaunchAgent invokes).
    Daemon {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Generate + load the LaunchAgent plist for an instance.
    Install {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
        /// Path to the installed binary (default: this executable).
        #[arg(long)]
        binary: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Unload + remove the LaunchAgent plist for an instance.
    Uninstall {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
    },
    /// One refresh tick: read tracker, build queue, print mode + top of queue.
    Once {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Run the deterministic mock task end-to-end (scaffold cave-test crate).
    Mock {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
        #[arg(long, default_value = "HEAD")]
        base: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Print the current health/metrics snapshot as text.
    Status {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Print the default config TOML for an instance.
    InitConfig {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
    },
    /// Print the Ollama model setup script.
    SetupScript {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
    },
    /// Write a daily report (uses live queue/metrics; mostly empty until the
    /// daemon has run a full day).
    Report {
        #[arg(long, default_value = "cave-runtime")]
        instance: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn load_cfg(instance: &str, config: &Option<PathBuf>) -> AutopilotConfig {
    match config {
        Some(p) if p.exists() => AutopilotConfig::load(p).unwrap_or_else(|e| {
            eprintln!("config load failed ({e}); falling back to defaults");
            AutopilotConfig::for_instance(instance)
        }),
        _ => AutopilotConfig::for_instance(instance),
    }
}

fn default_config_path(instance: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config/cave-autopilot")
        .join(format!("{instance}.toml"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Daemon { instance, config } => {
            let cfg = load_cfg(&instance, &config);
            cfg.validate()?;
            let daemon = Arc::new(Daemon::new(cfg));
            daemon.run().await?;
        }
        Command::Install {
            instance,
            binary,
            config,
        } => {
            let cfg = load_cfg(&instance, &config);
            // Persist a config file so the daemon has a stable path.
            let cfg_path = config.unwrap_or_else(|| default_config_path(&instance));
            if let Some(parent) = cfg_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&cfg_path, cfg.to_toml()?)?;
            let bin = binary
                .or_else(|| std::env::current_exe().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "cave-autopilot".into());
            let spec = launchagent::PlistSpec::from_config(
                &cfg,
                &bin,
                &cfg_path.to_string_lossy(),
            );
            let path = launchagent::install(&spec)?;
            println!("installed + loaded LaunchAgent: {}", path.display());
            println!("config: {}", cfg_path.display());
            println!("verify:  launchctl list | grep cave-autopilot");
        }
        Command::Uninstall { instance } => {
            let cfg = AutopilotConfig::for_instance(&instance);
            launchagent::uninstall(&cfg.launch_label())?;
            println!("uninstalled {}", cfg.launch_label());
        }
        Command::Once { instance, config } => {
            let cfg = load_cfg(&instance, &config);
            let daemon = Daemon::new(cfg);
            let budget_exhausted = false;
            let (mode, tracker) = daemon.refresh(budget_exhausted)?;
            let q = daemon.queue.lock().unwrap();
            println!("instance: {}", daemon.cfg.instance);
            println!("mode:     {mode:?}");
            println!("tracked:  {} crates", tracker.len());
            println!("queue:    {} pending", q.pending_depth());
            let mut top: Vec<_> = q.all().collect();
            top.sort_by_key(|t| t.priority);
            for t in top.into_iter().take(10) {
                println!("  [{}] {} ({:.3})", t.priority, t.subsystem, t.completion);
            }
        }
        Command::Mock {
            instance,
            base,
            config,
        } => {
            let cfg = load_cfg(&instance, &config);
            let daemon = Daemon::new(cfg);
            let outcome = daemon.run_mock_scaffold(&base)?;
            println!("mock task: {}", outcome.task_id);
            println!("passed:    {}", outcome.passed);
            println!("merged:    {}", outcome.merged);
            println!("detail:    {}", outcome.detail);
            if !outcome.merged {
                std::process::exit(1);
            }
        }
        Command::Status { instance, config } => {
            let cfg = load_cfg(&instance, &config);
            let daemon = Daemon::new(cfg);
            let m = daemon.metrics.lock().unwrap();
            println!("{}", m.render_health());
            println!("---");
            println!("{}", m.render_prometheus());
        }
        Command::InitConfig { instance } => {
            println!("{}", AutopilotConfig::for_instance(&instance).to_toml()?);
        }
        Command::SetupScript { instance } => {
            let cfg = AutopilotConfig::for_instance(&instance);
            println!("{}", launchagent::ollama_setup_script(&cfg));
        }
        Command::Report { instance, config } => {
            let cfg = load_cfg(&instance, &config);
            let daemon = Daemon::new(cfg);
            let (_mode, tracker) = daemon
                .refresh(false)
                .unwrap_or((cave_autopilot::daemon::RunMode::Idle, Default::default()));
            let q = daemon.queue.lock().unwrap();
            let mut ordered: Vec<_> = q.all().collect();
            ordered.sort_by_key(|t| t.priority);
            let next: Vec<String> = ordered.into_iter().take(10).map(|t| t.id.clone()).collect();
            let rpt = report::DailyReport {
                date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                instance: daemon.cfg.instance.clone(),
                completed: vec![],
                failed: vec![],
                escalated_human: vec![],
                claude_calls: 0,
                claude_tokens: 0,
                llm_calls: Default::default(),
                mean_completion_start: tracker.mean_completion(),
                mean_completion_end: tracker.mean_completion(),
                next_queue: next,
            };
            let path = rpt.write(&daemon.cfg.report_dir)?;
            println!("wrote report: {}", path.display());
        }
    }
    Ok(())
}
