//! Persistent watch-daemon state.
//!
//! State is a single JSON file at
//! `~/Library/Application Support/cave-runtime/upstream-state.json` (macOS;
//! `$XDG_DATA_HOME/cave-runtime/...` on other platforms via `dirs::data_dir`).
//!
//! The schema is one [`ProjectState`] entry per `github_repo` we track, keyed
//! by repo string. The whole file is rewritten atomically on every save (via
//! tempfile+rename) so a crash mid-write cannot leave a half-written file.
//!
//! ## Why not sled?
//!
//! sled is multi-file and has its own recovery semantics; for a daemon that
//! polls 66 repos every 15 minutes the entire state is small (a few hundred
//! KB at most) and the simplicity of "load whole file → mutate → atomic
//! rename" makes operational debugging much easier. Operators can `cat` the
//! file, hand-edit it to force a re-poll, etc.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Per-repo persistent state.
///
/// `github_repo` is the canonical key (`owner/name`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectState {
    /// Canonical key, e.g. `"etcd-io/etcd"`.
    pub github_repo: String,
    /// Last time we successfully completed a poll for this repo.
    pub last_checked: Option<DateTime<Utc>>,
    /// Tag string we last observed as the latest release.
    pub last_known_tag: Option<String>,
    /// One-line human-readable summary of the most recent delta we surfaced.
    pub last_delta_summary: Option<String>,
    /// Filename of the most recent pump payload we wrote (basename only,
    /// not the absolute path — callers prepend the queue dir).
    pub last_pump_payload_id: Option<String>,
    /// HTTP `ETag` from the last GitHub Releases response, used for
    /// `If-None-Match` conditional GET. Conditional 304 responses do not
    /// count against the GitHub authenticated rate limit.
    pub etag: Option<String>,
    /// HTTP `Last-Modified` from the last response, used for
    /// `If-Modified-Since` (fallback when ETag is absent).
    pub last_modified: Option<String>,
    /// Number of consecutive poll errors for this repo (resets to 0 on
    /// success). Used by daemon to apply per-repo backoff.
    pub consecutive_errors: u32,
}

impl ProjectState {
    /// Construct a fresh state record for a repo we have never polled.
    pub fn new(github_repo: impl Into<String>) -> Self {
        Self {
            github_repo: github_repo.into(),
            last_checked: None,
            last_known_tag: None,
            last_delta_summary: None,
            last_pump_payload_id: None,
            etag: None,
            last_modified: None,
            consecutive_errors: 0,
        }
    }
}

/// The whole on-disk document.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchState {
    /// Schema version — bump if the on-disk format changes incompatibly.
    pub schema_version: u32,
    /// Wall-clock time of last successful save.
    pub last_save: Option<DateTime<Utc>>,
    /// One entry per tracked repo.
    pub projects: BTreeMap<String, ProjectState>,
}

const CURRENT_SCHEMA_VERSION: u32 = 1;

impl WatchState {
    /// New empty state with the current schema version.
    pub fn new() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            last_save: None,
            projects: BTreeMap::new(),
        }
    }

    /// Get-or-insert a per-repo state record.
    pub fn entry(&mut self, repo: &str) -> &mut ProjectState {
        self.projects
            .entry(repo.to_string())
            .or_insert_with(|| ProjectState::new(repo))
    }

    /// Read-only access to a per-repo state record.
    pub fn get(&self, repo: &str) -> Option<&ProjectState> {
        self.projects.get(repo)
    }

    /// Default on-disk path. Honors `$CAVE_UPSTREAM_STATE_PATH` for tests
    /// and packaging overrides.
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("CAVE_UPSTREAM_STATE_PATH") {
            return PathBuf::from(p);
        }
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        // On macOS dirs::data_dir() is ~/Library/Application Support
        base.join("cave-runtime").join("upstream-state.json")
    }

    /// Load state from a path. Returns a fresh empty `WatchState` if the
    /// file does not exist (first run). Returns an error only on I/O or
    /// JSON parse failure of an existing file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read state file {}: {}", path.display(), e))?;
        let state: WatchState = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parse state file {}: {}", path.display(), e))?;
        if state.schema_version > CURRENT_SCHEMA_VERSION {
            anyhow::bail!(
                "state file schema_version {} is newer than supported {}",
                state.schema_version,
                CURRENT_SCHEMA_VERSION
            );
        }
        Ok(state)
    }

    /// Save state atomically: write to `<path>.tmp`, fsync, rename over
    /// `<path>`. A crash before rename leaves the original file intact.
    pub fn save(&mut self, path: &Path) -> anyhow::Result<()> {
        self.last_save = Some(Utc::now());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!("create state dir {}: {}", parent.display(), e)
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self)?;
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&body)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path).map_err(|e| {
            // Best-effort cleanup of the tmp file on rename failure
            let _ = std::fs::remove_file(&tmp);
            anyhow::anyhow!("rename state {} -> {}: {}", tmp.display(), path.display(), e)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_missing_file_returns_empty_state() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        let state = WatchState::load(&path).expect("missing file is not an error");
        assert_eq!(state.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(state.projects.is_empty());
        assert!(state.last_save.is_none());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("state.json");

        let mut state = WatchState::new();
        let entry = state.entry("etcd-io/etcd");
        entry.last_known_tag = Some("v3.5.10".to_string());
        entry.last_checked = Some(Utc::now());
        entry.consecutive_errors = 2;
        entry.etag = Some("\"abc123\"".to_string());

        state.save(&path).expect("save succeeds");
        assert!(path.exists(), "save must create the file");
        assert!(state.last_save.is_some(), "save must stamp last_save");

        let reloaded = WatchState::load(&path).expect("load succeeds");
        let etcd = reloaded.get("etcd-io/etcd").expect("etcd entry present");
        assert_eq!(etcd.last_known_tag.as_deref(), Some("v3.5.10"));
        assert_eq!(etcd.consecutive_errors, 2);
        assert_eq!(etcd.etag.as_deref(), Some("\"abc123\""));
    }

    #[test]
    fn entry_is_idempotent_get_or_insert() {
        let mut state = WatchState::new();
        let _ = state.entry("foo/bar");
        let _ = state.entry("foo/bar");
        let _ = state.entry("foo/bar");
        assert_eq!(state.projects.len(), 1);
    }

    #[test]
    fn save_is_atomic_no_tmp_file_left_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("state.json");
        let mut state = WatchState::new();
        state.entry("foo/bar").last_known_tag = Some("v1.0".to_string());
        state.save(&path).unwrap();

        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(entries.len(), 1, "only the final file should remain");
        assert_eq!(entries[0].to_string_lossy(), "state.json");
    }

    #[test]
    fn load_rejects_future_schema_version() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("future.json");
        let body = r#"{"schema_version": 9999, "last_save": null, "projects": {}}"#;
        std::fs::write(&path, body).unwrap();
        let err = WatchState::load(&path).expect_err("future schema must error");
        let msg = format!("{err}");
        assert!(msg.contains("9999"), "error mentions the offending version: {msg}");
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("a/b/c/state.json");
        let mut state = WatchState::new();
        state.save(&path).expect("save creates parent dirs");
        assert!(path.exists());
    }

    #[test]
    fn env_var_overrides_default_path() {
        let tmp = TempDir::new().unwrap();
        let custom = tmp.path().join("custom.json");
        // SAFETY: tests in this module are not run in parallel against this
        // env var; we set/unset locally.
        std::env::set_var("CAVE_UPSTREAM_STATE_PATH", &custom);
        let resolved = WatchState::default_path();
        std::env::remove_var("CAVE_UPSTREAM_STATE_PATH");
        assert_eq!(resolved, custom);
    }
}
