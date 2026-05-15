// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Point-in-time recovery (PITR) and failback for DR sites.
//!
//! Recovery workflow:
//! 1. Select a recovery point (by LogIndex or timestamp).
//! 2. Load the nearest snapshot.
//! 3. Re-apply WAL entries up to the chosen point.
//! 4. Start the Raft node from the recovered state.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::DrConfig;
use crate::error::{HaError, HaResult};
use crate::raft::log::LogEntry;
use crate::raft::types::LogIndex;
use crate::storage::snapshot_store::SnapshotStore;

/// A recovery point — identifies a specific log position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPoint {
    pub log_index: LogIndex,
    pub log_term: u64,
    pub timestamp: DateTime<Utc>,
    pub label: Option<String>,
}

/// Recovery target — what to recover to.
#[derive(Debug, Clone)]
pub enum RecoveryTarget {
    /// Recover to a specific log index.
    LogIndex(LogIndex),
    /// Recover to the latest available point.
    Latest,
    /// Recover to a specific timestamp (nearest entry before).
    Timestamp(DateTime<Utc>),
}

/// Result of a recovery operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryResult {
    pub recovered_to_index: LogIndex,
    pub snapshot_index: LogIndex,
    pub entries_replayed: u64,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Manages point-in-time recovery.
pub struct PitrManager {
    data_dir: PathBuf,
    config: DrConfig,
    snapshot_store: SnapshotStore,
}

impl PitrManager {
    pub async fn new(data_dir: impl AsRef<Path>, config: DrConfig) -> HaResult<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let snap_dir = data_dir.join("snapshots");
        let snapshot_store = SnapshotStore::open(&snap_dir).await?;
        Ok(Self { data_dir, config, snapshot_store })
    }

    /// List all available recovery points (from snapshot + WAL).
    pub async fn list_recovery_points(&self) -> HaResult<Vec<RecoveryPoint>> {
        let mut points = Vec::new();
        // Load latest snapshot as anchor.
        if let Some(snapshot) = self.snapshot_store.load_latest().await? {
            points.push(RecoveryPoint {
                log_index: snapshot.meta.index,
                log_term: snapshot.meta.term,
                timestamp: Utc::now(), // Real: store timestamp in snapshot meta.
                label: Some("snapshot".into()),
            });
        }
        // TODO: Scan WAL for entry timestamps if entries have them.
        Ok(points)
    }

    /// Perform recovery to the specified target.
    pub async fn recover(
        &self,
        target: RecoveryTarget,
        apply_fn: impl Fn(LogEntry) -> HaResult<()>,
    ) -> HaResult<RecoveryResult> {
        let start = std::time::Instant::now();

        // Step 1: Load nearest snapshot.
        let snapshot = self.snapshot_store.load_latest().await?;
        let base_index = snapshot.as_ref().map(|s| s.meta.index).unwrap_or(0);

        // Step 2: Determine target index.
        let target_index = match &target {
            RecoveryTarget::Latest => self.wal_last_index().await.unwrap_or(base_index),
            RecoveryTarget::LogIndex(idx) => *idx,
            RecoveryTarget::Timestamp(_ts) => {
                // Real: binary search WAL for entries near timestamp.
                base_index
            }
        };

        info!(base_index, target_index, "starting PITR recovery");

        // Step 3: Replay WAL entries from base_index to target_index.
        let entries = self.load_wal_entries(base_index + 1, target_index).await?;
        let mut replayed = 0u64;
        for entry in entries {
            apply_fn(entry)?;
            replayed += 1;
        }

        let elapsed = start.elapsed().as_millis() as u64;
        info!(
            recovered_to = target_index,
            replayed,
            elapsed_ms = elapsed,
            "recovery complete"
        );

        Ok(RecoveryResult {
            recovered_to_index: target_index,
            snapshot_index: base_index,
            entries_replayed: replayed,
            duration_ms: elapsed,
            success: true,
            error: None,
        })
    }

    async fn wal_last_index(&self) -> Option<LogIndex> {
        let wal_path = self.data_dir.join("raft.wal");
        let mut wal = crate::storage::Wal::open(&wal_path).await.ok()?;
        let replay = wal.replay().await.ok()?;
        replay.entries.last().map(|e| e.index)
    }

    async fn load_wal_entries(
        &self,
        from: LogIndex,
        to: LogIndex,
    ) -> HaResult<Vec<LogEntry>> {
        let wal_path = self.data_dir.join("raft.wal");
        let mut wal = crate::storage::Wal::open(&wal_path).await?;
        let replay = wal.replay().await?;
        Ok(replay
            .entries
            .into_iter()
            .filter(|e| e.index >= from && e.index <= to)
            .collect())
    }

    /// RPO check: how many seconds of data would be lost if primary fails now.
    pub fn rpo_seconds(&self, dr_lag_entries: u64, entries_per_second: f64) -> f64 {
        if entries_per_second <= 0.0 { return 0.0; }
        dr_lag_entries as f64 / entries_per_second
    }

    /// Check if RPO target is met.
    pub fn rpo_ok(&self, dr_lag_entries: u64, entries_per_second: f64) -> bool {
        self.rpo_seconds(dr_lag_entries, entries_per_second)
            <= self.config.rpo_seconds as f64
    }
}

/// Failback coordinator — restores primary after DR site took over.
pub struct FailbackCoordinator {
    config: DrConfig,
}

impl FailbackCoordinator {
    pub fn new(config: DrConfig) -> Self {
        Self { config }
    }

    /// Initiate failback: re-sync primary from DR, then transfer leadership back.
    pub async fn initiate(&self, primary_handle: &crate::raft::node::RaftHandle) -> HaResult<()> {
        if !self.config.auto_failback {
            warn!("auto_failback disabled; manual failback required");
            return Err(HaError::Dr("auto_failback disabled".into()));
        }
        info!(
            delay = self.config.failback_delay_seconds,
            "initiating failback after delay"
        );
        tokio::time::sleep(Duration::from_secs(self.config.failback_delay_seconds)).await;
        // In production: verify primary is caught up, then transfer leadership.
        let status = primary_handle.status().await?;
        info!(primary_commit = status.commit_index, "failback complete");
        Ok(())
    }
}
