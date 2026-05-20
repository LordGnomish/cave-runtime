// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Priority queue backed by per-repo YAML files in `<repo>/queue/`.
//!
//! File-lock (exclusive flock) + atomic rename guarantee concurrent-safe
//! reads and writes.  A missing queue file is seeded automatically with
//! three starter items the first time the store is opened.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("item not found: {0}")]
    ItemNotFound(String),
    #[error("lock acquire failed: {0}")]
    LockFailed(String),
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    Pending,
    InProgress,
    Done,
    Stuck,
}

impl std::fmt::Display for QueueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            QueueStatus::Pending => "pending",
            QueueStatus::InProgress => "in_progress",
            QueueStatus::Done => "done",
            QueueStatus::Stuck => "stuck",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub crate_name: String,
    pub upstream_repo: String,
    pub upstream_file: String,
    pub upstream_fn: String,
    pub status: QueueStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    /// Lower value → higher scheduling priority.
    pub priority: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Absolute path to the workspace root for this task.
    /// None → use the primary daemon workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<PathBuf>,
    /// Target branch for commits from this task.
    /// None → use the daemon's configured target_branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl QueueItem {
    pub fn new(
        crate_name: impl Into<String>,
        upstream_repo: impl Into<String>,
        upstream_file: impl Into<String>,
        upstream_fn: impl Into<String>,
        priority: u32,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            crate_name: crate_name.into(),
            upstream_repo: upstream_repo.into(),
            upstream_file: upstream_file.into(),
            upstream_fn: upstream_fn.into(),
            status: QueueStatus::Pending,
            attempts: 0,
            last_error: None,
            priority,
            created_at: now,
            updated_at: now,
            repo_path: None,
            branch: None,
        }
    }
}

// ── On-disk representation ────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct QueueMeta {
    version: u32,
    /// YYYY-MM-DD → commit count for that calendar day.
    #[serde(default)]
    daily_commits: std::collections::HashMap<String, u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueueFile {
    metadata: QueueMeta,
    items: Vec<QueueItem>,
}

// ── QueueStore ────────────────────────────────────────────────────────────────

/// Thread-safe (file-lock) persistent queue store.
#[derive(Debug, Clone)]
pub struct QueueStore {
    path: PathBuf,
}

impl QueueStore {
    /// Opens the primary runtime queue at `<workspace_root>/queue/runtime-tasks.yaml`.
    pub fn open(workspace_root: &Path) -> Self {
        Self {
            path: workspace_root.join("queue").join("runtime-tasks.yaml"),
        }
    }

    /// Opens a queue at an arbitrary path.
    pub fn open_at(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// Returns all queue items.  Seeds the file on first call if absent.
    pub fn list(&self) -> Result<Vec<QueueItem>, QueueError> {
        Ok(self.read_locked()?.items)
    }

    /// Appends a new item and persists.
    pub fn add(&self, item: QueueItem) -> Result<(), QueueError> {
        self.mutate(|qf| {
            qf.items.push(item);
            Ok(())
        })
    }

    /// Returns the highest-priority pending item (lowest `priority`, then
    /// oldest `created_at`).
    pub fn next_pending(&self) -> Result<Option<QueueItem>, QueueError> {
        let qf = self.read_locked()?;
        let item = qf
            .items
            .iter()
            .filter(|i| i.status == QueueStatus::Pending)
            .min_by_key(|i| (i.priority, i.created_at))
            .cloned();
        Ok(item)
    }

    /// Counts items with the given status.
    pub fn count_status(&self, status: &QueueStatus) -> Result<usize, QueueError> {
        Ok(self
            .read_locked()?
            .items
            .iter()
            .filter(|i| &i.status == status)
            .count())
    }

    /// Updates status, attempts, and last_error for the item with `id`.
    pub fn update_item(
        &self,
        id: &str,
        status: QueueStatus,
        attempts: u32,
        last_error: Option<String>,
    ) -> Result<(), QueueError> {
        self.mutate(|qf| {
            let item = qf
                .items
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or_else(|| QueueError::ItemNotFound(id.to_string()))?;
            item.status = status;
            item.attempts = attempts;
            item.last_error = last_error;
            item.updated_at = Utc::now();
            Ok(())
        })
    }

    /// Returns today's (UTC) commit count.
    pub fn daily_commits_today(&self) -> Result<u32, QueueError> {
        let qf = self.read_locked()?;
        Ok(*qf.metadata.daily_commits.get(&today_key()).unwrap_or(&0))
    }

    /// Increments today's commit counter by one.
    pub fn increment_daily_commits(&self) -> Result<(), QueueError> {
        self.mutate(|qf| {
            *qf.metadata.daily_commits.entry(today_key()).or_insert(0) += 1;
            Ok(())
        })
    }

    // ── Private ───────────────────────────────────────────────────────────────

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("yaml.lock")
    }

    /// Acquires an exclusive flock on the `.lock` file.
    /// The returned `File` keeps the lock alive until dropped.
    fn acquire_lock(&self) -> Result<File, QueueError> {
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p)?;
        }
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())?;
        f.lock_exclusive()
            .map_err(|e| QueueError::LockFailed(e.to_string()))?;
        Ok(f)
    }

    fn read_or_seed(&self) -> Result<QueueFile, QueueError> {
        if !self.path.exists() {
            let qf = seed_queue();
            self.write_atomic(&qf)?;
            return Ok(qf);
        }
        let raw = fs::read_to_string(&self.path)?;
        Ok(serde_yaml::from_str(&raw)?)
    }

    /// Acquires lock, reads (seeding if absent), releases lock, returns data.
    fn read_locked(&self) -> Result<QueueFile, QueueError> {
        let _lock = self.acquire_lock()?;
        self.read_or_seed()
    }

    /// Acquires lock, reads → mutates → writes atomically, releases lock.
    fn mutate<F>(&self, f: F) -> Result<(), QueueError>
    where
        F: FnOnce(&mut QueueFile) -> Result<(), QueueError>,
    {
        let _lock = self.acquire_lock()?;
        let mut qf = self.read_or_seed()?;
        f(&mut qf)?;
        self.write_atomic(&qf)
    }

    /// Writes `qf` to a `.tmp` file then atomically renames it to `self.path`.
    fn write_atomic(&self, qf: &QueueFile) -> Result<(), QueueError> {
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p)?;
        }
        let tmp = self.path.with_extension("yaml.tmp");
        let content = serde_yaml::to_string(qf)?;
        let mut f = File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
        drop(f);
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn today_key() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn seed_queue() -> QueueFile {
    let now = Utc::now();
    let make = |crate_name: &str,
                upstream_repo: &str,
                upstream_file: &str,
                upstream_fn: &str,
                priority: u32|
     -> QueueItem {
        QueueItem {
            id: Uuid::new_v4().to_string(),
            crate_name: crate_name.into(),
            upstream_repo: upstream_repo.into(),
            upstream_file: upstream_file.into(),
            upstream_fn: upstream_fn.into(),
            status: QueueStatus::Pending,
            attempts: 0,
            last_error: None,
            priority,
            created_at: now,
            updated_at: now,
            repo_path: None,
            branch: None,
        }
    };
    QueueFile {
        metadata: QueueMeta {
            version: 1,
            daily_commits: Default::default(),
        },
        items: vec![
            make(
                "cave-secrets",
                "trufflesecurity/trufflehog",
                "pkg/detectors/aws/aws.go",
                "FromData",
                1,
            ),
            make(
                "cave-auth",
                "trufflesecurity/trufflehog",
                "pkg/detectors/github/github_old.go",
                "FromData",
                2,
            ),
            make(
                "cave-events",
                "etcd-io/etcd",
                "server/etcdserver/api/v3rpc/watch.go",
                "Watch",
                3,
            ),
        ],
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store(dir: &TempDir) -> QueueStore {
        QueueStore::open(dir.path())
    }

    // ── Seed ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_seed_on_missing_file_creates_three_items() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        let items = q.list().unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_seed_items_have_expected_crates() {
        let dir = TempDir::new().unwrap();
        let items = store(&dir).list().unwrap();
        let crates: Vec<&str> = items.iter().map(|i| i.crate_name.as_str()).collect();
        assert!(crates.contains(&"cave-secrets"));
        assert!(crates.contains(&"cave-auth"));
        assert!(crates.contains(&"cave-events"));
    }

    #[test]
    fn test_seed_items_are_all_pending() {
        let dir = TempDir::new().unwrap();
        let items = store(&dir).list().unwrap();
        assert!(items.iter().all(|i| i.status == QueueStatus::Pending));
    }

    #[test]
    fn test_seed_yaml_file_is_created_on_disk() {
        let dir = TempDir::new().unwrap();
        let _ = store(&dir).list().unwrap();
        assert!(dir.path().join("queue").join("runtime-tasks.yaml").exists());
    }

    #[test]
    fn test_open_at_custom_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("custom").join("tasks.yaml");
        let q = QueueStore::open_at(&path);
        let items = q.list().unwrap();
        assert_eq!(items.len(), 3, "custom path should also seed on first open");
    }

    #[test]
    fn test_second_open_does_not_reseed() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        q.add(QueueItem::new("extra-crate", "org/repo", "f.go", "Fn", 10))
            .unwrap();
        // Re-open — must not wipe the extra item.
        let q2 = store(&dir);
        let items = q2.list().unwrap();
        assert_eq!(items.len(), 4, "expected 3 seed + 1 added");
    }

    // ── repo_path / branch fields ─────────────────────────────────────────────

    #[test]
    fn test_queue_item_repo_path_defaults_none() {
        let item = QueueItem::new("crate", "org/repo", "f.go", "Fn", 1);
        assert!(item.repo_path.is_none());
        assert!(item.branch.is_none());
    }

    #[test]
    fn test_queue_item_repo_path_roundtrips_yaml() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        let mut item = QueueItem::new("crate", "org/repo", "f.go", "Fn", 1);
        item.repo_path = Some(PathBuf::from("/some/repo"));
        item.branch = Some("qwen/auto-2026-W17".into());
        q.add(item.clone()).unwrap();
        let found = q
            .list()
            .unwrap()
            .into_iter()
            .find(|i| i.crate_name == "crate")
            .unwrap();
        assert_eq!(
            found.repo_path.as_deref(),
            Some(std::path::Path::new("/some/repo"))
        );
        assert_eq!(found.branch.as_deref(), Some("qwen/auto-2026-W17"));
    }

    // ── Add / list ────────────────────────────────────────────────────────────

    #[test]
    fn test_add_persists_item() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        q.add(QueueItem::new("cave-test", "org/repo", "f.go", "Fn", 5))
            .unwrap();
        let items = q.list().unwrap();
        assert!(items.iter().any(|i| i.crate_name == "cave-test"));
    }

    #[test]
    fn test_list_returns_all_items() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        assert_eq!(q.list().unwrap().len(), 3);
    }

    // ── Update ────────────────────────────────────────────────────────────────

    #[test]
    fn test_update_item_changes_status() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        let id = q.list().unwrap()[0].id.clone();
        q.update_item(&id, QueueStatus::InProgress, 1, None)
            .unwrap();
        let item = q.list().unwrap().into_iter().find(|i| i.id == id).unwrap();
        assert_eq!(item.status, QueueStatus::InProgress);
        assert_eq!(item.attempts, 1);
    }

    #[test]
    fn test_update_item_records_error() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        let id = q.list().unwrap()[0].id.clone();
        q.update_item(&id, QueueStatus::Stuck, 3, Some("test_fail".into()))
            .unwrap();
        let item = q.list().unwrap().into_iter().find(|i| i.id == id).unwrap();
        assert_eq!(item.last_error.as_deref(), Some("test_fail"));
    }

    #[test]
    fn test_update_item_unknown_id_returns_error() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        let err = q
            .update_item("nonexistent", QueueStatus::Done, 0, None)
            .unwrap_err();
        assert!(matches!(err, QueueError::ItemNotFound(_)));
    }

    // ── next_pending ──────────────────────────────────────────────────────────

    #[test]
    fn test_next_pending_returns_lowest_priority() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        // Seed gives priorities 1, 2, 3 — lowest == highest scheduling priority.
        let item = q.next_pending().unwrap().unwrap();
        assert_eq!(item.priority, 1);
        assert_eq!(item.crate_name, "cave-secrets");
    }

    #[test]
    fn test_next_pending_skips_non_pending() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        let id = q.list().unwrap()[0].id.clone(); // priority=1 item
        q.update_item(&id, QueueStatus::Done, 0, None).unwrap();
        let item = q.next_pending().unwrap().unwrap();
        assert_eq!(item.priority, 2);
    }

    #[test]
    fn test_next_pending_none_when_queue_empty_of_pending() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        for item in q.list().unwrap() {
            q.update_item(&item.id, QueueStatus::Done, 0, None).unwrap();
        }
        assert!(q.next_pending().unwrap().is_none());
    }

    // ── count_status ──────────────────────────────────────────────────────────

    #[test]
    fn test_count_status_all_pending_initially() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        assert_eq!(q.count_status(&QueueStatus::Pending).unwrap(), 3);
        assert_eq!(q.count_status(&QueueStatus::InProgress).unwrap(), 0);
    }

    // ── Daily commits ─────────────────────────────────────────────────────────

    #[test]
    fn test_daily_commits_starts_at_zero() {
        let dir = TempDir::new().unwrap();
        assert_eq!(store(&dir).daily_commits_today().unwrap(), 0);
    }

    #[test]
    fn test_daily_commits_increment() {
        let dir = TempDir::new().unwrap();
        let q = store(&dir);
        q.increment_daily_commits().unwrap();
        q.increment_daily_commits().unwrap();
        assert_eq!(q.daily_commits_today().unwrap(), 2);
    }

    // ── Atomic rename / concurrent safety ────────────────────────────────────

    #[test]
    fn test_concurrent_adds_do_not_lose_items() {
        use std::sync::Arc;
        let dir = TempDir::new().unwrap();
        let q = Arc::new(QueueStore::open(dir.path()));
        // Warm up seed.
        let _ = q.list().unwrap();

        let threads: Vec<_> = (0..4)
            .map(|i| {
                let q2 = Arc::clone(&q);
                std::thread::spawn(move || {
                    q2.add(QueueItem::new(
                        format!("crate-{i}"),
                        "org/repo",
                        "f.go",
                        "Fn",
                        10 + i as u32,
                    ))
                    .unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(q.list().unwrap().len(), 7); // 3 seed + 4 added
    }
}
