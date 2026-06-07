// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! YAML configuration for the daily upstream tracker.
//!
//! The default config embeds the canonical [`default_registry`] so a
//! fresh checkout polls a useful set with no config file. Operators
//! override fields — most usefully the per-upstream `pinned` baselines —
//! by writing `cave-runtime-tracker.yaml` (anywhere; pointed at with
//! `--config`). The reference copy lives at the repo root.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{TrackerError, TrackerResult};
use crate::registry::{default_registry, Upstream};

/// Top-level config consumed by the CLI and the daily LaunchAgent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackerConfig {
    /// Directory where `daily-<date>.{md,json}` reports land.
    pub output_dir: String,
    /// GitHub REST API base. Overridable for GitHub Enterprise or tests.
    pub github_api: String,
    /// Per-request HTTP timeout in seconds.
    pub request_timeout_secs: u64,
    /// Also write a `latest.json` copy next to the dated report.
    pub emit_latest: bool,
    /// The upstreams to poll.
    pub upstreams: Vec<Upstream>,
}

impl TrackerConfig {
    /// Default config: canonical registry, public GitHub, reports under
    /// `~/Library/Application Support/cave-runtime/runtime-tracker`.
    pub fn default_config() -> Self {
        Self {
            output_dir: "~/Library/Application Support/cave-runtime/runtime-tracker".to_string(),
            github_api: "https://api.github.com".to_string(),
            request_timeout_secs: 20,
            emit_latest: true,
            upstreams: default_registry(),
        }
    }

    pub fn load(path: &Path) -> TrackerResult<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: TrackerConfig = serde_yaml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn to_yaml(&self) -> TrackerResult<String> {
        serde_yaml::to_string(self).map_err(TrackerError::from)
    }

    pub fn validate(&self) -> TrackerResult<()> {
        if self.upstreams.is_empty() {
            return Err(TrackerError::Config("upstreams must not be empty".to_string()));
        }
        if self.request_timeout_secs == 0 {
            return Err(TrackerError::Config(
                "request_timeout_secs must be >= 1".to_string(),
            ));
        }
        if !self.github_api.starts_with("http") {
            return Err(TrackerError::Config(format!(
                "github_api must be an http(s) URL, got {:?}",
                self.github_api
            )));
        }
        for u in &self.upstreams {
            if !u.repo.contains('/') {
                return Err(TrackerError::Config(format!(
                    "upstream {:?} repo must be org/repo, got {:?}",
                    u.name, u.repo
                )));
            }
        }
        Ok(())
    }

    /// Distinct `org/repo` slugs across all upstreams (many cave modules
    /// share a single upstream, e.g. four crates track
    /// `kubernetes/kubernetes`). The poller fetches each repo once.
    pub fn distinct_repos(&self) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        self.upstreams
            .iter()
            .filter(|u| seen.insert(u.repo.clone()))
            .map(|u| u.repo.clone())
            .collect()
    }

    /// Expand a leading `~/` in `output_dir` using `$HOME`.
    pub fn expanded_output_dir(&self) -> String {
        Self::expand_tilde(&self.output_dir, std::env::var("HOME").ok().as_deref())
    }

    /// Pure-function variant of [`expanded_output_dir`] — explicit
    /// `$HOME` keeps it unit-testable without mutating the process env.
    pub fn expand_tilde(raw: &str, home: Option<&str>) -> String {
        if let Some(rest) = raw.strip_prefix("~/")
            && let Some(home) = home
        {
            return format!("{}/{}", home.trim_end_matches('/'), rest);
        }
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_full_registry_and_public_github() {
        let c = TrackerConfig::default_config();
        assert!(c.upstreams.len() >= 70);
        assert_eq!(c.github_api, "https://api.github.com");
        assert!(c.emit_latest);
    }

    #[test]
    fn default_config_validates() {
        TrackerConfig::default_config().validate().expect("default valid");
    }

    #[test]
    fn empty_upstreams_fail_validate() {
        let mut c = TrackerConfig::default_config();
        c.upstreams.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn zero_timeout_fails_validate() {
        let mut c = TrackerConfig::default_config();
        c.request_timeout_secs = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn non_http_github_api_fails_validate() {
        let mut c = TrackerConfig::default_config();
        c.github_api = "ftp://nope".to_string();
        assert!(c.validate().is_err());
    }

    #[test]
    fn distinct_repos_dedupes_shared_upstreams() {
        let c = TrackerConfig::default_config();
        let distinct = c.distinct_repos();
        // kubernetes/kubernetes is tracked by several crates but must
        // appear exactly once in the distinct set.
        let k8s = distinct.iter().filter(|r| *r == "kubernetes/kubernetes").count();
        assert_eq!(k8s, 1);
        assert!(distinct.len() < c.upstreams.len());
    }

    #[test]
    fn config_roundtrips_through_yaml() {
        let c = TrackerConfig::default_config();
        let s = c.to_yaml().unwrap();
        let back: TrackerConfig = serde_yaml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn load_reads_yaml_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.yaml");
        std::fs::write(&p, TrackerConfig::default_config().to_yaml().unwrap()).unwrap();
        let loaded = TrackerConfig::load(&p).unwrap();
        assert_eq!(loaded, TrackerConfig::default_config());
    }

    #[test]
    fn expand_tilde_uses_supplied_home() {
        assert_eq!(
            TrackerConfig::expand_tilde("~/cave-tmp", Some("/tmp/fakehome")),
            "/tmp/fakehome/cave-tmp"
        );
        assert_eq!(
            TrackerConfig::expand_tilde("/abs", Some("/tmp/home")),
            "/abs"
        );
        assert_eq!(TrackerConfig::expand_tilde("~/x", None), "~/x");
    }
}
