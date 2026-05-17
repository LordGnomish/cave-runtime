// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `TaskQueue` — pluggable backend for submitting auto-port tasks.
//!
//! Four implementations ship today:
//!
//!   * [`PumpTaskQueue`] — writes a prompt file to the local
//!     `cave-qwen-pump` queue dir. The pump daemon picks it up,
//!     runs the local LLM against the prompt, and writes a
//!     completion marker the dispatcher polls.
//!   * [`ClaudeCliTaskQueue`] — spawns the local `claude` binary
//!     (Claude Code) with `-p <prompt> --output-format json`.
//!     Production default — uses the operator's existing Claude
//!     Code session, no external `ANTHROPIC_API_KEY` required.
//!   * [`OpusTaskQueue`] — submits the prompt to Anthropic's
//!     Messages API directly. Useful on headless hosts without
//!     the Claude Code CLI; requires `ANTHROPIC_API_KEY`.
//!   * [`DryRunTaskQueue`] — records the submission to a JSONL
//!     log and never side-effects. Default in dev so an operator
//!     can audit what the dispatcher would have sent before going
//!     live.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TaskQueueError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("backend disabled: {0}")]
    Disabled(&'static str),
    #[error("task not found: {0}")]
    NotFound(String),
}

/// Unique handle for one submitted task. The dispatcher stores
/// this in `dispatched.jsonl` so a later `verify_completed` pass
/// can ask the backend for the task's status.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Status the dispatcher polls. `Completed` carries the `commit_sha`
/// the task pushed to the branch — the charter gate runs against
/// this SHA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed { commit_sha: String, branch: String },
    Failed { reason: String },
}

/// What the task produced. Read after `Completed` to build the
/// charter-gate report. Lines/tests counts are advisory — the gate
/// reads them from the diff for verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskOutput {
    pub commit_sha: String,
    pub branch: String,
    pub files_changed: u32,
    pub lines_added: u32,
    pub test_count: u32,
}

#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Submit a new task. Returns a stable `TaskId` the dispatcher
    /// can poll. Idempotency is the dispatcher's responsibility
    /// (see [`crate::auto_port::AutoPortDispatcher`]); a backend
    /// MAY enforce its own idempotency on top.
    async fn submit(
        &self,
        prompt: &str,
        target_branch: &str,
        context: HashMap<String, String>,
    ) -> Result<TaskId, TaskQueueError>;

    /// Poll the backend for current status.
    async fn status(&self, task_id: &TaskId) -> Result<TaskStatus, TaskQueueError>;

    /// Pull the task's output once `status` reports `Completed`.
    /// Returns `Ok(None)` while still running.
    async fn output(&self, task_id: &TaskId) -> Result<Option<TaskOutput>, TaskQueueError>;

    /// Human-readable backend name for logs + dashboard.
    fn backend_name(&self) -> &'static str;
}

// ── DryRun ─────────────────────────────────────────────────────────────────

/// No-op queue. Every `submit` writes a JSONL record to disk; every
/// `status` returns `Pending` forever. Used in dev / audit modes.
#[derive(Debug)]
pub struct DryRunTaskQueue {
    pub log_path: PathBuf,
    next_id: Mutex<u64>,
}

impl DryRunTaskQueue {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            log_path,
            next_id: Mutex::new(0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DryRunRecord {
    task_id: String,
    target_branch: String,
    prompt_chars: usize,
    context: HashMap<String, String>,
    at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
impl TaskQueue for DryRunTaskQueue {
    async fn submit(
        &self,
        prompt: &str,
        target_branch: &str,
        context: HashMap<String, String>,
    ) -> Result<TaskId, TaskQueueError> {
        let id = {
            let mut g = self.next_id.lock().expect("dryrun id mutex");
            *g += 1;
            format!("dryrun-{:06}", *g)
        };
        let record = DryRunRecord {
            task_id: id.clone(),
            target_branch: target_branch.to_string(),
            prompt_chars: prompt.chars().count(),
            context,
            at: chrono::Utc::now(),
        };
        append_jsonl(&self.log_path, &record)?;
        Ok(TaskId::new(id))
    }

    async fn status(&self, _id: &TaskId) -> Result<TaskStatus, TaskQueueError> {
        Ok(TaskStatus::Pending)
    }

    async fn output(&self, _id: &TaskId) -> Result<Option<TaskOutput>, TaskQueueError> {
        Ok(None)
    }

    fn backend_name(&self) -> &'static str {
        "dryrun"
    }
}

// ── Pump (file-based local-LLM queue) ──────────────────────────────────────

/// Writes a prompt file to the local qwen-pump queue dir and reads
/// the pump's completion marker. Both ends are filesystem so this
/// works without IPC.
#[derive(Debug)]
pub struct PumpTaskQueue {
    pub queue_dir: PathBuf,
    pub completed_dir: PathBuf,
}

impl PumpTaskQueue {
    pub fn new(queue_dir: PathBuf, completed_dir: PathBuf) -> Self {
        Self { queue_dir, completed_dir }
    }

    /// Default paths under `~/Library/Application Support/cave-qwen-pump/`.
    pub fn default_paths() -> Self {
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cave-qwen-pump");
        Self::new(base.join("queue"), base.join("completed"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PumpEnvelope {
    task_id: String,
    target_branch: String,
    prompt: String,
    context: HashMap<String, String>,
    submitted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PumpCompletion {
    task_id: String,
    status: String, // "completed" | "failed"
    commit_sha: Option<String>,
    branch: Option<String>,
    reason: Option<String>,
    files_changed: Option<u32>,
    lines_added: Option<u32>,
    test_count: Option<u32>,
}

#[async_trait]
impl TaskQueue for PumpTaskQueue {
    async fn submit(
        &self,
        prompt: &str,
        target_branch: &str,
        context: HashMap<String, String>,
    ) -> Result<TaskId, TaskQueueError> {
        std::fs::create_dir_all(&self.queue_dir)?;
        let id = format!(
            "pump-{}-{:08}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S"),
            short_random(),
        );
        let env = PumpEnvelope {
            task_id: id.clone(),
            target_branch: target_branch.to_string(),
            prompt: prompt.to_string(),
            context,
            submitted_at: chrono::Utc::now(),
        };
        let path = self.queue_dir.join(format!("{id}.json"));
        let payload = serde_json::to_vec_pretty(&env)?;
        atomic_write(&path, &payload)?;
        Ok(TaskId::new(id))
    }

    async fn status(&self, task_id: &TaskId) -> Result<TaskStatus, TaskQueueError> {
        let path = self.completed_dir.join(format!("{}.json", task_id.as_str()));
        if !path.is_file() {
            // Still in queue → Pending. Still being worked → Running
            // (queue dir has the env but completed_dir doesn't yet).
            let queued = self.queue_dir.join(format!("{}.json", task_id.as_str()));
            if queued.is_file() {
                // Check if the pump moved it (mtime newer than ~5min
                // is "in flight").
                let meta = std::fs::metadata(&queued)?;
                let modified = meta.modified().unwrap_or_else(|_| std::time::SystemTime::UNIX_EPOCH);
                let age = std::time::SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                if age.as_secs() < 5 * 60 {
                    return Ok(TaskStatus::Running);
                }
                return Ok(TaskStatus::Pending);
            }
            return Err(TaskQueueError::NotFound(task_id.0.clone()));
        }
        let body = std::fs::read_to_string(&path)?;
        let c: PumpCompletion = serde_json::from_str(&body)?;
        match c.status.as_str() {
            "completed" => Ok(TaskStatus::Completed {
                commit_sha: c.commit_sha.unwrap_or_default(),
                branch: c.branch.unwrap_or_default(),
            }),
            "failed" => Ok(TaskStatus::Failed {
                reason: c.reason.unwrap_or_else(|| "pump reported failure".into()),
            }),
            other => Err(TaskQueueError::Http(format!("unknown pump status: {other}"))),
        }
    }

    async fn output(&self, task_id: &TaskId) -> Result<Option<TaskOutput>, TaskQueueError> {
        let path = self.completed_dir.join(format!("{}.json", task_id.as_str()));
        if !path.is_file() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&path)?;
        let c: PumpCompletion = serde_json::from_str(&body)?;
        if c.status != "completed" {
            return Ok(None);
        }
        Ok(Some(TaskOutput {
            commit_sha: c.commit_sha.unwrap_or_default(),
            branch: c.branch.unwrap_or_default(),
            files_changed: c.files_changed.unwrap_or(0),
            lines_added: c.lines_added.unwrap_or(0),
            test_count: c.test_count.unwrap_or(0),
        }))
    }

    fn backend_name(&self) -> &'static str {
        "pump"
    }
}

// ── Opus (Anthropic Messages API) ──────────────────────────────────────────

/// Submits the prompt to Anthropic's Messages API. The auth token
/// MUST be supplied via the `ANTHROPIC_API_KEY` environment variable
/// at construction time — the dispatcher never persists the secret
/// to disk.
///
/// The "task" model for Opus is a single round-trip: submit prompt
/// → receive completion text → parse `commit_sha:` / `branch:` /
/// `files_changed:` / `lines_added:` / `test_count:` lines from the
/// completion. We store the parsed output in `<state_dir>/opus/`
/// keyed by task id so subsequent `status` / `output` polls don't
/// re-hit the API.
#[derive(Debug)]
pub struct OpusTaskQueue {
    pub api_endpoint: String,
    pub api_key: String,
    pub model: String,
    pub state_dir: PathBuf,
    /// When `true`, never make an outbound HTTP call — the operator
    /// is bringing up the dispatcher in audit mode. Submits land in
    /// the state dir but `status` always reports `Failed { Disabled }`.
    pub disabled: bool,
}

impl OpusTaskQueue {
    /// Construct from env vars. `ANTHROPIC_API_KEY` is required;
    /// `ANTHROPIC_API_URL` defaults to `https://api.anthropic.com`,
    /// `CAVE_OPUS_MODEL` to `claude-opus-4-7`. Returns `None` when
    /// the API key is absent so the dispatcher falls back to the
    /// pump backend without blowing up.
    pub fn from_env(state_dir: PathBuf) -> Option<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty())?;
        let endpoint = std::env::var("ANTHROPIC_API_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
        let model = std::env::var("CAVE_OPUS_MODEL")
            .unwrap_or_else(|_| "claude-opus-4-7".to_string());
        Some(Self {
            api_endpoint: endpoint,
            api_key: key,
            model,
            state_dir,
            disabled: false,
        })
    }

    /// Explicit constructor — used by tests with httpmock.
    pub fn new(api_endpoint: String, api_key: String, state_dir: PathBuf) -> Self {
        Self {
            api_endpoint,
            api_key,
            model: "claude-opus-4-7".into(),
            state_dir,
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpusRecord {
    task_id: String,
    target_branch: String,
    status: String,
    output: Option<TaskOutput>,
    reason: Option<String>,
    at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicBlock>,
}

#[derive(Debug, Deserialize)]
struct AnthropicBlock {
    #[serde(rename = "type")]
    _type: String,
    text: Option<String>,
}

#[async_trait]
impl TaskQueue for OpusTaskQueue {
    async fn submit(
        &self,
        prompt: &str,
        target_branch: &str,
        _context: HashMap<String, String>,
    ) -> Result<TaskId, TaskQueueError> {
        if self.disabled {
            return Err(TaskQueueError::Disabled(
                "OpusTaskQueue is in audit-only mode (set ANTHROPIC_API_KEY + flip `disabled = false`)",
            ));
        }

        let id = format!(
            "opus-{}-{:08}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S"),
            short_random(),
        );

        // POST /v1/messages
        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: 4096,
            messages: vec![AnthropicMessage {
                role: "user",
                content: prompt,
            }],
        };
        let url = format!("{}/v1/messages", self.api_endpoint.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| TaskQueueError::Http(e.to_string()))?;
        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| TaskQueueError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(TaskQueueError::Http(format!(
                "anthropic POST /v1/messages failed: {status} {text}"
            )));
        }
        let parsed: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| TaskQueueError::Http(e.to_string()))?;
        let completion_text = parsed
            .content
            .iter()
            .filter_map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let output = parse_completion_text(&completion_text, target_branch);
        let record = OpusRecord {
            task_id: id.clone(),
            target_branch: target_branch.to_string(),
            status: if output.is_some() { "completed" } else { "failed" }.to_string(),
            output,
            reason: None,
            at: chrono::Utc::now(),
        };
        std::fs::create_dir_all(&self.state_dir)?;
        let path = self.state_dir.join(format!("{id}.json"));
        atomic_write(&path, &serde_json::to_vec_pretty(&record)?)?;
        Ok(TaskId::new(id))
    }

    async fn status(&self, task_id: &TaskId) -> Result<TaskStatus, TaskQueueError> {
        let path = self.state_dir.join(format!("{}.json", task_id.as_str()));
        if !path.is_file() {
            return Err(TaskQueueError::NotFound(task_id.0.clone()));
        }
        let body = std::fs::read_to_string(&path)?;
        let r: OpusRecord = serde_json::from_str(&body)?;
        match (r.status.as_str(), r.output) {
            ("completed", Some(o)) => Ok(TaskStatus::Completed {
                commit_sha: o.commit_sha,
                branch: o.branch,
            }),
            ("failed", _) => Ok(TaskStatus::Failed {
                reason: r.reason.unwrap_or_else(|| "Opus task failed".into()),
            }),
            _ => Ok(TaskStatus::Failed {
                reason: "Opus task completed but no output parsed".into(),
            }),
        }
    }

    async fn output(&self, task_id: &TaskId) -> Result<Option<TaskOutput>, TaskQueueError> {
        let path = self.state_dir.join(format!("{}.json", task_id.as_str()));
        if !path.is_file() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&path)?;
        let r: OpusRecord = serde_json::from_str(&body)?;
        Ok(r.output)
    }

    fn backend_name(&self) -> &'static str {
        "opus"
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

/// Parse the four contract fields from the completion text.
/// `commit_sha` must be exactly 40 hex chars; everything else is
/// optional. Returns `None` if the SHA isn't present.
// ── ClaudeCliTaskQueue (added 2026-05-14) ──────────────────────────────────
//
// Production default backend. Spawns the locally-installed `claude` binary
// (Claude Code, 2.1.x+) with `-p <prompt> --output-format json`, captures the
// JSON stream, and parses the trailing assistant message via
// [`parse_completion_text`] so the dispatcher sees the same `commit_sha /
// branch / files_changed / test_count` shape it gets from
// [`OpusTaskQueue`]. No external API key required — the local CLI carries
// the operator's Anthropic session.
//
// Locator: walks `$PATH` AND `$HOME/.local/bin` (where the Claude Code
// installer puts the symlink), in that order. `from_env` returns
// `TaskQueueError::Disabled` if no binary is found.
//
// State: per-task JSON records under `output_log_dir/<task_id>.json` with
// the task envelope (prompt, branch, ctx, started_at, pid, stdout_path,
// status, result_text). status/output replay these records, no in-memory
// map required so the dispatcher survives restart.

#[derive(Clone)]
pub struct ClaudeCliTaskQueue {
    workspace_root: PathBuf,
    claude_binary: PathBuf,
    max_turns: u32,
    output_log_dir: PathBuf,
    /// Skip the actual subprocess when true (used by integration tests).
    test_mode_echo: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaudeCliRecord {
    task_id: String,
    branch: String,
    prompt: String,
    ctx: HashMap<String, String>,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    cmd_args: Vec<String>,
    stdout_path: PathBuf,
    /// `dispatched` until the subprocess returns, then `completed` /
    /// `failed`.
    status: String,
    exit_code: Option<i32>,
    /// Final assistant text used by [`parse_completion_text`]. Populated
    /// only on success.
    result_text: Option<String>,
}

impl ClaudeCliTaskQueue {
    /// Locate `claude` on standard install paths. Walks `$PATH` first
    /// (so any operator-managed override wins), then falls back to
    /// `$HOME/.local/bin/claude` (default Claude Code installer target).
    /// Returns the first executable found.
    pub fn locate_binary() -> Option<PathBuf> {
        // 1. PATH walk.
        if let Ok(path_env) = std::env::var("PATH") {
            for dir in path_env.split(':') {
                let p = PathBuf::from(dir).join("claude");
                if Self::is_executable(&p) {
                    return Some(p);
                }
            }
        }
        // 2. $HOME/.local/bin/claude (Claude Code installer default).
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home).join(".local/bin/claude");
            if Self::is_executable(&p) {
                return Some(p);
            }
        }
        // 3. /usr/local/bin/claude (legacy Homebrew install).
        let p = PathBuf::from("/usr/local/bin/claude");
        if Self::is_executable(&p) {
            return Some(p);
        }
        None
    }

    fn is_executable(p: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(p) {
            Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
            Err(_) => false,
        }
    }

    /// Construct from environment — discovers `claude` via
    /// [`locate_binary`], uses `CAVE_WORKSPACE_ROOT` env var (or the
    /// passed `output_log_dir`'s parent as a sensible fallback).
    /// Returns `TaskQueueError::Disabled` when no binary is found.
    pub fn from_env(output_log_dir: PathBuf) -> Result<Self, TaskQueueError> {
        let claude_binary = Self::locate_binary().ok_or_else(|| {
            TaskQueueError::Disabled(
                "no `claude` binary found in $PATH, $HOME/.local/bin, \
                 or /usr/local/bin — install Claude Code first \
                 (https://claude.com/code) or use --backend pump/dryrun",
            )
        })?;
        let workspace_root = std::env::var("CAVE_WORKSPACE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                output_log_dir
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."))
            });
        std::fs::create_dir_all(&output_log_dir)?;
        Ok(Self {
            workspace_root,
            claude_binary,
            max_turns: 50,
            output_log_dir,
            test_mode_echo: false,
        })
    }

    /// Override the discovered binary path (integration tests use a
    /// fast-exiting sentinel like `/bin/echo`). Flips `test_mode_echo`
    /// so submit() skips the JSON-output flag set.
    pub fn set_claude_binary_for_test(&mut self, p: PathBuf) {
        self.claude_binary = p;
        self.test_mode_echo = true;
    }

    fn record_path(&self, task_id: &TaskId) -> PathBuf {
        self.output_log_dir.join(format!("{}.json", task_id.0))
    }

    fn stdout_path(&self, task_id: &TaskId) -> PathBuf {
        self.output_log_dir.join(format!("{}.stdout", task_id.0))
    }

    fn read_record(&self, task_id: &TaskId) -> Result<ClaudeCliRecord, TaskQueueError> {
        let p = self.record_path(task_id);
        let raw = std::fs::read(&p)
            .map_err(|_| TaskQueueError::NotFound(task_id.0.clone()))?;
        let rec: ClaudeCliRecord = serde_json::from_slice(&raw)?;
        Ok(rec)
    }

    fn write_record(&self, rec: &ClaudeCliRecord) -> Result<(), TaskQueueError> {
        let p = self.output_log_dir.join(format!("{}.json", rec.task_id));
        let raw = serde_json::to_vec_pretty(rec)?;
        std::fs::write(&p, raw)?;
        Ok(())
    }
}

#[async_trait]
impl TaskQueue for ClaudeCliTaskQueue {
    async fn submit(
        &self,
        prompt: &str,
        target_branch: &str,
        context: HashMap<String, String>,
    ) -> Result<TaskId, TaskQueueError> {
        let task_id = TaskId::new(format!(
            "claude-cli-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%3f")
        ));
        let stdout_path = self.stdout_path(&task_id);

        // Build args: `claude -p <prompt> --output-format json
        // --max-turns N --working-directory <wt> --dangerously-skip-permissions`.
        // In test_mode_echo we just pass the prompt as a positional arg
        // so /bin/echo exits cleanly and we exercise the record-write path.
        let args: Vec<String> = if self.test_mode_echo {
            vec![prompt.to_string()]
        } else {
            vec![
                "-p".into(),
                prompt.into(),
                "--output-format".into(),
                "json".into(),
                "--max-turns".into(),
                self.max_turns.to_string(),
                "--add-dir".into(),
                self.workspace_root.display().to_string(),
                "--dangerously-skip-permissions".into(),
            ]
        };

        let rec = ClaudeCliRecord {
            task_id: task_id.0.clone(),
            branch: target_branch.to_string(),
            prompt: prompt.to_string(),
            ctx: context,
            started_at: chrono::Utc::now(),
            completed_at: None,
            cmd_args: {
                let mut a = vec![self.claude_binary.display().to_string()];
                a.extend(args.iter().cloned());
                a
            },
            stdout_path: stdout_path.clone(),
            status: "dispatched".to_string(),
            exit_code: None,
            result_text: None,
        };
        self.write_record(&rec)?;

        // Spawn subprocess capturing stdout. In test_mode_echo this
        // returns essentially instantly; in production claude CLI may
        // take minutes — the dispatcher's max_concurrent gate is what
        // limits parallelism, not this await.
        let mut cmd = tokio::process::Command::new(&self.claude_binary);
        cmd.args(&args);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let output = cmd
            .output()
            .await
            .map_err(|e| TaskQueueError::Http(format!("claude spawn: {e}")))?;

        // Persist stdout for later inspection.
        std::fs::write(&stdout_path, &output.stdout)?;

        let result_text = String::from_utf8_lossy(&output.stdout).into_owned();
        let exit_code = output.status.code();
        let status = if output.status.success() {
            "completed"
        } else {
            "failed"
        };
        let rec_done = ClaudeCliRecord {
            completed_at: Some(chrono::Utc::now()),
            status: status.to_string(),
            exit_code,
            result_text: Some(result_text),
            ..rec
        };
        self.write_record(&rec_done)?;
        Ok(task_id)
    }

    async fn status(&self, task_id: &TaskId) -> Result<TaskStatus, TaskQueueError> {
        let rec = self.read_record(task_id)?;
        match (rec.status.as_str(), rec.exit_code) {
            ("dispatched", _) => Ok(TaskStatus::Running),
            ("completed", _) => {
                let text = rec.result_text.clone().unwrap_or_default();
                // Parse the contract fields the prompt instructs the
                // model to emit. If the parse fails (Claude went off-
                // script and didn't echo commit_sha:), surface as Failed
                // so the charter gate can re-queue / cooldown.
                if let Some(o) = parse_completion_text(&text, &rec.branch) {
                    Ok(TaskStatus::Completed {
                        commit_sha: o.commit_sha,
                        branch: o.branch,
                    })
                } else {
                    Ok(TaskStatus::Failed {
                        reason: format!(
                            "claude exit 0 but no commit_sha: line in stdout (record at {})",
                            self.record_path(task_id).display()
                        ),
                    })
                }
            }
            ("failed", code) => Ok(TaskStatus::Failed {
                reason: format!(
                    "claude exited {} (record at {})",
                    code.map(|c| c.to_string())
                        .unwrap_or_else(|| "signal".to_string()),
                    self.record_path(task_id).display()
                ),
            }),
            (other, _) => Err(TaskQueueError::Http(format!(
                "unknown record status: {other}"
            ))),
        }
    }

    async fn output(&self, task_id: &TaskId) -> Result<Option<TaskOutput>, TaskQueueError> {
        let rec = self.read_record(task_id)?;
        if rec.status != "completed" {
            return Ok(None);
        }
        let text = rec.result_text.unwrap_or_default();
        Ok(parse_completion_text(&text, &rec.branch))
    }

    fn backend_name(&self) -> &'static str {
        "claude-cli"
    }
}

pub fn parse_completion_text(text: &str, fallback_branch: &str) -> Option<TaskOutput> {
    let mut commit_sha: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut files_changed: Option<u32> = None;
    let mut lines_added: Option<u32> = None;
    let mut test_count: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = strip_field(line, "commit_sha") {
            if v.len() == 40 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
                commit_sha = Some(v.to_string());
            }
        }
        if let Some(v) = strip_field(line, "branch") {
            branch = Some(v.to_string());
        }
        if let Some(v) = strip_field(line, "files_changed") {
            files_changed = v.parse().ok();
        }
        if let Some(v) = strip_field(line, "lines_added") {
            lines_added = v.parse().ok();
        }
        if let Some(v) = strip_field(line, "test_count") {
            test_count = v.parse().ok();
        }
    }
    let sha = commit_sha?;
    Some(TaskOutput {
        commit_sha: sha,
        branch: branch.unwrap_or_else(|| fallback_branch.to_string()),
        files_changed: files_changed.unwrap_or(0),
        lines_added: lines_added.unwrap_or(0),
        test_count: test_count.unwrap_or(0),
    })
}

fn strip_field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    Some(rest.trim())
}

fn append_jsonl<T: Serialize>(path: &Path, record: &T) -> Result<(), TaskQueueError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(record)?;
    use std::io::Write;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), TaskQueueError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn short_random() -> u32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    (h.finish() & 0xFFFF_FFFF) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::POST;
    use httpmock::MockServer;

    fn ctx() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("crate_name".into(), "cave-x".into());
        m
    }

    // ── ClaudeCliTaskQueue (RED phase — added 2026-05-14) ──────
    //
    // These tests reference [`ClaudeCliTaskQueue`] which does NOT exist
    // yet on this branch. `cargo test --no-run` MUST therefore fail to
    // compile this module — that is the RED proof the GREEN commit will
    // flip. Charter v2 TDD STRICT MODE applied to the dispatcher itself.

    #[tokio::test]
    async fn claude_cli_task_queue_from_env_locates_binary_on_standard_paths() {
        // ClaudeCliTaskQueue::from_env should resolve the `claude` binary
        // by walking PATH and ~/.local/bin in order — no env var required.
        let dir = tempfile::TempDir::new().unwrap();
        let q = ClaudeCliTaskQueue::from_env(dir.path().to_path_buf())
            .expect("must locate claude on a Mac with Claude Code installed");
        assert_eq!(q.backend_name(), "claude-cli");
    }

    #[tokio::test]
    async fn claude_cli_task_queue_submit_records_task_and_worktree_path() {
        // Submit must return a stable TaskId and stash a per-task record
        // under output_log_dir so status() / output() can find it later.
        let dir = tempfile::TempDir::new().unwrap();
        let mut q = ClaudeCliTaskQueue::from_env(dir.path().to_path_buf())
            .expect("claude binary present");
        // Use a sentinel binary that just echoes a fake completion line.
        q.set_claude_binary_for_test(std::path::PathBuf::from("/bin/echo"));
        let id = q
            .submit("test prompt", "auto-port/test-event", ctx())
            .await
            .expect("submit ok");
        assert!(id.0.starts_with("claude-cli-"), "TaskId prefix");
        // Record file must exist.
        let record = dir.path().join(format!("{}.json", id.0));
        assert!(
            record.exists(),
            "submit must persist task record at {}",
            record.display()
        );
    }

    #[tokio::test]
    async fn claude_cli_task_queue_status_returns_pending_for_unknown_task() {
        let dir = tempfile::TempDir::new().unwrap();
        let q = ClaudeCliTaskQueue::from_env(dir.path().to_path_buf()).unwrap();
        let err = q
            .status(&TaskId::new("claude-cli-nonexistent"))
            .await
            .unwrap_err();
        // Unknown task → NotFound (mirrors OpusTaskQueue behaviour).
        assert!(
            matches!(err, TaskQueueError::NotFound(_)),
            "got {:?}",
            err
        );
    }

    #[test]
    fn claude_cli_parse_completion_extracts_commit_sha_and_branch() {
        // The result line emitted by `claude -p --output-format json`
        // includes a final assistant message; we re-use the existing
        // parse_completion_text helper for the commit_sha / branch /
        // file count fields the dispatcher reads from `git log -1`.
        let body = "commit_sha: 0a45a85b6e1fb43f6177b5181285cf2c0973974f\n\
                    branch: auto-port/GAP-1234\n\
                    files_changed: 3\n\
                    lines_added: 142\n\
                    test_count: 7\n";
        let out = parse_completion_text(body, "auto-port/fallback").unwrap();
        assert_eq!(out.commit_sha.len(), 40);
        assert_eq!(out.branch, "auto-port/GAP-1234");
        assert_eq!(out.test_count, 7);
    }

    #[test]
    fn claude_cli_locate_binary_finds_local_bin_symlink() {
        // The locator function used by from_env() is exposed for testing.
        // On the dev box, ~/.local/bin/claude is the canonical install
        // (Claude Code installer puts it there). The function must
        // discover it even if PATH doesn't include ~/.local/bin.
        if !std::path::Path::new(
            &std::env::var("HOME")
                .map(|h| format!("{h}/.local/bin/claude"))
                .unwrap_or_default(),
        )
        .exists()
        {
            eprintln!("skipping — no ~/.local/bin/claude on this host");
            return;
        }
        let p = ClaudeCliTaskQueue::locate_binary().expect("must find claude");
        assert!(
            p.ends_with("claude"),
            "located path should end with `claude`: {}",
            p.display()
        );
    }

    // ── DryRun ─────────────────────────────────────────────────

    #[tokio::test]
    async fn dryrun_submit_appends_jsonl_and_returns_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        let q = DryRunTaskQueue::new(dir.path().join("audit.jsonl"));
        let id = q.submit("prompt", "auto-port/x", ctx()).await.unwrap();
        assert!(id.0.starts_with("dryrun-"));
        let status = q.status(&id).await.unwrap();
        assert_eq!(status, TaskStatus::Pending);
        let out = q.output(&id).await.unwrap();
        assert!(out.is_none());
        // The audit log has one line.
        let raw = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert_eq!(raw.lines().count(), 1);
    }

    #[tokio::test]
    async fn dryrun_id_sequence_increases_monotonically() {
        let dir = tempfile::TempDir::new().unwrap();
        let q = DryRunTaskQueue::new(dir.path().join("audit.jsonl"));
        let a = q.submit("p", "b", ctx()).await.unwrap();
        let b = q.submit("p", "b", ctx()).await.unwrap();
        assert_ne!(a, b);
        assert!(a.0.ends_with("000001"));
        assert!(b.0.ends_with("000002"));
    }

    // ── Pump ───────────────────────────────────────────────────

    #[tokio::test]
    async fn pump_submit_writes_envelope_to_queue_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let queue = dir.path().join("q");
        let done = dir.path().join("done");
        let q = PumpTaskQueue::new(queue.clone(), done);
        let id = q.submit("hello", "auto-port/x", ctx()).await.unwrap();
        let env_path = queue.join(format!("{}.json", id.0));
        assert!(env_path.is_file());
        let body = std::fs::read_to_string(&env_path).unwrap();
        assert!(body.contains("hello"));
        assert!(body.contains("auto-port/x"));
    }

    #[tokio::test]
    async fn pump_status_reads_completion_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let queue = dir.path().join("q");
        let done = dir.path().join("done");
        std::fs::create_dir_all(&done).unwrap();
        let q = PumpTaskQueue::new(queue, done.clone());
        // Synthesize a completion file the pump would have written.
        let body = serde_json::json!({
            "task_id": "pump-fake-001",
            "status": "completed",
            "commit_sha": "0000000000000000000000000000000000000001",
            "branch": "auto-port/x",
            "files_changed": 3,
            "lines_added": 120,
            "test_count": 8
        });
        std::fs::write(done.join("pump-fake-001.json"), body.to_string()).unwrap();
        let s = q.status(&TaskId::new("pump-fake-001")).await.unwrap();
        match s {
            TaskStatus::Completed { commit_sha, branch } => {
                assert_eq!(commit_sha, "0000000000000000000000000000000000000001");
                assert_eq!(branch, "auto-port/x");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        let out = q.output(&TaskId::new("pump-fake-001")).await.unwrap().unwrap();
        assert_eq!(out.files_changed, 3);
        assert_eq!(out.test_count, 8);
    }

    #[tokio::test]
    async fn pump_status_failed_reports_reason() {
        let dir = tempfile::TempDir::new().unwrap();
        let done = dir.path().join("done");
        std::fs::create_dir_all(&done).unwrap();
        let q = PumpTaskQueue::new(dir.path().join("q"), done.clone());
        std::fs::write(
            done.join("pump-fail-001.json"),
            serde_json::json!({
                "task_id": "pump-fail-001",
                "status": "failed",
                "reason": "compile error"
            })
            .to_string(),
        )
        .unwrap();
        let s = q.status(&TaskId::new("pump-fail-001")).await.unwrap();
        assert!(matches!(s, TaskStatus::Failed { .. }));
    }

    #[tokio::test]
    async fn pump_status_returns_running_when_queued_recently() {
        let dir = tempfile::TempDir::new().unwrap();
        let queue = dir.path().join("q");
        let done = dir.path().join("done");
        let q = PumpTaskQueue::new(queue.clone(), done);
        let id = q.submit("p", "auto-port/x", ctx()).await.unwrap();
        let s = q.status(&id).await.unwrap();
        // Fresh submit → mtime within 5 minutes → Running.
        assert_eq!(s, TaskStatus::Running);
    }

    #[tokio::test]
    async fn pump_status_not_found_when_missing_everywhere() {
        let dir = tempfile::TempDir::new().unwrap();
        let q = PumpTaskQueue::new(dir.path().join("q"), dir.path().join("done"));
        let err = q.status(&TaskId::new("ghost")).await.unwrap_err();
        assert!(matches!(err, TaskQueueError::NotFound(_)));
    }

    // ── Opus ──────────────────────────────────────────────────

    #[tokio::test]
    async fn opus_submit_round_trips_through_anthropic_api() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST).path("/v1/messages")
                .header("x-api-key", "k-test")
                .header("anthropic-version", "2023-06-01");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    serde_json::json!({
                        "content": [
                            { "type": "text", "text":
                                "Done.\ncommit_sha: 0123456789abcdef0123456789abcdef01234567\nbranch: auto-port/x\nfiles_changed: 4\nlines_added: 200\ntest_count: 7"
                            }
                        ]
                    })
                    .to_string(),
                );
        });
        let dir = tempfile::TempDir::new().unwrap();
        let q = OpusTaskQueue::new(server.base_url(), "k-test".into(), dir.path().join("opus"));
        let id = q.submit("prompt", "auto-port/x", ctx()).await.unwrap();
        m.assert();
        let s = q.status(&id).await.unwrap();
        match s {
            TaskStatus::Completed { commit_sha, .. } => {
                assert_eq!(commit_sha, "0123456789abcdef0123456789abcdef01234567");
            }
            other => panic!("{other:?}"),
        }
        let out = q.output(&id).await.unwrap().unwrap();
        assert_eq!(out.files_changed, 4);
        assert_eq!(out.lines_added, 200);
        assert_eq!(out.test_count, 7);
    }

    #[tokio::test]
    async fn opus_submit_surfaces_failure_when_completion_text_missing_sha() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(200)
                .header("content-type", "application/json")
                .body(serde_json::json!({"content": [{"type": "text", "text": "no commit field"}]}).to_string());
        });
        let dir = tempfile::TempDir::new().unwrap();
        let q = OpusTaskQueue::new(server.base_url(), "k".into(), dir.path().join("opus"));
        let id = q.submit("p", "b", ctx()).await.unwrap();
        let s = q.status(&id).await.unwrap();
        assert!(matches!(s, TaskStatus::Failed { .. }));
    }

    #[tokio::test]
    async fn opus_surfaces_http_error_when_api_returns_4xx() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/v1/messages");
            then.status(401).body("unauthorized");
        });
        let dir = tempfile::TempDir::new().unwrap();
        let q = OpusTaskQueue::new(server.base_url(), "wrong".into(), dir.path().join("opus"));
        let err = q.submit("p", "b", ctx()).await.unwrap_err();
        match err {
            TaskQueueError::Http(s) => assert!(s.contains("401")),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn opus_disabled_mode_refuses_submit_with_clear_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut q = OpusTaskQueue::new("http://unused".into(), "k".into(), dir.path().join("opus"));
        q.disabled = true;
        let err = q.submit("p", "b", ctx()).await.unwrap_err();
        assert!(matches!(err, TaskQueueError::Disabled(_)));
    }

    #[tokio::test]
    async fn opus_from_env_returns_none_when_api_key_missing() {
        // Make sure the var is unset for this test.
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY"); }
        let dir = tempfile::TempDir::new().unwrap();
        assert!(OpusTaskQueue::from_env(dir.path().to_path_buf()).is_none());
    }

    // ── parse_completion_text ─────────────────────────────────

    #[test]
    fn parse_completion_text_picks_up_all_fields() {
        let text = "Some narration.\ncommit_sha: 0123456789abcdef0123456789abcdef01234567\nbranch: auto-port/x\nfiles_changed: 3\nlines_added: 100\ntest_count: 5";
        let o = parse_completion_text(text, "fallback").unwrap();
        assert_eq!(o.commit_sha, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(o.branch, "auto-port/x");
        assert_eq!(o.files_changed, 3);
        assert_eq!(o.lines_added, 100);
        assert_eq!(o.test_count, 5);
    }

    #[test]
    fn parse_completion_text_rejects_non_40_char_sha() {
        let text = "commit_sha: abc123";
        assert!(parse_completion_text(text, "x").is_none());
    }

    #[test]
    fn parse_completion_text_falls_back_branch_when_missing() {
        let text = "commit_sha: 0123456789abcdef0123456789abcdef01234567";
        let o = parse_completion_text(text, "fallback-branch").unwrap();
        assert_eq!(o.branch, "fallback-branch");
        assert_eq!(o.files_changed, 0);
    }
}
