// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Picks the next queue item to process, subject to guardrails.
//!
//! Guardrails checked before each pick:
//!   • max 1 item in_progress (across all repos)
//!   • daily commit quota of 120 (tracked in primary queue)
//!   • at least 8 GiB of disk space free (primary workspace)
//!
//! Items with `attempts >= min_attempts_before_stuck` (default 3) are
//! transitioned to `stuck` rather than back to `pending` on failure.
//!
//! Multi-repo weighted pick: a 10-slot round-robin window distributes picks
//! across primary + N secondaries according to their weights.  If the
//! scheduled repo has no pending items, remaining repos are tried in order.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use thiserror::Error;

use crate::queue::{QueueError, QueueItem, QueueStatus, QueueStore};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("queue error: {0}")]
    Queue(#[from] QueueError),
    #[error("guard violated: {0}")]
    GuardViolated(String),
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct GuardrailConfig {
    pub max_in_progress: usize,
    pub daily_commit_quota: u32,
    pub min_disk_free_bytes: u64,
    pub min_attempts_before_stuck: u32,
}

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            max_in_progress: 1,
            daily_commit_quota: 120,
            min_disk_free_bytes: 8 * 1024 * 1024 * 1024, // 8 GiB
            min_attempts_before_stuck: 3,
        }
    }
}

// ── DiskFreeChecker trait ─────────────────────────────────────────────────────

pub trait DiskFreeChecker: Send + Sync {
    fn free_bytes(&self, path: &Path) -> Result<u64, std::io::Error>;
}

/// Production implementation — runs `df -P <path>` and parses the available
/// column, which is in 512-byte blocks under POSIX.
pub struct SystemDiskChecker;

impl DiskFreeChecker for SystemDiskChecker {
    fn free_bytes(&self, path: &Path) -> Result<u64, std::io::Error> {
        let out = std::process::Command::new("df")
            .args(["-P", &path.to_string_lossy()])
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        // Second line, fourth whitespace-delimited field (Available).
        let line = stdout.lines().nth(1).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "df: no data line")
        })?;
        let blocks: u64 = line
            .split_whitespace()
            .nth(3)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "df: missing available field")
            })?
            .parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e}")))?;
        // POSIX df -P reports 512-byte blocks.
        Ok(blocks * 512)
    }
}

// ── Secondary repo descriptor ─────────────────────────────────────────────────

/// Metadata for a secondary repository queue.
#[derive(Debug, Clone)]
pub struct SecondaryRepo {
    pub queue: QueueStore,
    /// Absolute path to the secondary repo's workspace root.
    pub repo_root: PathBuf,
    /// Default weekly branch for commits in this repo.
    pub default_branch: String,
    /// Scheduling weight (0.0–1.0).  Primary weight = 1 − this value.
    pub weight: f32,
}

// ── Scheduler ─────────────────────────────────────────────────────────────────

pub struct Scheduler<D: DiskFreeChecker = SystemDiskChecker> {
    queue: QueueStore,
    secondaries: Vec<SecondaryRepo>,
    config: GuardrailConfig,
    disk: D,
    workspace_root: PathBuf,
    /// Monotonic counter for weighted round-robin selection (10-slot window).
    pick_counter: AtomicU32,
}

impl Scheduler<SystemDiskChecker> {
    pub fn new(workspace_root: &Path) -> Self {
        Self::with_checker(workspace_root, GuardrailConfig::default(), SystemDiskChecker)
    }
}

impl<D: DiskFreeChecker> Scheduler<D> {
    pub fn with_checker(workspace_root: &Path, config: GuardrailConfig, disk: D) -> Self {
        Self {
            queue: QueueStore::open(workspace_root),
            secondaries: Vec::new(),
            config,
            disk,
            workspace_root: workspace_root.to_path_buf(),
            pick_counter: AtomicU32::new(0),
        }
    }

    /// Appends a secondary repo to the weighted scheduling pool.
    pub fn add_secondary(&mut self, secondary: SecondaryRepo) {
        self.secondaries.push(secondary);
    }

    /// Compat shim — replaces any existing single secondary (for tests).
    pub fn set_secondary(&mut self, secondary: SecondaryRepo) {
        self.secondaries.clear();
        self.secondaries.push(secondary);
    }

    /// Returns a shared reference to the primary queue store.
    pub fn queue(&self) -> &QueueStore {
        &self.queue
    }

    /// Returns the first secondary repo descriptor, if any (compat accessor).
    pub fn secondary(&self) -> Option<&SecondaryRepo> {
        self.secondaries.first()
    }

    /// Returns all secondary repo descriptors.
    pub fn secondaries(&self) -> &[SecondaryRepo] {
        &self.secondaries
    }

    /// Checks all guardrails.  Returns the first `GuardViolated` encountered.
    pub fn check_guards(&self) -> Result<(), SchedulerError> {
        // in_progress is aggregated across all repos.
        let mut in_progress = self.queue.count_status(&QueueStatus::InProgress)?;
        for sec in &self.secondaries {
            in_progress += sec.queue.count_status(&QueueStatus::InProgress)?;
        }
        if in_progress >= self.config.max_in_progress {
            return Err(SchedulerError::GuardViolated(format!(
                "max_in_progress reached ({in_progress}/{})",
                self.config.max_in_progress
            )));
        }

        // Daily commits are tracked only in the primary queue.
        let daily = self.queue.daily_commits_today()?;
        if daily >= self.config.daily_commit_quota {
            return Err(SchedulerError::GuardViolated(format!(
                "daily_commit_quota reached ({daily}/{})",
                self.config.daily_commit_quota
            )));
        }

        let free = self.disk.free_bytes(&self.workspace_root).map_err(|e| {
            SchedulerError::GuardViolated(format!("disk free check failed: {e}"))
        })?;
        if free < self.config.min_disk_free_bytes {
            return Err(SchedulerError::GuardViolated(format!(
                "disk free {free} bytes < {} bytes required",
                self.config.min_disk_free_bytes
            )));
        }

        Ok(())
    }

    /// Picks the next pending item and transitions it to `in_progress`.
    ///
    /// Builds a 10-slot weighted round-robin across primary + all secondaries.
    /// Slot assignment: primary gets `round(primary_weight * 10)` slots, each
    /// secondary gets `round(secondary.weight * 10)` slots; any remainder goes
    /// to primary.  On each tick the scheduled repo is tried first; if it has
    /// no pending items, remaining repos are tried in order.
    ///
    /// Returns `Ok(None)` when no pending items exist in any repo.
    pub fn pick_next(&self) -> Result<Option<QueueItem>, SchedulerError> {
        self.check_guards()?;

        // Build repo order for this slot (0 = primary, 1..N = secondaries).
        let total_secondary_weight: f32 = self.secondaries.iter().map(|s| s.weight).sum();
        let primary_slots =
            ((((1.0 - total_secondary_weight) * 10.0).round() as u32).max(1)).min(10);

        let counter = self.pick_counter.fetch_add(1, Ordering::Relaxed);
        let slot = counter % 10;

        // Determine which repo index is "scheduled" for this slot.
        // Slots [0..primary_slots) → primary (index 0 in the ordering below).
        // Subsequent slots carve out secondaries by their weights in order.
        let scheduled = if slot < primary_slots {
            0usize // primary
        } else {
            let mut offset = slot - primary_slots;
            let mut idx = 1usize;
            for sec in &self.secondaries {
                let sec_slots = ((sec.weight * 10.0).round() as u32).max(1);
                if offset < sec_slots {
                    break;
                }
                offset -= sec_slots;
                idx += 1;
            }
            idx.min(self.secondaries.len()) // clamp to valid secondary index
        };

        // Try repos starting from the scheduled one, wrapping around.
        // repo index 0 = primary, 1..=N = secondaries[0..N-1].
        let n = 1 + self.secondaries.len();
        for i in 0..n {
            let repo_idx = (scheduled + i) % n;
            let item = if repo_idx == 0 {
                self.try_pick_primary()?
            } else {
                self.try_pick_at_secondary(repo_idx - 1)?
            };
            if item.is_some() {
                return Ok(item);
            }
        }

        Ok(None)
    }

    /// Marks a failed item.
    ///
    /// If `item.attempts >= min_attempts_before_stuck` the item goes to `stuck`;
    /// otherwise it returns to `pending` for a retry.
    pub fn mark_failed(
        &self,
        item: &QueueItem,
        error: impl Into<String>,
    ) -> Result<(), SchedulerError> {
        let status = if item.attempts >= self.config.min_attempts_before_stuck {
            QueueStatus::Stuck
        } else {
            QueueStatus::Pending
        };
        self.store_for(item).update_item(&item.id, status, item.attempts, Some(error.into()))?;
        Ok(())
    }

    /// Marks an item as done and increments today's commit counter (primary queue).
    pub fn mark_done(&self, item: &QueueItem) -> Result<(), SchedulerError> {
        self.store_for(item).update_item(&item.id, QueueStatus::Done, 0, None)?;
        // Daily commit counter lives in the primary queue regardless of source repo.
        self.queue.increment_daily_commits()?;
        Ok(())
    }

    // ── Private ───────────────────────────────────────────────────────────────

    /// Returns the queue store that owns this item (determined by repo_path).
    fn store_for(&self, item: &QueueItem) -> &QueueStore {
        for sec in &self.secondaries {
            if item.repo_path.as_deref() == Some(sec.repo_root.as_path()) {
                return &sec.queue;
            }
        }
        &self.queue
    }

    fn try_pick_primary(&self) -> Result<Option<QueueItem>, SchedulerError> {
        let Some(item) = self.queue.next_pending()? else {
            return Ok(None);
        };
        let new_attempts = item.attempts + 1;
        self.queue.update_item(
            &item.id,
            QueueStatus::InProgress,
            new_attempts,
            item.last_error.clone(),
        )?;
        let updated = self.queue.list()?.into_iter().find(|i| i.id == item.id)
            .ok_or_else(|| SchedulerError::Queue(QueueError::ItemNotFound(item.id.clone())))?;
        Ok(Some(updated))
    }

    fn try_pick_at_secondary(&self, idx: usize) -> Result<Option<QueueItem>, SchedulerError> {
        let Some(sec) = self.secondaries.get(idx) else {
            return Ok(None);
        };
        let Some(item) = sec.queue.next_pending()? else {
            return Ok(None);
        };
        let new_attempts = item.attempts + 1;
        sec.queue.update_item(
            &item.id,
            QueueStatus::InProgress,
            new_attempts,
            item.last_error.clone(),
        )?;
        let mut updated = sec.queue.list()?.into_iter().find(|i| i.id == item.id)
            .ok_or_else(|| SchedulerError::Queue(QueueError::ItemNotFound(item.id.clone())))?;
        // Inject repo context if not already set in the YAML item.
        if updated.repo_path.is_none() {
            updated.repo_path = Some(sec.repo_root.clone());
        }
        if updated.branch.is_none() {
            updated.branch = Some(sec.default_branch.clone());
        }
        Ok(Some(updated))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::QueueItem;
    use tempfile::TempDir;

    // ── Mock disk checker ─────────────────────────────────────────────────────

    struct MockDisk(u64);
    impl DiskFreeChecker for MockDisk {
        fn free_bytes(&self, _: &Path) -> Result<u64, std::io::Error> {
            Ok(self.0)
        }
    }

    struct FailDisk;
    impl DiskFreeChecker for FailDisk {
        fn free_bytes(&self, _: &Path) -> Result<u64, std::io::Error> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "simulated failure"))
        }
    }

    fn plenty_of_disk() -> u64 {
        100 * 1024 * 1024 * 1024 // 100 GiB
    }

    fn sched(dir: &TempDir) -> Scheduler<MockDisk> {
        Scheduler::with_checker(dir.path(), GuardrailConfig::default(), MockDisk(plenty_of_disk()))
    }

    // ── pick_next happy path ──────────────────────────────────────────────────

    #[test]
    fn test_pick_next_returns_highest_priority_item() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        let item = s.pick_next().unwrap().unwrap();
        assert_eq!(item.crate_name, "cave-secrets");
        assert_eq!(item.status, QueueStatus::InProgress);
    }

    #[test]
    fn test_pick_next_increments_attempts() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        let item = s.pick_next().unwrap().unwrap();
        assert_eq!(item.attempts, 1);
    }

    #[test]
    fn test_pick_next_none_when_no_pending() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        for i in s.queue().list().unwrap() {
            s.queue().update_item(&i.id, QueueStatus::Done, 0, None).unwrap();
        }
        assert!(s.pick_next().unwrap().is_none());
    }

    // ── Guards ────────────────────────────────────────────────────────────────

    #[test]
    fn test_guard_max_in_progress() {
        let dir = TempDir::new().unwrap();
        let cfg = GuardrailConfig { max_in_progress: 1, ..GuardrailConfig::default() };
        let s = Scheduler::with_checker(dir.path(), cfg, MockDisk(plenty_of_disk()));
        // First pick succeeds.
        s.pick_next().unwrap().unwrap();
        // Second pick hits the guard.
        let err = s.pick_next().unwrap_err();
        assert!(matches!(err, SchedulerError::GuardViolated(_)));
        assert!(err.to_string().contains("max_in_progress"));
    }

    #[test]
    fn test_guard_daily_commit_quota() {
        let dir = TempDir::new().unwrap();
        let cfg = GuardrailConfig { daily_commit_quota: 0, ..GuardrailConfig::default() };
        let s = Scheduler::with_checker(dir.path(), cfg, MockDisk(plenty_of_disk()));
        let err = s.pick_next().unwrap_err();
        assert!(err.to_string().contains("daily_commit_quota"));
    }

    #[test]
    fn test_guard_disk_too_low() {
        let dir = TempDir::new().unwrap();
        let cfg = GuardrailConfig {
            min_disk_free_bytes: 10 * 1024 * 1024 * 1024,
            ..GuardrailConfig::default()
        };
        // Only 1 byte free.
        let s = Scheduler::with_checker(dir.path(), cfg, MockDisk(1));
        let err = s.pick_next().unwrap_err();
        assert!(err.to_string().contains("disk free"));
    }

    #[test]
    fn test_guard_disk_check_io_error() {
        let dir = TempDir::new().unwrap();
        let s = Scheduler::with_checker(dir.path(), GuardrailConfig::default(), FailDisk);
        let err = s.pick_next().unwrap_err();
        assert!(matches!(err, SchedulerError::GuardViolated(_)));
    }

    #[test]
    fn test_check_guards_passes_with_plenty_of_disk() {
        let dir = TempDir::new().unwrap();
        sched(&dir).check_guards().unwrap();
    }

    // ── mark_failed / mark_done ───────────────────────────────────────────────

    #[test]
    fn test_mark_failed_below_threshold_goes_pending() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        let item = s.pick_next().unwrap().unwrap(); // attempts=1
        s.mark_failed(&item, "test_fail").unwrap();
        let updated = s
            .queue()
            .list()
            .unwrap()
            .into_iter()
            .find(|i| i.id == item.id)
            .unwrap();
        // min_attempts_before_stuck default is 3; attempts=1 → pending
        assert_eq!(updated.status, QueueStatus::Pending);
    }

    #[test]
    fn test_mark_failed_at_threshold_goes_stuck() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        let mut item = s.pick_next().unwrap().unwrap();
        // Simulate we've already tried twice; this attempt is the 3rd.
        item.attempts = 3;
        s.mark_failed(&item, "timeout").unwrap();
        let updated = s
            .queue()
            .list()
            .unwrap()
            .into_iter()
            .find(|i| i.id == item.id)
            .unwrap();
        assert_eq!(updated.status, QueueStatus::Stuck);
        assert_eq!(updated.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn test_mark_done_increments_commit_counter() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        let item = s.pick_next().unwrap().unwrap();
        assert_eq!(s.queue().daily_commits_today().unwrap(), 0);
        s.mark_done(&item).unwrap();
        assert_eq!(s.queue().daily_commits_today().unwrap(), 1);
    }

    #[test]
    fn test_mark_done_sets_status_done() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        let item = s.pick_next().unwrap().unwrap();
        s.mark_done(&item).unwrap();
        let updated = s
            .queue()
            .list()
            .unwrap()
            .into_iter()
            .find(|i| i.id == item.id)
            .unwrap();
        assert_eq!(updated.status, QueueStatus::Done);
    }

    #[test]
    fn test_add_extra_item_is_schedulable() {
        let dir = TempDir::new().unwrap();
        let s = sched(&dir);
        // Complete all seeded items.
        for i in s.queue().list().unwrap() {
            s.queue().update_item(&i.id, QueueStatus::Done, 0, None).unwrap();
        }
        s.queue()
            .add(QueueItem::new("new-crate", "org/repo", "f.go", "Fn", 1))
            .unwrap();
        let item = s.pick_next().unwrap().unwrap();
        assert_eq!(item.crate_name, "new-crate");
    }

    // ── Multi-repo / secondary queue ──────────────────────────────────────────

    #[test]
    fn test_secondary_queue_items_get_repo_path_injected() {
        let primary_dir = TempDir::new().unwrap();
        let secondary_dir = TempDir::new().unwrap();
        let secondary_queue = QueueStore::open(secondary_dir.path());

        // Mark all primary items done so secondary must be used.
        let primary_store = QueueStore::open(primary_dir.path());
        for item in primary_store.list().unwrap() {
            primary_store.update_item(&item.id, QueueStatus::Done, 0, None).unwrap();
        }

        let sec = SecondaryRepo {
            queue: secondary_queue,
            repo_root: secondary_dir.path().to_path_buf(),
            default_branch: "qwen/auto-2026-W17".into(),
            weight: 0.3,
        };

        let mut s = Scheduler::with_checker(
            primary_dir.path(),
            GuardrailConfig::default(),
            MockDisk(plenty_of_disk()),
        );
        s.set_secondary(sec);

        let item = s.pick_next().unwrap().unwrap();
        assert_eq!(item.repo_path.as_deref(), Some(secondary_dir.path()));
        assert_eq!(item.branch.as_deref(), Some("qwen/auto-2026-W17"));
    }

    #[test]
    fn test_secondary_in_progress_counts_toward_guard() {
        let primary_dir = TempDir::new().unwrap();
        let secondary_dir = TempDir::new().unwrap();

        // Set in_progress on a secondary item manually.
        let sec_store = QueueStore::open(secondary_dir.path());
        let sec_item = sec_store.list().unwrap().into_iter().next().unwrap();
        sec_store
            .update_item(&sec_item.id, QueueStatus::InProgress, 1, None)
            .unwrap();

        let cfg = GuardrailConfig { max_in_progress: 1, ..GuardrailConfig::default() };
        let mut s = Scheduler::with_checker(primary_dir.path(), cfg, MockDisk(plenty_of_disk()));
        s.set_secondary(SecondaryRepo {
            queue: QueueStore::open(secondary_dir.path()),
            repo_root: secondary_dir.path().to_path_buf(),
            default_branch: "qwen/auto-2026-W17".into(),
            weight: 0.3,
        });

        // Guard should fire because secondary already has 1 in_progress.
        let err = s.pick_next().unwrap_err();
        assert!(err.to_string().contains("max_in_progress"));
    }

    #[test]
    fn test_weighted_pick_prefers_primary() {
        let primary_dir = TempDir::new().unwrap();
        let secondary_dir = TempDir::new().unwrap();

        // Add many items to both queues.
        let pri = QueueStore::open(primary_dir.path());
        let sec_store = QueueStore::open(secondary_dir.path());
        for i in 0..20u32 {
            pri.add(QueueItem::new(format!("p-{i}"), "o/r", "f.go", "Fn", i + 1)).unwrap();
            sec_store.add(QueueItem::new(format!("s-{i}"), "o/r", "f.go", "Fn", i + 1)).unwrap();
        }

        let mut s = Scheduler::with_checker(
            primary_dir.path(),
            GuardrailConfig { max_in_progress: 100, ..GuardrailConfig::default() },
            MockDisk(plenty_of_disk()),
        );
        s.set_secondary(SecondaryRepo {
            queue: QueueStore::open(secondary_dir.path()),
            repo_root: secondary_dir.path().to_path_buf(),
            default_branch: "qwen/auto-2026-W17".into(),
            weight: 0.3,
        });

        let mut primary_picks = 0u32;
        let mut secondary_picks = 0u32;
        for _ in 0..10 {
            if let Some(item) = s.pick_next().unwrap() {
                if item.repo_path.is_none() {
                    primary_picks += 1;
                } else {
                    secondary_picks += 1;
                }
            }
        }
        // 10-slot window: 7 primary slots, 3 secondary slots.
        assert_eq!(primary_picks, 7, "expected 7 primary picks in 10-slot window");
        assert_eq!(secondary_picks, 3, "expected 3 secondary picks in 10-slot window");
    }

    #[test]
    fn test_mark_done_on_secondary_item_updates_secondary_queue() {
        let primary_dir = TempDir::new().unwrap();
        let secondary_dir = TempDir::new().unwrap();

        // Drain primary queue.
        let pri = QueueStore::open(primary_dir.path());
        for i in pri.list().unwrap() {
            pri.update_item(&i.id, QueueStatus::Done, 0, None).unwrap();
        }

        let mut s = Scheduler::with_checker(
            primary_dir.path(),
            GuardrailConfig::default(),
            MockDisk(plenty_of_disk()),
        );
        s.set_secondary(SecondaryRepo {
            queue: QueueStore::open(secondary_dir.path()),
            repo_root: secondary_dir.path().to_path_buf(),
            default_branch: "qwen/auto-2026-W17".into(),
            weight: 0.3,
        });

        let item = s.pick_next().unwrap().unwrap();
        assert!(item.repo_path.is_some(), "item should come from secondary");
        s.mark_done(&item).unwrap();

        // Secondary store should have the item marked done.
        let sec_items = QueueStore::open(secondary_dir.path()).list().unwrap();
        let found = sec_items.iter().find(|i| i.id == item.id).unwrap();
        assert_eq!(found.status, QueueStatus::Done);
    }
}
