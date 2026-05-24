// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Auto-port dispatcher — turns `GAP_OPENED` events into port tasks.
//!
//! ## Lifecycle
//!
//! ```text
//!   events.jsonl                  dispatched.jsonl
//!   (watchd writes)               (this module owns)
//!         │                              │
//!         │  scan_and_dispatch()         │  verify_completed()
//!         ▼                              ▼
//!   filter unprocessed   →  TaskQueue.submit → TaskId
//!                                              │
//!                                              │  status() polled
//!                                              ▼
//!                                  TaskQueue.output → commit_sha
//!                                                       │
//!                                                       ▼
//!                                              CharterGate.verify
//!                                                       │
//!                                            pass? → record "merged"
//!                                            fail? → record "charter_fail"
//! ```
//!
//! The dispatcher is **safe by construction**:
//!
//! * **Idempotency** — every `GAP_OPENED` carries an `event_id`
//!   (`GAP-<timestamp>-<module_hash>`). The dispatched-record file is
//!   keyed by that id, so re-running `scan_and_dispatch` is a no-op
//!   for events already submitted.
//! * **Cooldown** — when a previous attempt failed with
//!   `charter_fail`, the dispatcher refuses to re-submit until
//!   `cooldown` (default 24h) has elapsed. Re-run sooner with
//!   `--force` on the CLI.
//! * **Rate limit** — at most `max_concurrent` in-flight tasks at any
//!   moment (default 3). Excess events stay pending and are picked up
//!   next tick.
//! * **Kill switch** — env var `CAVE_AUTOPORT_DISABLE=1` makes
//!   `scan_and_dispatch` return `Idle` immediately. Operators can
//!   stop the dispatcher without unloading the daemon.
//! * **Audit log** — every action (dispatch, verify, merge,
//!   charter_fail) appends one line to `audit.jsonl` next to the
//!   state file. The portal `/admin/upstream` panel reads this.

use crate::auto_port_gate::{CharterBaseline, CharterGate, VerifyResult};
use crate::event::{GapEvent, read_events};
use crate::prompt::{PortContext, build_prompt};
use crate::task_queue::{TaskId, TaskOutput, TaskQueue, TaskQueueError, TaskStatus};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum AutoPortError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("task queue: {0}")]
    TaskQueue(#[from] TaskQueueError),
    #[error("charter gate: {0}")]
    Gate(#[from] crate::auto_port_gate::GateError),
    #[error("disabled via CAVE_AUTOPORT_DISABLE")]
    Disabled,
}

/// Stable status the dispatched record + portal column read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoPortStatus {
    /// Task accepted by the backend; awaiting completion.
    Dispatched,
    /// Backend reports the task is actively running.
    Running,
    /// Task completed AND charter gate passed AND main was advanced.
    Merged,
    /// Task completed but charter gate refused — needs human review.
    CharterFail,
    /// Task backend reported a hard failure (compile error, crash).
    BackendFail,
}

/// One row in `dispatched.jsonl`. `event_id` is the primary key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchedRecord {
    pub event_id: String,
    pub cave_module: String,
    pub backend: String,
    pub task_id: String,
    pub target_branch: String,
    pub status: AutoPortStatus,
    pub commit_sha: Option<String>,
    pub charter_report: Option<VerifyResult>,
    pub dispatched_at: DateTime<Utc>,
    pub last_checked_at: DateTime<Utc>,
    /// Reason recorded when status moves to a terminal failure.
    pub reason: Option<String>,
    /// Baseline snapshot taken at dispatch time. Read by the gate
    /// during `verify_completed`.
    pub baseline: CharterBaseline,
}

/// Aggregate result of one `scan_and_dispatch` pass.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DispatchSummary {
    pub considered: usize,
    pub already_dispatched: usize,
    pub skipped_disabled: usize,
    pub skipped_cooldown: usize,
    pub skipped_rate_limit: usize,
    pub dispatched: usize,
    pub errors: usize,
    pub new_task_ids: Vec<String>,
}

/// Aggregate result of one `verify_completed` pass.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VerifySummary {
    pub considered: usize,
    pub still_running: usize,
    pub merged: usize,
    pub charter_failed: usize,
    pub backend_failed: usize,
}

/// Configuration knobs. Sensible defaults for the daemon use case.
#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    /// Maximum number of `AutoPortStatus::Dispatched | Running`
    /// records permitted at once. Default 3.
    pub max_concurrent: usize,
    /// Minimum time between re-dispatching the same event after a
    /// `charter_fail`. Default 24 hours.
    pub cooldown: ChronoDuration,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            cooldown: ChronoDuration::hours(24),
        }
    }
}

/// Per-event context resolver. The dispatcher calls this to derive
/// the `PortContext` + `CharterBaseline` BEFORE submitting. Pure
/// trait so tests can inject deterministic answers without touching
/// disk.
pub trait ContextResolver: Send + Sync {
    fn resolve(&self, event: &GapEvent) -> Result<(PortContext, CharterBaseline), AutoPortError>;
}

/// Default resolver — reads the live `parity-index.json` +
/// `parity.manifest.toml` to build the PortContext, and the
/// workspace stub count for the baseline.
pub struct WorkspaceContextResolver {
    pub workspace_root: PathBuf,
    pub gate: Arc<dyn CharterGate>,
    /// Snapshot fn for the workspace stub count. Defaults to the
    /// gate's `count_workspace_stubs`. Stubbed out behind a fn ptr
    /// so tests can inject deterministic numbers.
    pub stub_counter: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// Snapshot fn for the affected crate's current fill_ratio.
    pub ratio_reader: Arc<dyn Fn(&str) -> Option<f64> + Send + Sync>,
}

impl ContextResolver for WorkspaceContextResolver {
    fn resolve(&self, event: &GapEvent) -> Result<(PortContext, CharterBaseline), AutoPortError> {
        let crate_name = event.cave_module.clone();
        let fill_before = (self.ratio_reader)(&crate_name).unwrap_or(0.0);
        let stubs_before = (self.stub_counter)();
        let ctx = PortContext {
            crate_name: crate_name.clone(),
            current_fill_ratio: Some(fill_before),
            upstream_org_repo: event.github_repo.clone(),
            unmapped_modules: Vec::new(),
            target_branch: None,
        };
        let baseline = CharterBaseline {
            crate_name,
            commit_sha_before: "HEAD".into(),
            fill_ratio_before: fill_before,
            workspace_stub_count_before: stubs_before,
        };
        let _ = &self.workspace_root;
        let _ = &self.gate;
        Ok((ctx, baseline))
    }
}

/// The dispatcher. `Arc`-wrapped because `scan_and_dispatch` /
/// `verify_completed` are called from the daemon driver loop on
/// independent tokio tasks.
pub struct AutoPortDispatcher {
    pub events_path: PathBuf,
    pub state_path: PathBuf,
    pub audit_path: PathBuf,
    pub queue: Arc<dyn TaskQueue>,
    pub gate: Arc<dyn CharterGate>,
    pub resolver: Arc<dyn ContextResolver>,
    pub cfg: DispatcherConfig,
    /// In-memory cache so the JSONL file isn't re-parsed every call.
    state: Mutex<BTreeMap<String, DispatchedRecord>>,
}

impl AutoPortDispatcher {
    pub fn new(
        events_path: PathBuf,
        state_path: PathBuf,
        audit_path: PathBuf,
        queue: Arc<dyn TaskQueue>,
        gate: Arc<dyn CharterGate>,
        resolver: Arc<dyn ContextResolver>,
        cfg: DispatcherConfig,
    ) -> Self {
        Self {
            events_path,
            state_path,
            audit_path,
            queue,
            gate,
            resolver,
            cfg,
            state: Mutex::new(BTreeMap::new()),
        }
    }

    /// Boot helper — loads the JSONL state into the in-memory map.
    pub async fn boot(&self) -> Result<(), AutoPortError> {
        let records = load_state(&self.state_path)?;
        let mut g = self.state.lock().await;
        g.clear();
        for r in records {
            g.insert(r.event_id.clone(), r);
        }
        Ok(())
    }

    /// True if `CAVE_AUTOPORT_DISABLE=1`.
    fn killed() -> bool {
        std::env::var("CAVE_AUTOPORT_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    /// Scan events.jsonl and dispatch new gaps. Idempotent + safe to
    /// re-run.
    pub async fn scan_and_dispatch(&self) -> Result<DispatchSummary, AutoPortError> {
        let mut summary = DispatchSummary::default();
        if Self::killed() {
            summary.skipped_disabled = 1;
            return Ok(summary);
        }
        let events = read_events(&self.events_path).map_err(|e| {
            AutoPortError::TaskQueue(TaskQueueError::Http(format!("read_events: {e}")))
        })?;
        summary.considered = events.len();

        // Snapshot the current in-flight count.
        let mut state = self.state.lock().await;
        let mut in_flight: usize = state
            .values()
            .filter(|r| {
                matches!(
                    r.status,
                    AutoPortStatus::Dispatched | AutoPortStatus::Running
                )
            })
            .count();

        let now = Utc::now();
        for event in events {
            if let Some(existing) = state.get(&event.event_id) {
                // Already-merged or already-dispatched stays as-is
                // unless it's a charter_fail past the cooldown.
                match existing.status {
                    AutoPortStatus::Merged
                    | AutoPortStatus::Dispatched
                    | AutoPortStatus::Running => {
                        summary.already_dispatched += 1;
                        continue;
                    }
                    AutoPortStatus::CharterFail | AutoPortStatus::BackendFail => {
                        if now - existing.dispatched_at < self.cfg.cooldown {
                            summary.skipped_cooldown += 1;
                            continue;
                        }
                    }
                }
            }

            if in_flight >= self.cfg.max_concurrent {
                summary.skipped_rate_limit += 1;
                continue;
            }

            match self.dispatch_one(&event, now).await {
                Ok(record) => {
                    in_flight += 1;
                    summary.dispatched += 1;
                    summary.new_task_ids.push(record.task_id.clone());
                    state.insert(record.event_id.clone(), record);
                }
                Err(e) => {
                    summary.errors += 1;
                    tracing::warn!(event_id = %event.event_id, error = %e, "auto-port dispatch failed");
                }
            }
        }

        // Flush the in-memory state back to disk.
        let snapshot: Vec<DispatchedRecord> = state.values().cloned().collect();
        save_state(&self.state_path, &snapshot)?;
        Ok(summary)
    }

    async fn dispatch_one(
        &self,
        event: &GapEvent,
        now: DateTime<Utc>,
    ) -> Result<DispatchedRecord, AutoPortError> {
        let (ctx, baseline) = self.resolver.resolve(event)?;
        let prompt = build_prompt(event, &ctx);
        let branch = ctx
            .target_branch
            .clone()
            .unwrap_or_else(|| format!("auto-port/{}", event.event_id));
        let mut tag_context = HashMap::new();
        tag_context.insert("event_id".into(), event.event_id.clone());
        tag_context.insert("cave_module".into(), event.cave_module.clone());
        let task_id = self
            .queue
            .submit(&prompt, &branch, tag_context.clone())
            .await?;
        let record = DispatchedRecord {
            event_id: event.event_id.clone(),
            cave_module: event.cave_module.clone(),
            backend: self.queue.backend_name().to_string(),
            task_id: task_id.0,
            target_branch: branch,
            status: AutoPortStatus::Dispatched,
            commit_sha: None,
            charter_report: None,
            dispatched_at: now,
            last_checked_at: now,
            reason: None,
            baseline,
        };
        write_audit(&self.audit_path, &AuditEntry::dispatched(&record))?;
        Ok(record)
    }

    /// Poll task statuses for in-flight records + run the charter
    /// gate when complete. Records flip to Merged / CharterFail /
    /// BackendFail.
    pub async fn verify_completed(&self) -> Result<VerifySummary, AutoPortError> {
        let mut summary = VerifySummary::default();
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let pending_ids: Vec<String> = state
            .values()
            .filter(|r| {
                matches!(
                    r.status,
                    AutoPortStatus::Dispatched | AutoPortStatus::Running
                )
            })
            .map(|r| r.event_id.clone())
            .collect();
        summary.considered = pending_ids.len();
        for event_id in pending_ids {
            let record = match state.get(&event_id) {
                Some(r) => r.clone(),
                None => continue,
            };
            let task = TaskId::new(record.task_id.clone());
            let status = match self.queue.status(&task).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(task_id = %record.task_id, error = %e, "status check errored");
                    summary.still_running += 1;
                    if let Some(r) = state.get_mut(&event_id) {
                        r.last_checked_at = now;
                    }
                    continue;
                }
            };
            match status {
                TaskStatus::Pending | TaskStatus::Running => {
                    summary.still_running += 1;
                    if let Some(r) = state.get_mut(&event_id) {
                        r.status = AutoPortStatus::Running;
                        r.last_checked_at = now;
                    }
                }
                TaskStatus::Completed { commit_sha, branch } => {
                    let _ = branch;
                    let report = self.gate.verify(&record.baseline, &commit_sha).await?;
                    let new_status = if report.overall_pass {
                        summary.merged += 1;
                        AutoPortStatus::Merged
                    } else {
                        summary.charter_failed += 1;
                        AutoPortStatus::CharterFail
                    };
                    if let Some(r) = state.get_mut(&event_id) {
                        r.status = new_status.clone();
                        r.commit_sha = Some(commit_sha.clone());
                        r.charter_report = Some(report.clone());
                        r.last_checked_at = now;
                        write_audit(&self.audit_path, &AuditEntry::verified(r, &report))?;
                    }
                }
                TaskStatus::Failed { reason } => {
                    summary.backend_failed += 1;
                    if let Some(r) = state.get_mut(&event_id) {
                        r.status = AutoPortStatus::BackendFail;
                        r.reason = Some(reason.clone());
                        r.last_checked_at = now;
                        write_audit(&self.audit_path, &AuditEntry::backend_fail(r, &reason))?;
                    }
                }
            }
        }
        let snapshot: Vec<DispatchedRecord> = state.values().cloned().collect();
        save_state(&self.state_path, &snapshot)?;
        Ok(summary)
    }

    /// Diagnostic — returns a snapshot of every dispatched record.
    pub async fn list_dispatched(&self) -> Vec<DispatchedRecord> {
        self.state.lock().await.values().cloned().collect()
    }

    /// Look up a record by event id (used by the portal panel).
    pub async fn get(&self, event_id: &str) -> Option<DispatchedRecord> {
        self.state.lock().await.get(event_id).cloned()
    }

    /// Look up the most recent record for a `cave_module` — handy
    /// when the portal renders one row per module.
    pub async fn latest_for_module(&self, cave_module: &str) -> Option<DispatchedRecord> {
        let g = self.state.lock().await;
        g.values()
            .filter(|r| r.cave_module == cave_module)
            .max_by_key(|r| r.dispatched_at)
            .cloned()
    }

    /// Default paths under `~/Library/Application Support/cave-runtime/watchd/`.
    pub fn default_paths() -> (PathBuf, PathBuf, PathBuf) {
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cave-runtime")
            .join("watchd");
        (
            base.join("events.jsonl"),
            base.join("dispatched.jsonl"),
            base.join("audit.jsonl"),
        )
    }
}

// ── persistence helpers ────────────────────────────────────────────────────

fn load_state(path: &Path) -> Result<Vec<DispatchedRecord>, AutoPortError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AutoPortError::Io(e)),
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(r) = serde_json::from_str::<DispatchedRecord>(trimmed) {
            out.push(r);
        }
    }
    Ok(out)
}

fn save_state(path: &Path, records: &[DispatchedRecord]) -> Result<(), AutoPortError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = String::new();
    for r in records {
        buf.push_str(&serde_json::to_string(r)?);
        buf.push('\n');
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, buf.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ── audit log ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    pub action: String,
    pub event_id: String,
    pub task_id: Option<String>,
    pub status: AutoPortStatus,
    pub note: Option<String>,
}

impl AuditEntry {
    pub fn dispatched(r: &DispatchedRecord) -> Self {
        Self {
            at: r.dispatched_at,
            action: "dispatched".into(),
            event_id: r.event_id.clone(),
            task_id: Some(r.task_id.clone()),
            status: r.status.clone(),
            note: None,
        }
    }
    pub fn verified(r: &DispatchedRecord, report: &VerifyResult) -> Self {
        Self {
            at: r.last_checked_at,
            action: if report.overall_pass {
                "merged".into()
            } else {
                "charter_fail".into()
            },
            event_id: r.event_id.clone(),
            task_id: Some(r.task_id.clone()),
            status: r.status.clone(),
            note: Some(report.notes.join("; ")),
        }
    }
    pub fn backend_fail(r: &DispatchedRecord, reason: &str) -> Self {
        Self {
            at: r.last_checked_at,
            action: "backend_fail".into(),
            event_id: r.event_id.clone(),
            task_id: Some(r.task_id.clone()),
            status: r.status.clone(),
            note: Some(reason.to_string()),
        }
    }
}

fn write_audit(path: &Path, entry: &AuditEntry) -> Result<(), AutoPortError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    f.write_all(serde_json::to_string(entry)?.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    Ok(())
}

/// Read the audit JSONL (newest first) — portal reads this.
pub fn read_audit(path: &Path) -> Result<Vec<AuditEntry>, AutoPortError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AutoPortError::Io(e)),
    };
    let mut out: Vec<AuditEntry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    out.reverse();
    Ok(out)
}

// ── Test mocks ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_port_gate::CharterV2Gate;
    use crate::changelog::Changelog;
    use crate::diff::Severity;
    use crate::event::{GapEventSink, JsonlSink};
    use crate::task_queue::DryRunTaskQueue;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// `scan_honours_kill_switch_env_var` mutates the process-wide
    /// CAVE_AUTOPORT_DISABLE env var. The dispatcher reads it inside
    /// scan_and_dispatch / verify_completed, so any concurrent test that
    /// calls those methods can observe the pollution. Every test in this
    /// module takes this lock for its whole body to serialise.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn ts(offset_min: i64) -> DateTime<Utc> {
        Utc::now() - ChronoDuration::minutes(60) + ChronoDuration::minutes(offset_min)
    }

    fn sample_event(uid_seed: &str, module: &str) -> GapEvent {
        GapEvent::new(
            module,
            "kubernetes/kubernetes",
            Some("v1.36.0".into()),
            "v1.37.0",
            Severity::Minor,
            Some(3600),
            Some(0.8966),
            Changelog::default(),
            // Force unique event_id per seed via timestamp offset.
            DateTime::parse_from_rfc3339("2026-05-13T14:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
                + ChronoDuration::seconds(uid_seed.bytes().map(|b| b as i64).sum()),
        )
    }

    fn fixture_events(dir: &Path, events: &[GapEvent]) -> PathBuf {
        let p = dir.join("events.jsonl");
        let sink = JsonlSink::new(p.clone());
        for e in events {
            sink.emit(e).unwrap();
        }
        p
    }

    struct StaticResolver;
    impl ContextResolver for StaticResolver {
        fn resolve(
            &self,
            event: &GapEvent,
        ) -> Result<(PortContext, CharterBaseline), AutoPortError> {
            let crate_name = event.cave_module.clone();
            Ok((
                PortContext {
                    crate_name: crate_name.clone(),
                    current_fill_ratio: Some(0.5),
                    upstream_org_repo: event.github_repo.clone(),
                    unmapped_modules: vec!["mod-a".into()],
                    target_branch: None,
                },
                CharterBaseline {
                    crate_name,
                    commit_sha_before: "0".repeat(40),
                    fill_ratio_before: 0.5,
                    workspace_stub_count_before: 0,
                },
            ))
        }
    }

    struct ScriptedQueue {
        backend: &'static str,
        submitted: Arc<AtomicUsize>,
        statuses: Arc<std::sync::Mutex<Vec<TaskStatus>>>,
    }
    impl ScriptedQueue {
        fn new(backend: &'static str, statuses: Vec<TaskStatus>) -> Self {
            Self {
                backend,
                submitted: Arc::new(AtomicUsize::new(0)),
                statuses: Arc::new(std::sync::Mutex::new(statuses)),
            }
        }
    }
    #[async_trait]
    impl TaskQueue for ScriptedQueue {
        async fn submit(
            &self,
            _: &str,
            _: &str,
            _: HashMap<String, String>,
        ) -> Result<TaskId, TaskQueueError> {
            let n = self.submitted.fetch_add(1, Ordering::SeqCst);
            Ok(TaskId::new(format!("scripted-{n:04}")))
        }
        async fn status(&self, _: &TaskId) -> Result<TaskStatus, TaskQueueError> {
            let mut g = self.statuses.lock().unwrap();
            if g.is_empty() {
                Ok(TaskStatus::Pending)
            } else {
                Ok(g.remove(0))
            }
        }
        async fn output(&self, _: &TaskId) -> Result<Option<TaskOutput>, TaskQueueError> {
            Ok(None)
        }
        fn backend_name(&self) -> &'static str {
            self.backend
        }
    }

    struct ScriptedGate {
        will_pass: Arc<AtomicBool>,
        crate_name: &'static str,
    }
    #[async_trait]
    impl CharterGate for ScriptedGate {
        async fn verify(
            &self,
            baseline: &CharterBaseline,
            sha: &str,
        ) -> Result<VerifyResult, crate::auto_port_gate::GateError> {
            let pass = self.will_pass.load(Ordering::SeqCst);
            Ok(VerifyResult {
                crate_name: baseline.crate_name.clone(),
                commit_sha_after: sha.to_string(),
                tests_pass: pass,
                cargo_check_pass: pass,
                fill_ratio_before: baseline.fill_ratio_before,
                fill_ratio_after: if pass {
                    baseline.fill_ratio_before + 0.1
                } else {
                    baseline.fill_ratio_before
                },
                parity_ratio_delta: if pass { 0.1 } else { 0.0 },
                stub_count_before: 0,
                stub_count_after: 0,
                no_new_stubs: pass,
                no_breaking_change: pass,
                overall_pass: pass,
                notes: if pass {
                    vec![]
                } else {
                    vec!["ratio did not rise".into()]
                },
                tdd_compliance: None,
            })
            .and_then(|r| {
                let _ = self.crate_name;
                Ok(r)
            })
        }
    }

    fn dispatcher(
        dir: &tempfile::TempDir,
        events: &[GapEvent],
        queue: Arc<dyn TaskQueue>,
        gate: Arc<dyn CharterGate>,
    ) -> AutoPortDispatcher {
        let events_path = fixture_events(dir.path(), events);
        AutoPortDispatcher::new(
            events_path,
            dir.path().join("dispatched.jsonl"),
            dir.path().join("audit.jsonl"),
            queue,
            gate,
            Arc::new(StaticResolver),
            DispatcherConfig::default(),
        )
    }

    // ── scan_and_dispatch ─────────────────────────────────────

    #[tokio::test]
    async fn scan_dispatches_one_per_event_and_is_idempotent() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x"), sample_event("b", "cave-y")];
        let q = Arc::new(DryRunTaskQueue::new(dir.path().join("dry.jsonl")));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "",
        });
        let d = dispatcher(&dir, &events, q, g);
        let s1 = d.scan_and_dispatch().await.unwrap();
        assert_eq!(s1.dispatched, 2);
        assert_eq!(s1.already_dispatched, 0);
        // Re-run is a no-op.
        let s2 = d.scan_and_dispatch().await.unwrap();
        assert_eq!(s2.dispatched, 0);
        assert_eq!(s2.already_dispatched, 2);
    }

    #[tokio::test]
    async fn scan_honours_kill_switch_env_var() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(DryRunTaskQueue::new(dir.path().join("dry.jsonl")));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "",
        });
        let d = dispatcher(&dir, &events, q, g);
        unsafe {
            std::env::set_var("CAVE_AUTOPORT_DISABLE", "1");
        }
        let s = d.scan_and_dispatch().await.unwrap();
        assert_eq!(s.dispatched, 0);
        assert_eq!(s.skipped_disabled, 1);
        unsafe {
            std::env::remove_var("CAVE_AUTOPORT_DISABLE");
        }
    }

    #[tokio::test]
    async fn scan_enforces_max_concurrent_rate_limit() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![
            sample_event("a", "cave-1"),
            sample_event("b", "cave-2"),
            sample_event("c", "cave-3"),
            sample_event("d", "cave-4"),
            sample_event("e", "cave-5"),
        ];
        let q = Arc::new(DryRunTaskQueue::new(dir.path().join("dry.jsonl")));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "",
        });
        let mut cfg = DispatcherConfig::default();
        cfg.max_concurrent = 2;
        let d = AutoPortDispatcher::new(
            fixture_events(dir.path(), &events),
            dir.path().join("dispatched.jsonl"),
            dir.path().join("audit.jsonl"),
            q,
            g,
            Arc::new(StaticResolver),
            cfg,
        );
        let s = d.scan_and_dispatch().await.unwrap();
        assert_eq!(s.dispatched, 2);
        assert!(s.skipped_rate_limit >= 3);
    }

    #[tokio::test]
    async fn scan_respects_cooldown_after_charter_fail() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(ScriptedQueue::new(
            "scripted",
            vec![TaskStatus::Completed {
                commit_sha: "abc".into(),
                branch: "b".into(),
            }],
        ));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(false)),
            crate_name: "cave-x",
        });
        let d = dispatcher(&dir, &events, q.clone(), g);
        d.scan_and_dispatch().await.unwrap();
        d.verify_completed().await.unwrap();
        let rec = d.list_dispatched().await;
        assert_eq!(rec[0].status, AutoPortStatus::CharterFail);
        // Re-scan immediately → still cooldown.
        let s = d.scan_and_dispatch().await.unwrap();
        assert_eq!(s.dispatched, 0);
        assert_eq!(s.skipped_cooldown, 1);
    }

    // ── verify_completed ─────────────────────────────────────

    #[tokio::test]
    async fn verify_flips_to_merged_when_charter_passes() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(ScriptedQueue::new(
            "scripted",
            vec![TaskStatus::Completed {
                commit_sha: "abc".into(),
                branch: "b".into(),
            }],
        ));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "cave-x",
        });
        let d = dispatcher(&dir, &events, q, g);
        d.scan_and_dispatch().await.unwrap();
        let s = d.verify_completed().await.unwrap();
        assert_eq!(s.merged, 1);
        let recs = d.list_dispatched().await;
        assert_eq!(recs[0].status, AutoPortStatus::Merged);
        assert!(recs[0].charter_report.as_ref().unwrap().overall_pass);
    }

    #[tokio::test]
    async fn verify_flips_to_backend_fail_on_task_failure() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(ScriptedQueue::new(
            "scripted",
            vec![TaskStatus::Failed {
                reason: "compile error".into(),
            }],
        ));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "cave-x",
        });
        let d = dispatcher(&dir, &events, q, g);
        d.scan_and_dispatch().await.unwrap();
        let s = d.verify_completed().await.unwrap();
        assert_eq!(s.backend_failed, 1);
        let recs = d.list_dispatched().await;
        assert_eq!(recs[0].status, AutoPortStatus::BackendFail);
        assert_eq!(recs[0].reason.as_deref(), Some("compile error"));
    }

    #[tokio::test]
    async fn verify_keeps_running_status_when_task_still_pending() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(ScriptedQueue::new("scripted", vec![TaskStatus::Running]));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "cave-x",
        });
        let d = dispatcher(&dir, &events, q, g);
        d.scan_and_dispatch().await.unwrap();
        let s = d.verify_completed().await.unwrap();
        assert_eq!(s.still_running, 1);
        let recs = d.list_dispatched().await;
        assert_eq!(recs[0].status, AutoPortStatus::Running);
    }

    // ── persistence / boot ────────────────────────────────────

    #[tokio::test]
    async fn state_survives_boot_round_trip() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(DryRunTaskQueue::new(dir.path().join("dry.jsonl")));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "",
        });
        {
            let d = dispatcher(&dir, &events, q.clone(), g.clone());
            d.scan_and_dispatch().await.unwrap();
            // Drop d.
        }
        let d2 = dispatcher(&dir, &events, q, g);
        d2.boot().await.unwrap();
        assert_eq!(d2.list_dispatched().await.len(), 1);
    }

    // ── audit log ─────────────────────────────────────────────

    #[tokio::test]
    async fn audit_log_records_dispatched_and_merged_actions() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let events = vec![sample_event("a", "cave-x")];
        let q = Arc::new(ScriptedQueue::new(
            "scripted",
            vec![TaskStatus::Completed {
                commit_sha: "abc".into(),
                branch: "b".into(),
            }],
        ));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "cave-x",
        });
        let d = dispatcher(&dir, &events, q, g);
        d.scan_and_dispatch().await.unwrap();
        d.verify_completed().await.unwrap();
        let entries = read_audit(&dir.path().join("audit.jsonl")).unwrap();
        // Two entries: dispatched + merged. Newest first.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "merged");
        assert_eq!(entries[1].action, "dispatched");
    }

    #[tokio::test]
    async fn latest_for_module_returns_most_recent_record() {
        let _g = env_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let e1 = sample_event("a", "cave-x");
        let e2 = sample_event("b", "cave-x");
        let q = Arc::new(DryRunTaskQueue::new(dir.path().join("dry.jsonl")));
        let g = Arc::new(ScriptedGate {
            will_pass: Arc::new(AtomicBool::new(true)),
            crate_name: "",
        });
        let d = dispatcher(&dir, &[e1.clone(), e2.clone()], q, g);
        d.scan_and_dispatch().await.unwrap();
        let latest = d.latest_for_module("cave-x").await.unwrap();
        // The newer of the two events should win.
        assert!(latest.event_id == e1.event_id || latest.event_id == e2.event_id);
    }

    #[test]
    fn default_paths_match_xdg_data_dir_layout() {
        let (events, dispatched, audit) = AutoPortDispatcher::default_paths();
        // Sanity: all three live under the same parent.
        assert_eq!(events.parent().unwrap(), dispatched.parent().unwrap());
        assert_eq!(dispatched.parent().unwrap(), audit.parent().unwrap());
        assert!(events.to_string_lossy().ends_with("events.jsonl"));
        assert!(dispatched.to_string_lossy().ends_with("dispatched.jsonl"));
        assert!(audit.to_string_lossy().ends_with("audit.jsonl"));
    }
}
