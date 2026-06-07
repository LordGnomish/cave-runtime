// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Daemon configuration.
//!
//! One [`AutopilotConfig`] drives a single instance. Two instances run in
//! parallel (cave-runtime on :9101, cave-home on :9102), each pointed at a
//! different repo + tracker output. Config loads from a TOML file with
//! sensible defaults; the Claude token budget can be overridden by the
//! `CAVE_AUTOPILOT_CLAUDE_TOKEN_BUDGET` env var so a LaunchAgent can cap cost
//! without editing files.

use crate::error::{AutopilotError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Full configuration for one autopilot instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutopilotConfig {
    /// Instance name — `"cave-runtime"` or `"cave-home"`. Drives the plist
    /// label and the default metrics port.
    pub instance: String,
    /// Absolute path to the git repository the daemon operates on.
    pub repo_root: PathBuf,
    /// Path to the tracker state file (parity-index.json) this instance reads.
    pub tracker_state_path: PathBuf,
    /// Directory under which per-task worktrees are created.
    pub worktree_root: PathBuf,
    /// Directory daily reports are written to.
    pub report_dir: PathBuf,
    /// Prometheus + healthz bind port.
    pub metrics_port: u16,
    /// Ollama HTTP base URL.
    pub ollama_url: String,
    /// L1 routing model (analysis / context sizing).
    pub model_l1_router: String,
    /// L2 local code-generation model.
    pub model_l2_coder: String,
    /// Resident fallback model used when the named L1/L2 models are not pulled.
    pub model_fallback: String,
    /// Claude API model for L3 escalation.
    pub claude_model: String,
    /// Daily Claude output-token budget. Once exceeded the daemon drops to
    /// local-LLM-only mode for the rest of the day.
    pub claude_daily_token_budget: u64,
    /// Subsystems whose completion ratio is **below** this are eligible for the
    /// work queue.
    pub completion_threshold: f64,
    /// When **every** subsystem is at or above this, the daemon enters idle
    /// (monitor-only) mode.
    pub idle_threshold: f64,
    /// Hard stop: if free disk drops below this many GiB, halt and notify.
    pub min_free_disk_gb: u64,
    /// Max local-LLM retries on a single task before escalating to Claude.
    pub max_local_retries: u32,
    /// Seconds between scheduler ticks in the daemon loop.
    pub tick_interval_secs: u64,
}

impl Default for AutopilotConfig {
    fn default() -> Self {
        Self::for_instance("cave-runtime")
    }
}

impl AutopilotConfig {
    /// Build a default config for a named instance, picking the conventional
    /// port and repo path for the two known instances.
    pub fn for_instance(instance: &str) -> Self {
        let (port, repo, tracker) = match instance {
            "cave-home" => (
                9102u16,
                "/Users/gnomish/Code/cave-home",
                "/Users/gnomish/Code/cave-home/docs/parity/parity-index.json",
            ),
            // default / cave-runtime
            _ => (
                9101u16,
                "/Users/gnomish/Code/cave-runtime",
                "/Users/gnomish/Code/cave-runtime/docs/parity/parity-index.json",
            ),
        };
        Self {
            instance: instance.to_string(),
            repo_root: PathBuf::from(repo),
            tracker_state_path: PathBuf::from(tracker),
            worktree_root: PathBuf::from(format!("{repo}/.autopilot/worktrees")),
            report_dir: PathBuf::from(format!("{repo}/docs/audit")),
            metrics_port: port,
            ollama_url: "http://localhost:11434".to_string(),
            model_l1_router: "mellum2:12b-moe".to_string(),
            model_l2_coder: "qwen3-coder-next:80b-moe".to_string(),
            model_fallback: "qwen3.6:35b-a3b-coding-mxfp8".to_string(),
            claude_model: "claude-opus-4-7".to_string(),
            claude_daily_token_budget: 2_000_000,
            completion_threshold: 0.95,
            idle_threshold: 0.95,
            min_free_disk_gb: 5,
            max_local_retries: 5,
            tick_interval_secs: 300,
        }
    }

    /// Load config from a TOML file, then apply env overrides.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let mut cfg: AutopilotConfig = toml::from_str(&raw)?;
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize to a TOML string (used by the `init-config` sub-command).
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Override mutable knobs from the environment. Currently only the Claude
    /// token budget, so a LaunchAgent can cap cost without a file edit.
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("CAVE_AUTOPILOT_CLAUDE_TOKEN_BUDGET") {
            if let Ok(n) = v.parse::<u64>() {
                self.claude_daily_token_budget = n;
            }
        }
    }

    /// Reject nonsensical configs early.
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.completion_threshold) {
            return Err(AutopilotError::Config(format!(
                "completion_threshold must be in [0,1], got {}",
                self.completion_threshold
            )));
        }
        if !(0.0..=1.0).contains(&self.idle_threshold) {
            return Err(AutopilotError::Config(format!(
                "idle_threshold must be in [0,1], got {}",
                self.idle_threshold
            )));
        }
        if self.instance.is_empty() {
            return Err(AutopilotError::Config("instance must not be empty".into()));
        }
        if self.metrics_port == 0 {
            return Err(AutopilotError::Config("metrics_port must be non-zero".into()));
        }
        Ok(())
    }

    /// LaunchAgent label for this instance, e.g.
    /// `com.gnomish.cave-runtime-autopilot`.
    pub fn launch_label(&self) -> String {
        format!("com.gnomish.{}-autopilot", self.instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_cave_runtime_on_9101() {
        let c = AutopilotConfig::default();
        assert_eq!(c.instance, "cave-runtime");
        assert_eq!(c.metrics_port, 9101);
        assert!(c.tracker_state_path.ends_with("parity-index.json"));
    }

    #[test]
    fn cave_home_instance_gets_9102_and_own_repo() {
        let c = AutopilotConfig::for_instance("cave-home");
        assert_eq!(c.metrics_port, 9102);
        assert!(c.repo_root.to_string_lossy().contains("cave-home"));
        assert!(c.tracker_state_path.to_string_lossy().contains("cave-home"));
    }

    #[test]
    fn launch_label_is_namespaced_per_instance() {
        assert_eq!(
            AutopilotConfig::for_instance("cave-runtime").launch_label(),
            "com.gnomish.cave-runtime-autopilot"
        );
        assert_eq!(
            AutopilotConfig::for_instance("cave-home").launch_label(),
            "com.gnomish.cave-home-autopilot"
        );
    }

    #[test]
    fn toml_round_trips() {
        let c = AutopilotConfig::default();
        let s = c.to_toml().unwrap();
        let back: AutopilotConfig = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn load_applies_env_token_budget_override() {
        // Safe: serial within this test, restored after.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.toml");
        std::fs::write(&p, AutopilotConfig::default().to_toml().unwrap()).unwrap();
        unsafe {
            std::env::set_var("CAVE_AUTOPILOT_CLAUDE_TOKEN_BUDGET", "12345");
        }
        let c = AutopilotConfig::load(&p).unwrap();
        assert_eq!(c.claude_daily_token_budget, 12345);
        unsafe {
            std::env::remove_var("CAVE_AUTOPILOT_CLAUDE_TOKEN_BUDGET");
        }
    }

    #[test]
    fn validate_rejects_out_of_range_threshold() {
        let mut c = AutopilotConfig::default();
        c.completion_threshold = 1.5;
        assert!(c.validate().is_err());
    }
}
