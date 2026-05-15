// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Persistent state — `<data_dir>/watchd/state.json`.
//!
//! One file, JSON, atomic write via tempfile+rename. Per-project
//! entry records:
//!
//! * `last_poll`         — RFC-3339 timestamp of the last successful HTTP exchange.
//! * `last_known_tag`    — the latest release tag observed.
//! * `etag`              — HTTP `ETag` from the latest poll (304 cache).
//! * `last_modified`     — HTTP `Last-Modified` header (secondary cache).
//! * `consecutive_errors`— transient failure counter driving backoff.
//!
//! Operators can `cat` this file, hand-edit it to force a re-poll
//! (delete the entry), and the next tick will refresh it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchStateEntry {
    pub last_poll: Option<DateTime<Utc>>,
    pub last_known_tag: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    #[serde(default)]
    pub consecutive_errors: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchState {
    /// `github_repo` → entry.
    pub entries: BTreeMap<String, WatchStateEntry>,
    /// Daemon's `last_tick_at` — used by the portal status panel
    /// to render "last polled X minutes ago".
    pub last_tick_at: Option<DateTime<Utc>>,
}

impl WatchState {
    /// Load state from disk. Returns `Ok(default())` when the file
    /// doesn't exist yet — first boot is normal.
    pub fn load(path: &Path) -> Result<Self, StateError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(StateError::Io(e)),
        }
    }

    /// Atomic write via tempfile+rename. Same dir as the target so the
    /// rename is on the same filesystem.
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let tmp = parent.join(format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("watchd-state")
        ));
        let json = serde_json::to_vec_pretty(self)?;
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Resolve the default state-file path. Honours
    /// `$CAVE_WATCHD_STATE` then falls back to
    /// `<dirs::data_dir>/cave-runtime/watchd/state.json`.
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("CAVE_WATCHD_STATE") {
            return PathBuf::from(p);
        }
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("cave-runtime").join("watchd").join("state.json")
    }

    pub fn upsert(&mut self, repo: &str, mut update: impl FnMut(&mut WatchStateEntry)) {
        let entry = self.entries.entry(repo.to_string()).or_default();
        update(entry);
    }

    pub fn get(&self, repo: &str) -> Option<&WatchStateEntry> {
        self.entries.get(repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let mut s = WatchState::default();
        s.last_tick_at = Some(ts("2026-05-13T13:00:00Z"));
        s.upsert("etcd-io/etcd", |e| {
            e.last_poll = Some(ts("2026-05-13T13:00:00Z"));
            e.last_known_tag = Some("v3.5.13".into());
            e.etag = Some("\"deadbeef\"".into());
        });
        s.save(&path).unwrap();
        let back = WatchState::load(&path).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("never-exists.json");
        let s = WatchState::load(&path).unwrap();
        assert!(s.entries.is_empty());
        assert!(s.last_tick_at.is_none());
    }

    #[test]
    fn upsert_creates_then_updates_entry() {
        let mut s = WatchState::default();
        s.upsert("a/b", |e| e.consecutive_errors = 1);
        s.upsert("a/b", |e| {
            e.consecutive_errors += 1;
            e.etag = Some("\"x\"".into());
        });
        let e = s.get("a/b").unwrap();
        assert_eq!(e.consecutive_errors, 2);
        assert_eq!(e.etag.as_deref(), Some("\"x\""));
    }

    #[test]
    fn save_overwrites_atomically_no_partial_files_left() {
        // Atomicity test: write, then write a smaller payload, then
        // confirm the on-disk content is exactly the second payload
        // (no stray tempfile, no leftover from the first write).
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("state.json");

        let mut big = WatchState::default();
        for i in 0..100 {
            big.upsert(&format!("o/r{i}"), |e| {
                e.last_known_tag = Some(format!("v{i}"));
            });
        }
        big.save(&path).unwrap();
        assert!(path.is_file());

        let small = WatchState::default();
        small.save(&path).unwrap();
        let back = WatchState::load(&path).unwrap();
        assert!(back.entries.is_empty());

        // No leftover tempfile.
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".state.json")
            })
            .collect();
        assert!(leftover.is_empty());
    }

    #[test]
    fn default_path_honors_cave_watchd_state_env() {
        unsafe {
            std::env::set_var("CAVE_WATCHD_STATE", "/tmp/__cave_watchd_test/state.json");
        }
        let p = WatchState::default_path();
        assert_eq!(p, PathBuf::from("/tmp/__cave_watchd_test/state.json"));
        unsafe {
            std::env::remove_var("CAVE_WATCHD_STATE");
        }
    }
}
