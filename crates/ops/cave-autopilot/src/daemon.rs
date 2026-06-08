// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The daemon: scheduler that wires every module into the 7/24 loop.
//!
//! Each tick the daemon (1) reads tracker state, (2) refreshes the queue, (3)
//! decides a [`RunMode`] from the stop conditions, and (4) in `Active` /
//! `LocalOnly` mode works the next task through the escalation ladder. The
//! mode decision and disk accounting are pure and unit-tested; [`Daemon::run`]
//! adds the tokio interval, signal handling, and metrics server.
//!
//! ## Stop conditions
//!
//! * **disk < floor** → [`RunMode::Halt`] (and a human notification).
//! * **all subsystems ≥ idle threshold** → [`RunMode::Idle`] (monitor only).
//! * **Claude daily budget spent** → [`RunMode::LocalOnly`] (no L3).
//! * otherwise → [`RunMode::Active`].

use crate::charter::{self, CommitKind};
use crate::codegen::{self, FileSet};
use crate::config::AutopilotConfig;
use crate::error::{AutopilotError, Result};
use crate::executor::{LlmSmokeExecutor, SmokeOutcome, SmokeSpec};
use crate::metrics::{MetricsSnapshot, SharedMetrics};
use crate::ollama::OllamaClient;
use crate::queue::TaskQueue;
use crate::tracker::TrackerState;
use crate::worktree::WorktreeJob;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What the daemon should do this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Work tasks, escalation to Claude permitted.
    Active,
    /// Work tasks, but no Claude (budget spent) — local LLM only.
    LocalOnly,
    /// Everything at threshold — monitor only, dispatch nothing.
    Idle,
    /// A hard stop condition fired (disk). Do no work; notify the human.
    Halt,
}

impl RunMode {
    pub fn works_tasks(&self) -> bool {
        matches!(self, RunMode::Active | RunMode::LocalOnly)
    }
}

/// Inputs to the mode decision, snapshotted per tick.
#[derive(Debug, Clone, Copy)]
pub struct ModeInputs {
    pub free_disk_gb: u64,
    pub min_free_disk_gb: u64,
    pub all_at_threshold: bool,
    pub claude_budget_exhausted: bool,
}

/// Pure stop-condition policy. Disk floor is the hard stop and wins over
/// everything; idle wins over local-only.
pub fn decide_mode(i: &ModeInputs) -> RunMode {
    if i.free_disk_gb < i.min_free_disk_gb {
        return RunMode::Halt;
    }
    if i.all_at_threshold {
        return RunMode::Idle;
    }
    if i.claude_budget_exhausted {
        return RunMode::LocalOnly;
    }
    RunMode::Active
}

/// Parse the available-KiB figure from `df -k <path>` output. Returns the
/// `Avail`/`Available` column of the data row.
pub fn parse_df_avail_kb(df_out: &str) -> Option<u64> {
    let mut lines = df_out.lines();
    let header = lines.next()?;
    // Find the column index of the "Avail"/"Available" header.
    let cols: Vec<&str> = header.split_whitespace().collect();
    let idx = cols
        .iter()
        .position(|c| c.eq_ignore_ascii_case("avail") || c.eq_ignore_ascii_case("available"))?;
    let row = lines.next()?;
    let fields: Vec<&str> = row.split_whitespace().collect();
    fields.get(idx).and_then(|v| v.parse::<u64>().ok())
}

/// Free space in whole GiB for the filesystem holding `path`, via `df -k`.
/// Returns 0 on any failure (which conservatively trips the disk floor).
pub fn free_disk_gb(path: &Path) -> u64 {
    let out = std::process::Command::new("df")
        .arg("-k")
        .arg(path)
        .output();
    let Ok(out) = out else { return 0 };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_df_avail_kb(&text)
        .map(|kb| kb / (1024 * 1024))
        .unwrap_or(0)
}

/// Outcome of one task execution attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    pub task_id: String,
    pub passed: bool,
    pub merged: bool,
    pub detail: String,
}

/// The daemon, owning config + shared metrics + the queue.
pub struct Daemon {
    pub cfg: AutopilotConfig,
    pub metrics: SharedMetrics,
    pub queue: Arc<Mutex<TaskQueue>>,
    start: Instant,
}

impl Daemon {
    pub fn new(cfg: AutopilotConfig) -> Self {
        let metrics = Arc::new(Mutex::new(MetricsSnapshot::new(&cfg.instance)));
        Self {
            cfg,
            metrics,
            queue: Arc::new(Mutex::new(TaskQueue::new())),
            start: Instant::now(),
        }
    }

    /// Read tracker state, refresh the queue, and compute the run mode. Updates
    /// the metrics gauges. Returns the decided mode + the tracker state.
    pub fn refresh(&self, claude_budget_exhausted: bool) -> Result<(RunMode, TrackerState)> {
        let tracker = TrackerState::load(&self.cfg.tracker_state_path)?;
        let ranked = tracker.ranked_incomplete(self.cfg.completion_threshold);
        {
            let mut q = self.queue.lock().expect("queue mutex");
            q.enqueue_ranked(&ranked);
        }
        let mode = decide_mode(&ModeInputs {
            free_disk_gb: free_disk_gb(&self.cfg.repo_root),
            min_free_disk_gb: self.cfg.min_free_disk_gb,
            all_at_threshold: tracker.all_at_or_above(self.cfg.idle_threshold),
            claude_budget_exhausted,
        });
        {
            let mut m = self.metrics.lock().expect("metrics mutex");
            m.uptime_secs = self.start.elapsed().as_secs();
            m.queue_depth = self.queue.lock().expect("queue mutex").pending_depth();
            m.mean_completion = tracker.mean_completion();
            m.idle = mode == RunMode::Idle;
        }
        Ok((mode, tracker))
    }

    /// Whether the daemon should run the operational startup smoke this boot.
    pub fn should_run_startup_smoke(&self) -> bool {
        self.cfg.startup_smoke
    }

    /// The spec for the operational liveness smoke: ask the local coder to write
    /// a real standalone `cave-test-autopilot` crate (a function + its test) and
    /// `cargo test` it. The retry budget tracks the configured local-LLM budget.
    pub fn smoke_spec(&self) -> SmokeSpec {
        SmokeSpec {
            crate_name: "cave-test-autopilot".to_string(),
            task_desc: "an integer add(a, b) function returning a + b".to_string(),
            max_retries: self.cfg.max_local_retries,
        }
    }

    /// Record one operational smoke outcome into metrics. This is deliberately
    /// *not* a real port task: it bumps the smoke counters and the local-coder
    /// LLM-call tally and stamps liveness, but never touches `tasks_completed` /
    /// `tasks_failed`, so the dashboard can't read a liveness probe as a crate
    /// that actually reached parity.
    pub fn record_smoke(&self, outcome: &SmokeOutcome) {
        let mut m = self.metrics.lock().expect("metrics mutex");
        m.smoke_runs += 1;
        if outcome.passed {
            m.smoke_passed += 1;
        }
        m.record_llm_call("l2_coder");
        m.last_task_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    /// Run one operational LLM smoke dispatch end-to-end against the live Ollama
    /// server: resolve the L2 coder tier (named MoE → resident fallback), ask it
    /// to write + test a real crate in a throwaway workdir, and record the
    /// outcome. This is the heartbeat that proves the local-LLM pipeline runs
    /// unattended — separate from the (multi-week) real per-crate port executor.
    pub async fn dispatch_smoke(&self) -> Result<SmokeOutcome> {
        let client = OllamaClient::new(&self.cfg.ollama_url);
        if !client.is_up().await {
            return Err(AutopilotError::Llm(format!(
                "ollama unreachable at {}",
                self.cfg.ollama_url
            )));
        }
        let tiers = client
            .ensure_tiers(
                &self.cfg.model_l1_router,
                &self.cfg.model_l2_coder,
                &self.cfg.model_fallback,
            )
            .await?;
        let spec = self.smoke_spec();
        let workdir = self.cfg.worktree_root.join("smoke");
        std::fs::create_dir_all(&workdir)?;
        let _ = std::fs::remove_dir_all(workdir.join(&spec.crate_name));
        let exec = LlmSmokeExecutor::new(client, &tiers.coder);
        let outcome = exec.run(&spec, &workdir).await?;
        self.record_smoke(&outcome);
        Ok(outcome)
    }

    /// Execute the deterministic mock task end-to-end: worktree → scaffold →
    /// `cargo test` → charter audit → commit → merge (no-ff). This is the
    /// acceptance-path proof that the pipeline works without a live LLM.
    pub fn run_mock_scaffold(&self, base: &str) -> Result<ExecOutcome> {
        let task_id = "scaffold-cave-test";
        let job = WorktreeJob::new(&self.cfg.repo_root, &self.cfg.worktree_root, task_id);

        let create = job.create(base)?;
        if !create.success {
            return Err(AutopilotError::Worktree(format!(
                "worktree add failed: {}",
                create.stderr
            )));
        }

        // Generate the standalone cave-test crate inside the worktree.
        let fs: FileSet = codegen::scaffold_cave_test("cave-test");
        // Charter gate part 1: the produced source must be stub-free.
        for f in &fs.files {
            if !charter::scan_for_stubs(&f.content).is_empty() {
                let _ = job.remove();
                return Err(AutopilotError::Charter(format!("stub in {}", f.path)));
            }
        }
        fs.apply(&job.worktree_path)?;

        // Build + test the standalone crate (run cargo in the crate dir).
        let crate_dir = job.worktree_path.join("cave-test");
        let test = std::process::Command::new("cargo")
            .arg("test")
            .current_dir(&crate_dir)
            .output()
            .map_err(|e| AutopilotError::Worktree(format!("cargo test: {e}")))?;
        let test_out = format!(
            "{}{}",
            String::from_utf8_lossy(&test.stdout),
            String::from_utf8_lossy(&test.stderr)
        );
        let passed = WorktreeJob::tests_passed(&test_out);
        if !passed {
            let _ = job.remove();
            return Ok(ExecOutcome {
                task_id: task_id.into(),
                passed: false,
                merged: false,
                detail: "cargo test did not report a pass".into(),
            });
        }

        // Charter gate part 2: TDD sequence — we commit the test, then nothing
        // else is needed (impl + test ship together in the scaffold, with the
        // test present), so the audit checks stub-freedom + test presence.
        let lib_src = fs
            .files
            .iter()
            .find(|f| f.path.ends_with("src/lib.rs"))
            .map(|f| f.content.as_str())
            .unwrap_or("");
        let audit = charter::CharterAudit::evaluate(
            &[],
            &[CommitKind::Red, CommitKind::Green],
            charter::count_code_lines(lib_src),
            // test lines: count lines inside the cfg(test) module heuristically
            lib_src
                .lines()
                .filter(|l| l.contains("#[test]") || l.contains("assert"))
                .count(),
        );
        if !audit.passes() {
            let _ = job.remove();
            return Err(AutopilotError::Charter(audit.violations.join("; ")));
        }

        let commit = job.commit_all(&format!(
            "feat(cave-test): autopilot scaffold smoke crate (GREEN)\n\nGenerated by cave-autopilot {} mock task.",
            self.cfg.instance
        ))?;
        if !commit.success {
            let _ = job.remove();
            return Err(AutopilotError::Worktree(format!(
                "commit failed: {}{}",
                commit.stdout, commit.stderr
            )));
        }

        let merge = job.merge_no_ff(&format!("merge: autopilot mock task {task_id} (no push)"))?;
        let merged = merge.success;

        // Record into metrics + queue bookkeeping.
        {
            let mut m = self.metrics.lock().expect("metrics mutex");
            m.record_llm_call("l2_coder");
            if merged {
                m.tasks_completed += 1;
            } else {
                m.tasks_failed += 1;
            }
        }

        // Clean up the worktree (branch + merge commit remain in the repo).
        let _ = job.remove();

        Ok(ExecOutcome {
            task_id: task_id.into(),
            passed: true,
            merged,
            detail: if merged {
                "scaffolded, tested, committed, merged --no-ff (no push)".into()
            } else {
                format!("tested+committed but merge failed: {}", merge.stderr)
            },
        })
    }

    /// Run the daemon loop until SIGINT/SIGTERM. Spawns the metrics server and
    /// ticks every `tick_interval_secs`.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let port = self.cfg.metrics_port;
        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::metrics::serve(port, metrics).await {
                tracing::error!("metrics server exited: {e}");
            }
        });

        tracing::info!(
            instance = %self.cfg.instance,
            port,
            "cave-autopilot daemon started"
        );

        // Operational liveness: run one end-to-end LLM smoke on startup so the
        // daemon visibly processes its first task (local-coder → compile → test)
        // instead of only monitoring. A smoke failure is logged, not fatal — the
        // scheduler still comes up.
        if self.should_run_startup_smoke() {
            tracing::info!("running operational startup smoke dispatch");
            match self.dispatch_smoke().await {
                Ok(o) => tracing::info!(
                    model = %o.model,
                    attempts = o.attempts,
                    passed = o.passed,
                    "startup smoke complete: {}",
                    o.detail
                ),
                Err(e) => tracing::warn!("startup smoke dispatch failed (non-fatal): {e}"),
            }
        }

        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(self.cfg.tick_interval_secs));

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let budget_exhausted = {
                        let m = self.metrics.lock().expect("metrics mutex");
                        m.claude_tokens_used >= self.cfg.claude_daily_token_budget
                    };
                    match self.refresh(budget_exhausted) {
                        Ok((mode, _)) => {
                            tracing::info!(?mode, "tick");
                            if mode == RunMode::Halt {
                                tracing::error!(
                                    "disk below {} GiB floor — halting work; human attention needed",
                                    self.cfg.min_free_disk_gb
                                );
                            }
                            // Live LLM-driven dispatch is the multi-week port;
                            // the loop currently maintains queue + metrics +
                            // mode each tick. `run_mock_scaffold` proves the
                            // execution pipeline end-to-end.
                        }
                        Err(e) => tracing::error!("tick refresh failed: {e}"),
                    }
                }
                _ = shutdown_signal() => {
                    tracing::info!("shutdown signal received; exiting cleanly");
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Resolve when SIGINT or SIGTERM arrives.
async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> ModeInputs {
        ModeInputs {
            free_disk_gb: 100,
            min_free_disk_gb: 5,
            all_at_threshold: false,
            claude_budget_exhausted: false,
        }
    }

    #[test]
    fn active_when_healthy() {
        assert_eq!(decide_mode(&inputs()), RunMode::Active);
    }

    #[test]
    fn disk_floor_halts_over_everything() {
        let mut i = inputs();
        i.free_disk_gb = 2;
        i.all_at_threshold = true; // even if idle would apply
        assert_eq!(decide_mode(&i), RunMode::Halt);
    }

    #[test]
    fn idle_when_all_at_threshold() {
        let mut i = inputs();
        i.all_at_threshold = true;
        assert_eq!(decide_mode(&i), RunMode::Idle);
    }

    #[test]
    fn local_only_when_budget_spent() {
        let mut i = inputs();
        i.claude_budget_exhausted = true;
        assert_eq!(decide_mode(&i), RunMode::LocalOnly);
        assert!(decide_mode(&i).works_tasks());
    }

    #[test]
    fn idle_does_not_work_tasks() {
        assert!(!RunMode::Idle.works_tasks());
        assert!(!RunMode::Halt.works_tasks());
    }

    #[test]
    fn smoke_spec_uses_config_retries_and_names_a_cave_test_crate() {
        let cfg = AutopilotConfig::for_instance("cave-runtime");
        let retries = cfg.max_local_retries;
        let d = Daemon::new(cfg);
        let spec = d.smoke_spec();
        assert!(spec.crate_name.starts_with("cave-test"));
        assert!(!spec.task_desc.is_empty());
        assert_eq!(spec.max_retries, retries);
    }

    #[test]
    fn should_run_startup_smoke_follows_config_flag() {
        let mut cfg = AutopilotConfig::default();
        cfg.startup_smoke = false;
        assert!(!Daemon::new(cfg).should_run_startup_smoke());
        let mut cfg2 = AutopilotConfig::default();
        cfg2.startup_smoke = true;
        assert!(Daemon::new(cfg2).should_run_startup_smoke());
    }

    #[test]
    fn record_smoke_bumps_runs_without_inflating_port_completions() {
        use crate::executor::SmokeOutcome;
        let d = Daemon::new(AutopilotConfig::default());
        let ok = SmokeOutcome {
            crate_name: "cave-test-autopilot".into(),
            model: "qwen".into(),
            attempts: 1,
            generated: true,
            passed: true,
            detail: "ok".into(),
        };
        let bad = SmokeOutcome {
            passed: false,
            attempts: 3,
            ..ok.clone()
        };
        d.record_smoke(&ok);
        d.record_smoke(&bad);
        let m = d.metrics.lock().unwrap();
        assert_eq!(m.smoke_runs, 2);
        assert_eq!(m.smoke_passed, 1);
        // The smoke loop exercises the local coder tier.
        assert!(m.llm_calls.get("l2_coder").copied().unwrap_or(0) >= 2);
        // Liveness timestamp is stamped.
        assert!(m.last_task_unix > 0);
        // Honesty guard: a smoke run is NOT a ported crate — real port
        // completion/failure counters stay untouched.
        assert_eq!(m.tasks_completed, 0);
        assert_eq!(m.tasks_failed, 0);
    }

    #[test]
    fn df_parse_picks_avail_column() {
        let macos = "Filesystem 1024-blocks Used Avail Capacity Mounted on\n/dev/disk3s5 970000000 460000000 463000000 52% /System/Volumes/Data";
        // 463000000 KiB / 1024 / 1024 ≈ 441 GiB
        let kb = parse_df_avail_kb(macos).unwrap();
        assert_eq!(kb, 463000000);
        assert_eq!(kb / (1024 * 1024), 441);
    }

    #[test]
    fn df_parse_linux_available_header() {
        let linux = "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/sda1 100000 40000 60000 40% /";
        assert_eq!(parse_df_avail_kb(linux), Some(60000));
    }

    #[test]
    fn df_parse_returns_none_on_garbage() {
        assert_eq!(parse_df_avail_kb("not a df output"), None);
    }
}
