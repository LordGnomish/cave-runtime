// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! 24/7 daemon loop: pick → draft → patch → test → commit or escalate.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{Datelike, IsoWeek, Utc};
use thiserror::Error;
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::draft::{
    Draft, DraftFrontmatter, DraftStatus, build_ollama_prompt, draft_filename,
    parse_ollama_response,
};
use crate::metrics::DaemonMetrics;
use crate::ollama::{GenerateRequest, OllamaClient};
use crate::queue::{QueueItem, QueueStatus};
use crate::scheduler::{DiskFreeChecker, Scheduler, SecondaryRepo, SystemDiskChecker};

// ── Error types ───────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("scheduler: {0}")]
    Scheduler(#[from] crate::scheduler::SchedulerError),
    #[error("queue: {0}")]
    Queue(#[from] crate::queue::QueueError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("git: {0}")]
    Git(String),
    #[error("ollama unreachable after {budget_secs}s: {last_error}")]
    OllamaUnreachable {
        budget_secs: u64,
        last_error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    TestFail,
    CompileFail,
    Timeout,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::TestFail => f.write_str("test_fail"),
            ErrorKind::CompileFail => f.write_str("compile_fail"),
            ErrorKind::Timeout => f.write_str("timeout"),
        }
    }
}

// ── Secondary repo config ─────────────────────────────────────────────────────

/// Configuration for a secondary repository processed by the daemon.
#[derive(Debug, Clone)]
pub struct SecondaryRepoConfig {
    pub workspace_root: PathBuf,
    pub queue_path: PathBuf,
    pub branch: String,
    pub weight: f32,
}

impl SecondaryRepoConfig {
    /// Create a secondary repo config. `queue_file` is the filename within
    /// `<root>/queue/` (e.g. "pipeline-platform-tasks.yaml" or "queue.yaml").
    pub fn new(workspace_root: impl Into<PathBuf>, queue_file: &str, weight: f32) -> Self {
        let root: PathBuf = workspace_root.into();
        let queue_path = root.join("queue").join(queue_file);
        let branch = default_weekly_branch(Utc::now().iso_week());
        Self {
            workspace_root: root,
            queue_path,
            branch,
            weight,
        }
    }
}

// ── Daemon config ─────────────────────────────────────────────────────────────

pub struct DaemonConfig {
    pub workspace_root: PathBuf,
    /// Path to the stop-signal file; daemon exits when this file appears.
    pub stop_signal_path: PathBuf,
    pub tick_interval: Duration,
    pub cargo_test_timeout: Duration,
    /// Ollama base URL (env: OLLAMA_URL)
    pub ollama_url: String,
    /// Qwen model tag (env: OLLAMA_MODEL)
    pub ollama_model: String,
    /// Weekly branch commits land on (env: CAVE_DAEMON_BRANCH)
    pub target_branch: String,
    /// Optional secondary repos (e.g. pipeline-platform-starter).
    pub secondary_repos: Vec<SecondaryRepoConfig>,
}

impl DaemonConfig {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = workspace_root.into();
        let stop = root.join(".cave-daemon.stop");
        let ollama_url =
            std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let ollama_model =
            std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| crate::ollama::DEFAULT_MODEL.into());
        let target_branch = std::env::var("CAVE_DAEMON_BRANCH")
            .unwrap_or_else(|_| default_weekly_branch(Utc::now().iso_week()));

        // Secondary repos from env variables (CAVE_SECONDARY_REPO_PATH, CAVE_TERTIARY_REPO_PATH).
        let mut secondary_repos = Vec::new();
        if let Ok(sec_path) = std::env::var("CAVE_SECONDARY_REPO_PATH") {
            let weight: f32 = std::env::var("CAVE_SECONDARY_REPO_WEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.2);
            secondary_repos.push(SecondaryRepoConfig::new(
                sec_path,
                "pipeline-platform-tasks.yaml",
                weight,
            ));
        }
        if let Ok(ter_path) = std::env::var("CAVE_TERTIARY_REPO_PATH") {
            let weight: f32 = std::env::var("CAVE_TERTIARY_REPO_WEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.2);
            secondary_repos.push(SecondaryRepoConfig::new(ter_path, "queue.yaml", weight));
        }

        Self {
            workspace_root: root,
            stop_signal_path: stop,
            tick_interval: Duration::from_secs(5),
            cargo_test_timeout: Duration::from_secs(600),
            ollama_url,
            ollama_model,
            target_branch,
            secondary_repos,
        }
    }
}

fn default_weekly_branch(week: IsoWeek) -> String {
    format!("qwen/auto-{}-W{:02}", week.year(), week.week())
}

// ── Daemon ────────────────────────────────────────────────────────────────────

pub struct Daemon<D: DiskFreeChecker = SystemDiskChecker> {
    scheduler: Scheduler<D>,
    config: DaemonConfig,
    metrics: DaemonMetrics,
}

impl Daemon<SystemDiskChecker> {
    pub fn new(config: DaemonConfig, metrics: DaemonMetrics) -> Self {
        let mut sched = Scheduler::new(&config.workspace_root);
        for sec in &config.secondary_repos {
            let queue = crate::queue::QueueStore::open_at(&sec.queue_path);
            sched.add_secondary(SecondaryRepo {
                queue,
                repo_root: sec.workspace_root.clone(),
                default_branch: sec.branch.clone(),
                weight: sec.weight,
            });
        }
        Self {
            scheduler: sched,
            config,
            metrics,
        }
    }
}

impl<D: DiskFreeChecker> Daemon<D> {
    pub fn with_scheduler(
        scheduler: Scheduler<D>,
        config: DaemonConfig,
        metrics: DaemonMetrics,
    ) -> Self {
        Self {
            scheduler,
            config,
            metrics,
        }
    }

    /// Runs the daemon until a SIGTERM signal is received or the stop-signal
    /// file appears.
    pub async fn run(self) -> Result<(), DaemonError> {
        #[cfg(unix)]
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(DaemonError::Io)?;

        // Wait for Ollama to be reachable before we start ticking.
        // If the server is still down after the budget expires, exit so launchd
        // can retry us later (ThrottleInterval kicks us back up).
        self.wait_for_ollama(Duration::from_secs(600)).await?;

        // Ensure we are on the configured weekly branch in the primary repo.
        self.ensure_branch(&self.config.workspace_root, &self.config.target_branch)?;

        // Ensure weekly branch also exists in each secondary repo.
        for sec in self.scheduler.secondaries() {
            self.ensure_branch(&sec.repo_root, &sec.default_branch)?;
        }

        // Crash recovery: any items left in_progress from the previous run get
        // reset to pending so the guard doesn't block indefinitely.
        for store in std::iter::once(self.scheduler.queue())
            .chain(self.scheduler.secondaries().iter().map(|s| &s.queue))
        {
            if let Ok(items) = store.list() {
                for item in items
                    .into_iter()
                    .filter(|i| i.status == crate::queue::QueueStatus::InProgress)
                {
                    let _ = store.update_item(
                        &item.id,
                        crate::queue::QueueStatus::Pending,
                        item.attempts,
                        item.last_error.clone(),
                    );
                }
            }
        }

        info!(
            branch = %self.config.target_branch,
            model  = %self.config.ollama_model,
            "cave-local-llm daemon started"
        );

        loop {
            #[cfg(unix)]
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("SIGTERM received — shutting down");
                    break;
                }
                _ = sleep(self.config.tick_interval) => {}
            }
            #[cfg(not(unix))]
            sleep(self.config.tick_interval).await;

            self.metrics.daemon_ticks_total.inc();

            if self.config.stop_signal_path.exists() {
                info!("stop signal file found — shutting down gracefully");
                break;
            }

            if let Err(e) = self.do_tick().await {
                error!("tick error: {e}");
            }
        }

        info!("cave-local-llm daemon stopped");
        Ok(())
    }

    /// Performs one scheduler tick.
    pub(crate) async fn do_tick(&self) -> Result<(), DaemonError> {
        let item = match self.scheduler.pick_next()? {
            Some(i) => i,
            None => return Ok(()),
        };

        info!(
            crate = %item.crate_name,
            upstream_fn = %item.upstream_fn,
            attempt = item.attempts,
            repo = ?item.repo_path,
            "processing item"
        );

        self.metrics
            .queue_items_by_status
            .set_pending(self.scheduler.queue().count_status(&QueueStatus::Pending)? as i64);
        self.metrics.queue_items_by_status.set_in_progress(
            self.scheduler
                .queue()
                .count_status(&QueueStatus::InProgress)? as i64,
        );

        let start = std::time::Instant::now();
        let outcome = self.process_item(&item).await;
        let elapsed = start.elapsed().as_secs_f64();
        self.metrics.daemon_sleep_duration_seconds.observe(elapsed);

        match outcome {
            Ok(branch) => {
                info!(
                    crate = %item.crate_name,
                    branch = %branch,
                    elapsed_s = elapsed,
                    "tier1 commit successful"
                );
                self.scheduler.mark_done(&item)?;
                self.metrics.tier1_commits_total.inc_crate(&item.crate_name);
            }
            Err((kind, msg)) => {
                warn!(
                    crate = %item.crate_name,
                    error_kind = %kind,
                    msg = %msg,
                    attempts = item.attempts,
                    "draft failed — escalating"
                );
                self.scheduler
                    .mark_failed(&item, format!("{kind}: {msg}"))?;
                self.metrics
                    .tier2_escalations_total
                    .inc_crate_kind(&item.crate_name, &kind.to_string());
            }
        }

        Ok(())
    }

    /// Generates draft via Ollama, writes it, runs tests, commits on target branch.
    /// Returns `Ok(branch_name)` on success or `Err((kind, message))` on failure.
    async fn process_item(&self, item: &QueueItem) -> Result<String, (ErrorKind, String)> {
        // Effective repo root and branch for this task.
        let repo_root = item
            .repo_path
            .as_deref()
            .unwrap_or(&self.config.workspace_root);
        let branch = item.branch.as_deref().unwrap_or(&self.config.target_branch);

        let local_fn = item.upstream_fn.to_lowercase();

        // Build prompt and call Qwen.
        let prompt = build_ollama_prompt(
            &item.crate_name,
            &local_fn,
            &item.upstream_repo,
            &item.upstream_fn,
            &item.upstream_file,
        );

        let ollama = OllamaClient::new(&self.config.ollama_url);
        let req = GenerateRequest {
            model: self.config.ollama_model.clone(),
            prompt,
            stream: Some(false),
            options: Some(serde_json::json!({
                "num_ctx": 32768,
                "temperature": 0.1
            })),
            keep_alive: None, // defaults to 24h in OllamaClient
            images: None,
        };

        let response = ollama
            .generate(req)
            .await
            .map_err(|e| (ErrorKind::CompileFail, format!("ollama: {e}")))?;

        let (test_code, skel_code) = parse_ollama_response(&response.response);

        let draft = Draft {
            frontmatter: DraftFrontmatter {
                crate_name: item.crate_name.clone(),
                upstream_repo: item.upstream_repo.clone(),
                upstream_file: item.upstream_file.clone(),
                upstream_fn: item.upstream_fn.clone(),
                status: DraftStatus::Draft,
                tier: 1,
                created_at: Utc::now(),
            },
            failing_test: test_code,
            skeleton: skel_code,
        };
        let rendered = draft.render();

        // Write draft file to <repo_root>/docs/drafts/.
        let drafts_dir = repo_root.join("docs").join("drafts");
        std::fs::create_dir_all(&drafts_dir)
            .map_err(|e| (ErrorKind::CompileFail, e.to_string()))?;
        let filename = draft_filename(&item.crate_name, &local_fn, Utc::now());
        let draft_path = drafts_dir.join(&filename);
        std::fs::write(&draft_path, &rendered)
            .map_err(|e| (ErrorKind::CompileFail, e.to_string()))?;

        // Run cargo test in the effective repo root.
        match self.run_cargo_test(&item.crate_name, repo_root).await {
            Ok(()) => {
                // Stage draft and commit onto the target branch.
                self.git(repo_root, &["add", &draft_path.to_string_lossy()])
                    .map_err(|e| (ErrorKind::CompileFail, e.to_string()))?;

                // Determine short repo name for commit prefix.
                let repo_short = repo_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                let msg = format!(
                    "[qwen-amele] {repo_short}/{id}: tier1({crate}): draft {fn_} — {repo}",
                    id = &item.id[..8],
                    crate = item.crate_name,
                    fn_ = item.upstream_fn,
                    repo = item.upstream_repo,
                );
                self.git_commit(repo_root, &msg)
                    .map_err(|e| (ErrorKind::CompileFail, e.to_string()))?;
                Ok(branch.to_string())
            }
            Err((kind, msg)) => {
                // Remove draft, no commit.
                let _ = std::fs::remove_file(&draft_path);
                Err((kind, msg))
            }
        }
    }

    /// Runs `cargo test -p <crate>` with the configured timeout.
    async fn run_cargo_test(
        &self,
        crate_name: &str,
        repo_root: &Path,
    ) -> Result<(), (ErrorKind, String)> {
        let mut child = tokio::process::Command::new("cargo")
            .args(["test", "-p", crate_name])
            .current_dir(repo_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| (ErrorKind::CompileFail, e.to_string()))?;

        match tokio::time::timeout(self.config.cargo_test_timeout, child.wait()).await {
            Err(_) => {
                let _ = child.kill().await;
                Err((
                    ErrorKind::Timeout,
                    format!("cargo test -p {crate_name} timed out"),
                ))
            }
            Ok(Err(e)) => Err((ErrorKind::CompileFail, e.to_string())),
            Ok(Ok(status)) => {
                if status.success() {
                    Ok(())
                } else {
                    let kind = if status.code() == Some(101) {
                        ErrorKind::CompileFail
                    } else {
                        ErrorKind::TestFail
                    };
                    Err((
                        kind,
                        format!("cargo test -p {crate_name} exited {:?}", status.code()),
                    ))
                }
            }
        }
    }

    /// Poll `GET /api/version` on the configured Ollama URL until it returns
    /// success or `budget` elapses. Uses exponential-with-cap backoff starting
    /// at 5s up to 60s between probes, so we don't hammer the server while it
    /// boots but also don't dawdle once it's healthy.
    ///
    /// Returns `OllamaUnreachable` when the budget is exhausted, which causes
    /// `run()` to exit; launchd's `KeepAlive + ThrottleInterval` will then
    /// bring us back up automatically so the daemon self-heals once Ollama
    /// comes back.
    async fn wait_for_ollama(&self, budget: Duration) -> Result<(), DaemonError> {
        let client = OllamaClient::new(&self.config.ollama_url);
        let start = std::time::Instant::now();
        let mut backoff = Duration::from_secs(5);
        let max_backoff = Duration::from_secs(60);
        // Overwritten on every failed probe; initial value is only observed
        // if the budget is already zero (test or misconfig).
        #[allow(unused_assignments)]
        let mut last_error = String::from("no probe attempted yet");
        let mut attempts: u32 = 0;

        loop {
            attempts += 1;
            match client.health_check().await {
                Ok(v) => {
                    info!(
                        ollama_url = %self.config.ollama_url,
                        version = %v.version,
                        attempts,
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        "ollama reachable"
                    );
                    return Ok(());
                }
                Err(e) => {
                    last_error = e.to_string();
                    let elapsed = start.elapsed();
                    if elapsed >= budget {
                        warn!(
                            ollama_url = %self.config.ollama_url,
                            attempts,
                            elapsed_secs = elapsed.as_secs(),
                            err = %last_error,
                            "ollama still unreachable — exiting so launchd can retry"
                        );
                        return Err(DaemonError::OllamaUnreachable {
                            budget_secs: budget.as_secs(),
                            last_error,
                        });
                    }
                    let remaining = budget - elapsed;
                    let sleep_for = backoff.min(remaining);
                    warn!(
                        ollama_url = %self.config.ollama_url,
                        attempts,
                        err = %last_error,
                        retry_in_secs = sleep_for.as_secs(),
                        "ollama health_check failed — retrying"
                    );
                    sleep(sleep_for).await;
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }

    /// Ensures the repo is on `branch`, creating it if needed.
    ///
    /// Idempotent:
    /// 1. If HEAD already == branch → no-op.
    /// 2. If branch exists locally  → `git checkout` (with `-m` fallback for dirty tree).
    /// 3. If branch does NOT exist  → `git checkout -b`; on "already exists" race
    ///    (e.g., another worktree just created it), log warn and fall back to plain checkout.
    ///
    /// Never propagates a `git checkout -b … already exists` error, which used to fail
    /// every tick once the branch had been created once.
    fn ensure_branch(&self, repo_root: &Path, branch: &str) -> Result<(), DaemonError> {
        let head = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_root)
            .output()?;
        if String::from_utf8_lossy(&head.stdout).trim() == branch {
            return Ok(());
        }

        // Does a local branch with this name already exist?
        let exists = std::process::Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .current_dir(repo_root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if exists {
            // Branch exists — just switch to it. Fallback to -m for dirty tree.
            self.git(repo_root, &["checkout", branch])
                .or_else(|_| self.git(repo_root, &["checkout", "-m", branch]))
        } else {
            // Branch does not exist — create it. Tolerate a race where another
            // worktree created it between our rev-parse check and `checkout -b`.
            match self.git(repo_root, &["checkout", "-b", branch]) {
                Ok(()) => Ok(()),
                Err(DaemonError::Git(ref err)) if err.contains("already exists") => {
                    warn!(
                        repo = %repo_root.display(),
                        %branch,
                        "branch already exists (race); falling back to plain checkout"
                    );
                    self.git(repo_root, &["checkout", branch])
                }
                Err(e) => Err(e),
            }
        }
    }

    fn git(&self, repo_root: &Path, args: &[&str]) -> Result<(), DaemonError> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(DaemonError::Git(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }

    fn git_commit(&self, repo_root: &Path, message: &str) -> Result<(), DaemonError> {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.email=cave-local-llm@localhost",
                "-c",
                "user.name=cave-local-llm",
                "commit",
                "-m",
                message,
            ])
            .current_dir(repo_root)
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(DaemonError::Git(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_kind_display_test_fail() {
        assert_eq!(ErrorKind::TestFail.to_string(), "test_fail");
    }

    #[test]
    fn test_error_kind_display_compile_fail() {
        assert_eq!(ErrorKind::CompileFail.to_string(), "compile_fail");
    }

    #[test]
    fn test_error_kind_display_timeout() {
        assert_eq!(ErrorKind::Timeout.to_string(), "timeout");
    }

    #[test]
    fn test_default_weekly_branch_format() {
        use chrono::TimeZone;
        let dt = chrono::Utc.with_ymd_and_hms(2026, 4, 22, 0, 0, 0).unwrap();
        let branch = default_weekly_branch(dt.iso_week());
        assert_eq!(branch, "qwen/auto-2026-W17");
    }

    // Env-mutating tests share a mutex to prevent parallel pollution.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_daemon_config_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: guarded by ENV_LOCK — no concurrent env reads.
        unsafe {
            std::env::remove_var("OLLAMA_MODEL");
            std::env::remove_var("OLLAMA_URL");
            std::env::remove_var("CAVE_DAEMON_BRANCH");
            std::env::remove_var("CAVE_SECONDARY_REPO_PATH");
        }
        let cfg = DaemonConfig::new("/tmp");
        assert_eq!(cfg.ollama_model, crate::ollama::DEFAULT_MODEL);
        assert_eq!(cfg.ollama_url, "http://localhost:11434");
        assert!(cfg.secondary_repos.is_empty());
    }

    #[test]
    fn test_daemon_config_env_override() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: guarded by ENV_LOCK — no concurrent env reads.
        unsafe {
            std::env::set_var("CAVE_DAEMON_BRANCH", "qwen/auto-test");
            std::env::set_var("OLLAMA_MODEL", "qwen2.5-coder:7b");
        }
        let cfg = DaemonConfig::new("/tmp");
        assert_eq!(cfg.target_branch, "qwen/auto-test");
        assert_eq!(cfg.ollama_model, "qwen2.5-coder:7b");
        unsafe {
            std::env::remove_var("CAVE_DAEMON_BRANCH");
            std::env::remove_var("OLLAMA_MODEL");
        }
    }

    #[test]
    fn test_daemon_config_secondary_repo_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("CAVE_SECONDARY_REPO_PATH", "/tmp/pipeline-platform-starter");
            std::env::set_var("CAVE_SECONDARY_REPO_WEIGHT", "0.3");
        }
        let cfg = DaemonConfig::new("/tmp");
        assert_eq!(cfg.secondary_repos.len(), 1);
        assert_eq!(
            cfg.secondary_repos[0].workspace_root,
            PathBuf::from("/tmp/pipeline-platform-starter")
        );
        assert!((cfg.secondary_repos[0].weight - 0.3).abs() < 0.001);
        unsafe {
            std::env::remove_var("CAVE_SECONDARY_REPO_PATH");
            std::env::remove_var("CAVE_SECONDARY_REPO_WEIGHT");
        }
    }
}
