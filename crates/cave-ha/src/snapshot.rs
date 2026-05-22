// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Snapshotting and log compaction for Raft nodes.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;

/// A point-in-time snapshot of the state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The highest log index included in this snapshot.
    pub last_included_index: u64,
    /// The term of `last_included_index`.
    pub last_included_term: u64,
    /// Serialized state machine data.
    pub data: Vec<u8>,
    /// When this snapshot was created.
    pub created_at: DateTime<Utc>,
    /// Identifies the cluster this snapshot belongs to.
    pub cluster_id: String,
}

impl Snapshot {
    pub fn new(
        last_included_index: u64,
        last_included_term: u64,
        data: Vec<u8>,
        cluster_id: &str,
    ) -> Self {
        Self {
            last_included_index,
            last_included_term,
            data,
            created_at: Utc::now(),
            cluster_id: cluster_id.to_string(),
        }
    }

    /// Size of the raw snapshot data in bytes.
    pub fn size_bytes(&self) -> usize {
        self.data.len()
    }
}

/// Manages the lifecycle of in-memory snapshots for a Raft cluster.
pub struct SnapshotManager {
    snapshots: Arc<RwLock<Vec<Snapshot>>>,
    max_retained: usize,
}

impl SnapshotManager {
    /// Create a new manager that keeps at most `max_retained` snapshots.
    pub fn new(max_retained: usize) -> Self {
        Self {
            snapshots: Arc::new(RwLock::new(Vec::new())),
            max_retained,
        }
    }

    /// Store a snapshot.  Snapshots are kept sorted by `last_included_index`.
    pub async fn store(&self, snapshot: Snapshot) {
        let mut snaps = self.snapshots.write().await;
        info!(
            cluster_id = %snapshot.cluster_id,
            last_included_index = snapshot.last_included_index,
            "storing snapshot",
        );
        snaps.push(snapshot);
        snaps.sort_by_key(|s| s.last_included_index);
    }

    /// Return the most recent snapshot, if any.
    pub async fn latest(&self) -> Option<Snapshot> {
        let snaps = self.snapshots.read().await;
        snaps.last().cloned()
    }

    /// List all retained snapshots (oldest first).
    pub async fn list(&self) -> Vec<Snapshot> {
        self.snapshots.read().await.clone()
    }

    /// Remove snapshots beyond `max_retained` (oldest discarded first).
    /// Returns how many were removed.
    pub async fn prune(&self) -> usize {
        let mut snaps = self.snapshots.write().await;
        let len = snaps.len();
        if len <= self.max_retained {
            return 0;
        }
        let to_remove = len - self.max_retained;
        snaps.drain(..to_remove);
        info!(removed = to_remove, retained = self.max_retained, "pruned old snapshots");
        to_remove
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(index: u64, data: &[u8]) -> Snapshot {
        Snapshot::new(index, 1, data.to_vec(), "test-cluster")
    }

    #[tokio::test]
    async fn test_snapshot_store_and_retrieve() {
        let manager = SnapshotManager::new(5);

        let snap = make_snapshot(10, b"state-at-10");
        manager.store(snap.clone()).await;

        let latest = manager.latest().await.expect("should have a snapshot");
        assert_eq!(latest.last_included_index, 10);
        assert_eq!(latest.data, b"state-at-10");
        assert_eq!(latest.cluster_id, "test-cluster");
    }

    #[tokio::test]
    async fn test_snapshot_size_bytes() {
        let snap = Snapshot::new(5, 1, vec![0u8; 1024], "c1");
        assert_eq!(snap.size_bytes(), 1024);
    }

    #[tokio::test]
    async fn test_log_compaction_after_snapshot() {
        // After taking a snapshot at index N, entries <= N should be discardable.
        // Here we verify the manager keeps the snapshot correctly so callers can
        // use last_included_index for compaction.
        let manager = SnapshotManager::new(5);

        manager.store(make_snapshot(100, b"compacted")).await;
        manager.store(make_snapshot(200, b"compacted-2")).await;

        let snaps = manager.list().await;
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].last_included_index, 100);
        assert_eq!(snaps[1].last_included_index, 200);

        let latest = manager.latest().await.unwrap();
        assert_eq!(latest.last_included_index, 200);
    }

    #[tokio::test]
    async fn test_snapshot_pruning() {
        let manager = SnapshotManager::new(3);

        for i in 1..=6u64 {
            manager.store(make_snapshot(i * 10, b"data")).await;
        }

        assert_eq!(manager.list().await.len(), 6);

        let removed = manager.prune().await;
        assert_eq!(removed, 3);

        let remaining = manager.list().await;
        assert_eq!(remaining.len(), 3);
        // The three newest should remain.
        assert_eq!(remaining[0].last_included_index, 40);
        assert_eq!(remaining[1].last_included_index, 50);
        assert_eq!(remaining[2].last_included_index, 60);
    }

    #[tokio::test]
    async fn test_snapshot_ordering() {
        let manager = SnapshotManager::new(10);
        // Insert out of order.
        manager.store(make_snapshot(30, b"c")).await;
        manager.store(make_snapshot(10, b"a")).await;
        manager.store(make_snapshot(20, b"b")).await;

        let snaps = manager.list().await;
        let indices: Vec<u64> = snaps.iter().map(|s| s.last_included_index).collect();
        assert_eq!(indices, vec![10, 20, 30]);
    }
}
