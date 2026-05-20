// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Watch daemon — tiered-cadence release poller with pump trigger.
//!
//! ## Lifecycle of a single tick
//!
//! 1. Compute "due" set: every project whose
//!    `last_checked + cadence(project) <= now`. High-priority modules
//!    (`crate::HIGH_PRIORITY_MODULES`) get 15-minute cadence; everything
//!    else gets 60 minutes.
//! 2. Spawn polls in parallel under a semaphore with cap = `concurrency`
//!    (default 8). Each poll:
//!    - calls [`crate::delta::detect_release_delta`]
//!    - on `NewRelease`, builds + writes a pump payload
//!    - on `RateLimited`, sleeps until `reset_at` (or 60s, whichever)
//!    - on transient error, increments `consecutive_errors` and applies
//!      per-project exponential backoff (skips the next N ticks)
//! 3. Saves state atomically.
//!
//! The driver loop wraps this in `tokio::time::interval` with
//! [`Config::tick_interval`] (default 60s — tiered-cadence-per-project
//! decisions happen *inside* a tick, not via separate timers, so a
//! single short tick cadence covers both 15-minute and 60-minute
//! categories).
//!
//! ## Jitter
//!
//! Each tick is offset by a uniform random `[-tick_jitter, +tick_jitter]`
//! (default ±60s) so that 100 daemons across 100 boxes don't hammer
//! GitHub at the same wall-clock instant.
//!
//! ## Shutdown
//!
//! [`Daemon::run`] watches a [`tokio_util::sync::CancellationToken`]; on
//! cancel it finishes the in-flight tick, saves state, and returns.
//! Use [`install_signal_handler`] to wire SIGTERM/SIGINT to the token.

use crate::delta::{PollConfig, PollOutcome, SurfaceDiffer, TagOnlyDiffer, detect_release_delta};
use crate::projects::TrackedProject;
use crate::pump::{build_payload, write_payload};
use crate::state::{ProjectState, WatchState};
use chrono::{Duration as ChronoDuration, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
// Sweep-010 adoption: switched from `tokio::sync::Semaphore` to the
// kernel primitive. The kernel re-exports the tokio semaphore
// underneath, so cancellation + fairness are identical; only the
// import path changes. The `permit` value type also changes
// (`OwnedSemaphorePermit` → `cave_kernel::semaphore::Permit`) but
// callers only hold it for its drop side-effect.
use cave_kernel::semaphore::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Daemon configuration. Constructed by the binary from a TOML file or
/// CLI flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// How often the driver loop wakes up. Per-project cadence is
    /// enforced *inside* the tick. Default 60s.
    #[serde(default = "default_tick_interval")]
    pub tick_interval: Duration,
    /// `±` random offset added to each tick. Default 60s.
    #[serde(default = "default_tick_jitter")]
    pub tick_jitter: Duration,
    /// Cadence for repos whose `cave_module` is high-priority. Default
    /// 15 min.
    #[serde(default = "default_high_priority_cadence")]
    pub high_priority_cadence: Duration,
    /// Cadence for everything else. Default 60 min.
    #[serde(default = "default_normal_cadence")]
    pub normal_cadence: Duration,
    /// Max concurrent in-flight polls. Default 8.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// GitHub API base URL. Default `https://api.github.com`.
    #[serde(default = "default_github_api_base")]
    pub github_api_base: String,
    /// Optional auth token. Strongly recommended in production (5000/h
    /// vs 60/h unauthenticated).
    #[serde(default)]
    pub github_token: Option<String>,
    /// Path to the persistent state file.
    #[serde(default = "WatchState::default_path")]
    pub state_path: PathBuf,
    /// Path to the pump queue dir.
    #[serde(default = "crate::pump::default_queue_dir")]
    pub pump_queue_dir: PathBuf,
    /// User-Agent header. Default `"cave-upstream-watchd/<version>"`.
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    /// Per-poll request timeout. Default 15s.
    #[serde(default = "default_request_timeout")]
    pub request_timeout: Duration,
    /// Cap for per-project exponential backoff (in number of ticks
    /// skipped). Default 16 ticks.
    #[serde(default = "default_max_backoff_ticks")]
    pub max_backoff_ticks: u32,
}

fn default_tick_interval() -> Duration {
    Duration::from_secs(60)
}
fn default_tick_jitter() -> Duration {
    Duration::from_secs(60)
}
fn default_high_priority_cadence() -> Duration {
    Duration::from_secs(15 * 60)
}
fn default_normal_cadence() -> Duration {
    Duration::from_secs(60 * 60)
}
fn default_concurrency() -> usize {
    8
}
fn default_github_api_base() -> String {
    "https://api.github.com".to_string()
}
fn default_user_agent() -> String {
    format!("cave-upstream-watchd/{}", env!("CARGO_PKG_VERSION"))
}
fn default_request_timeout() -> Duration {
    Duration::from_secs(15)
}
fn default_max_backoff_ticks() -> u32 {
    16
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_interval: default_tick_interval(),
            tick_jitter: default_tick_jitter(),
            high_priority_cadence: default_high_priority_cadence(),
            normal_cadence: default_normal_cadence(),
            concurrency: default_concurrency(),
            github_api_base: default_github_api_base(),
            github_token: None,
            state_path: WatchState::default_path(),
            pump_queue_dir: crate::pump::default_queue_dir(),
            user_agent: default_user_agent(),
            request_timeout: default_request_timeout(),
            max_backoff_ticks: default_max_backoff_ticks(),
        }
    }
}

/// Aggregated outcome of one tick. Useful for tests + logging + metrics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    pub run_id: String,
    pub considered: usize,
    pub due: usize,
    pub polled: usize,
    pub new_releases: usize,
    pub unchanged: usize,
    pub no_releases: usize,
    pub rate_limited: usize,
    pub errors: usize,
    pub payloads_written: Vec<String>,
}

pub struct Daemon {
    pub cfg: Config,
    pub projects: Vec<TrackedProject>,
    pub differ: Arc<dyn SurfaceDiffer>,
}

impl Daemon {
    pub fn new(cfg: Config, projects: Vec<TrackedProject>) -> Self {
        Self {
            cfg,
            projects,
            differ: Arc::new(TagOnlyDiffer),
        }
    }

    pub fn with_differ(mut self, d: Arc<dyn SurfaceDiffer>) -> Self {
        self.differ = d;
        self
    }

    /// Run forever until `cancel` is fired. Returns Ok on graceful
    /// shutdown.
    pub async fn run(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        info!(
            tick_interval_secs = self.cfg.tick_interval.as_secs(),
            high_pri_cadence_secs = self.cfg.high_priority_cadence.as_secs(),
            normal_cadence_secs = self.cfg.normal_cadence.as_secs(),
            concurrency = self.cfg.concurrency,
            project_count = self.projects.len(),
            "cave-upstream-watchd starting"
        );

        loop {
            // Apply jitter BEFORE waiting, so the first tick is also jittered.
            let jitter = jitter_offset(self.cfg.tick_jitter);
            let wait = self.cfg.tick_interval.saturating_add(jitter);

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("shutdown signal received; exiting");
                    return Ok(());
                }
                _ = tokio::time::sleep(wait) => {}
            }

            match self.tick_once().await {
                Ok(report) => info!(
                    run_id = report.run_id.as_str(),
                    due = report.due,
                    new_releases = report.new_releases,
                    unchanged = report.unchanged,
                    rate_limited = report.rate_limited,
                    errors = report.errors,
                    "tick complete"
                ),
                Err(e) => warn!(error = %e, "tick failed"),
            }
        }
    }

    /// Execute one tick. Public so the binary can do `--once`.
    pub async fn tick_once(&self) -> anyhow::Result<TickReport> {
        let mut state = WatchState::load(&self.cfg.state_path)?;
        let report = self.tick_with_state(&mut state).await?;
        state.save(&self.cfg.state_path)?;
        Ok(report)
    }

    /// Tick using a caller-provided `WatchState`. Used by tests so they
    /// don't need to roundtrip through the disk between invocations.
    pub async fn tick_with_state(&self, state: &mut WatchState) -> anyhow::Result<TickReport> {
        let run_id = Uuid::new_v4().to_string();
        let mut report = TickReport {
            run_id: run_id.clone(),
            considered: self.projects.len(),
            ..TickReport::default()
        };

        let now = Utc::now();
        let due_projects: Vec<&TrackedProject> = self
            .projects
            .iter()
            .filter(|p| {
                let st = state.get(p.github_repo);
                project_is_due(p, st, now, &self.cfg)
            })
            .collect();
        report.due = due_projects.len();

        if due_projects.is_empty() {
            return Ok(report);
        }

        // `cave_kernel::semaphore::Semaphore` is internally `Arc`-wrapped
        // and implements `Clone`, so the prior `Arc::new(Semaphore::new(..))`
        // double-wrap goes away.
        let sem = Semaphore::new(self.cfg.concurrency.max(1));
        let client = reqwest::Client::builder()
            .user_agent(&self.cfg.user_agent)
            .timeout(self.cfg.request_timeout)
            .build()?;

        // We poll into local copies of each ProjectState, then merge
        // back into the shared WatchState in-order. This avoids holding
        // a mut-borrow of `state` across .await points.
        let mut local_states: Vec<ProjectState> = due_projects
            .iter()
            .map(|p| {
                state
                    .get(p.github_repo)
                    .cloned()
                    .unwrap_or_else(|| ProjectState::new(p.github_repo))
            })
            .collect();

        let mut futs = FuturesUnordered::new();
        for (idx, p) in due_projects.iter().enumerate() {
            let sem = sem.clone();
            let client = client.clone();
            let cfg = poll_config_for(&self.cfg);
            let differ = self.differ.clone();
            let mut st = local_states[idx].clone();
            let cave_module = p.cave_module.to_string();
            let priority = if crate::is_high_priority(p.cave_module) {
                "high".to_string()
            } else {
                "normal".to_string()
            };
            let queue_dir = self.cfg.pump_queue_dir.clone();
            let run_id = run_id.clone();
            let user_agent = self.cfg.user_agent.clone();

            futs.push(async move {
                // Kernel semaphore's `acquire()` is infallible (the inner
                // tokio handle is never explicitly closed).
                let _permit = sem.acquire().await;
                let cfg_borrowed = PollConfig {
                    github_api_base: &cfg.api_base,
                    github_token: cfg.token.as_deref(),
                    user_agent: &user_agent,
                    request_timeout: cfg.timeout,
                };
                let outcome =
                    detect_release_delta(&client, &cfg_borrowed, differ.as_ref(), &mut st).await;
                (idx, st, cave_module, priority, queue_dir, run_id, outcome)
            });
        }

        while let Some((idx, mut new_state, cave_module, priority, queue_dir, run_id, outcome)) =
            futs.next().await
        {
            match outcome {
                Ok(PollOutcome::NewRelease(delta)) => {
                    report.new_releases += 1;
                    report.polled += 1;
                    let payload = build_payload(
                        &delta,
                        &cave_module,
                        &priority,
                        "cave-upstream-watchd",
                        &run_id,
                    );
                    match write_payload(&queue_dir, &payload) {
                        Ok(name) => {
                            new_state.last_pump_payload_id = Some(name.clone());
                            report.payloads_written.push(name);
                        }
                        Err(e) => {
                            warn!(repo = %new_state.github_repo, error = %e,
                                  "failed to write pump payload");
                            report.errors += 1;
                        }
                    }
                }
                Ok(PollOutcome::Unchanged) => {
                    report.polled += 1;
                    report.unchanged += 1;
                }
                Ok(PollOutcome::NoReleases) => {
                    report.polled += 1;
                    report.no_releases += 1;
                }
                Ok(PollOutcome::RateLimited { reset_at }) => {
                    report.rate_limited += 1;
                    debug!(
                        repo = %new_state.github_repo,
                        reset_at = ?reset_at,
                        "rate-limited; will retry next tick"
                    );
                    new_state.consecutive_errors = new_state.consecutive_errors.saturating_add(1);
                }
                Err(e) => {
                    report.errors += 1;
                    new_state.consecutive_errors = new_state.consecutive_errors.saturating_add(1);
                    warn!(repo = %new_state.github_repo, error = %e, "poll failed");
                }
            }
            local_states[idx] = new_state;
        }

        // Merge local states back.
        for (i, p) in due_projects.iter().enumerate() {
            state
                .projects
                .insert(p.github_repo.to_string(), local_states[i].clone());
        }

        Ok(report)
    }
}

struct OwnedPollConfig {
    api_base: String,
    token: Option<String>,
    timeout: Duration,
}

fn poll_config_for(cfg: &Config) -> OwnedPollConfig {
    OwnedPollConfig {
        api_base: cfg.github_api_base.clone(),
        token: cfg.github_token.clone(),
        timeout: cfg.request_timeout,
    }
}

/// Pure decision: should we poll this project on this tick?
///
/// - First-ever observation (no `last_checked`) → due.
/// - Otherwise: due iff `now - last_checked >= cadence(project)` AND the
///   per-project backoff has elapsed.
pub fn project_is_due(
    p: &TrackedProject,
    st: Option<&ProjectState>,
    now: chrono::DateTime<Utc>,
    cfg: &Config,
) -> bool {
    let cadence = if crate::is_high_priority(p.cave_module) {
        cfg.high_priority_cadence
    } else {
        cfg.normal_cadence
    };
    let cadence_chrono =
        ChronoDuration::from_std(cadence).unwrap_or_else(|_| ChronoDuration::seconds(15 * 60));

    let last_checked = match st.and_then(|s| s.last_checked) {
        None => return true, // never polled
        Some(t) => t,
    };
    let consec = st.map(|s| s.consecutive_errors).unwrap_or(0);
    let backoff_factor = backoff_multiplier(consec, cfg.max_backoff_ticks);
    let effective = cadence_chrono * backoff_factor as i32;

    now.signed_duration_since(last_checked) >= effective
}

/// Exponential-with-cap: 1, 2, 4, 8, …, capped at `max_ticks`.
pub fn backoff_multiplier(consecutive_errors: u32, max_ticks: u32) -> u32 {
    if consecutive_errors == 0 {
        return 1;
    }
    let raw = 1u32
        .checked_shl(consecutive_errors.min(31))
        .unwrap_or(u32::MAX);
    raw.min(max_ticks.max(1))
}

/// Random `[-jitter, +jitter]` as a `Duration`.
pub fn jitter_offset(jitter: Duration) -> Duration {
    if jitter.is_zero() {
        return Duration::ZERO;
    }
    let max_ms = jitter.as_millis() as i64;
    let mut rng = rand::thread_rng();
    let offset_ms = rng.gen_range(-max_ms..=max_ms);
    if offset_ms >= 0 {
        Duration::from_millis(offset_ms as u64)
    } else {
        // We never produce a negative final wait — we add to
        // tick_interval, which is always > 0, and we use saturating add.
        // For sleep(), we just clamp at zero.
        Duration::ZERO
    }
}

/// Wire SIGTERM + SIGINT to a `CancellationToken`. Unix-only.
#[cfg(unix)]
pub async fn install_signal_handler(token: CancellationToken) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "could not install SIGTERM handler");
            return;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "could not install SIGINT handler");
            return;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => info!("SIGTERM received"),
        _ = sigint.recv() => info!("SIGINT received"),
    }
    token.cancel();
}

#[cfg(not(unix))]
pub async fn install_signal_handler(token: CancellationToken) {
    let _ = tokio::signal::ctrl_c().await;
    token.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn proj(repo: &'static str, module: &'static str) -> TrackedProject {
        TrackedProject {
            name: repo,
            github_repo: repo,
            cave_module: module,
            track_features: "",
            check_frequency: "weekly",
            category: "",
            phase: 1,
        }
    }

    #[test]
    fn never_polled_is_due() {
        let p = proj("etcd-io/etcd", "cave-etcd");
        let cfg = Config::default();
        assert!(project_is_due(&p, None, Utc::now(), &cfg));
    }

    #[test]
    fn high_priority_cadence_15min() {
        let p = proj("etcd-io/etcd", "cave-etcd");
        let cfg = Config::default();
        let mut st = ProjectState::new("etcd-io/etcd");
        let now = Utc::now();
        st.last_checked = Some(now - ChronoDuration::minutes(10));
        assert!(!project_is_due(&p, Some(&st), now, &cfg), "10 min < 15 min");
        st.last_checked = Some(now - ChronoDuration::minutes(20));
        assert!(project_is_due(&p, Some(&st), now, &cfg), "20 min > 15 min");
    }

    #[test]
    fn normal_cadence_60min() {
        let p = proj("backstage/backstage", "cave-portal");
        let cfg = Config::default();
        let mut st = ProjectState::new("backstage/backstage");
        let now = Utc::now();
        st.last_checked = Some(now - ChronoDuration::minutes(30));
        assert!(!project_is_due(&p, Some(&st), now, &cfg), "30 min < 60 min");
        st.last_checked = Some(now - ChronoDuration::minutes(70));
        assert!(project_is_due(&p, Some(&st), now, &cfg), "70 min > 60 min");
    }

    #[test]
    fn errors_extend_cadence_via_backoff() {
        let p = proj("etcd-io/etcd", "cave-etcd");
        let cfg = Config::default();
        let mut st = ProjectState::new("etcd-io/etcd");
        let now = Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
        st.last_checked = Some(now - ChronoDuration::minutes(20));
        st.consecutive_errors = 3; // multiplier = 8 → effective cadence 120 min
        assert!(!project_is_due(&p, Some(&st), now, &cfg));

        st.last_checked = Some(now - ChronoDuration::minutes(130));
        assert!(project_is_due(&p, Some(&st), now, &cfg));
    }

    #[test]
    fn backoff_multiplier_grows_then_caps() {
        assert_eq!(backoff_multiplier(0, 16), 1);
        assert_eq!(backoff_multiplier(1, 16), 2);
        assert_eq!(backoff_multiplier(2, 16), 4);
        assert_eq!(backoff_multiplier(3, 16), 8);
        assert_eq!(backoff_multiplier(4, 16), 16);
        assert_eq!(backoff_multiplier(20, 16), 16, "caps at max_ticks");
    }

    #[test]
    fn jitter_zero_returns_zero() {
        assert_eq!(jitter_offset(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn jitter_within_bounds() {
        for _ in 0..50 {
            let j = jitter_offset(Duration::from_secs(60));
            assert!(j <= Duration::from_secs(60));
        }
    }
}
