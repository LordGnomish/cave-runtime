// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Snapshot storage — persists and retrieves snapshots for crash recovery and peer catch-up.

use std::path::{Path, PathBuf};

use tokio::fs;
use tracing::{info, warn};

use crate::error::{HaError, HaResult};
use crate::raft::snapshot::Snapshot;
use crate::raft::types::{LogIndex, SnapshotMeta};

/// Manages snapshot files in a directory.
///
/// File naming: `snapshot-{index}.snap`
/// A `snapshot.meta` symlink (or copy) points to the latest.
pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub async fn open(dir: impl AsRef<Path>) -> HaResult<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).await?;
        Ok(Self { dir })
    }

    /// Persist a snapshot. Atomically compatible with the previous snapshot.
    pub async fn save(&self, snapshot: &Snapshot) -> HaResult<()> {
        let tmp_path = self
            .dir
            .join(format!("snapshot-{}.tmp", snapshot.meta.index));
        let final_path = self
            .dir
            .join(format!("snapshot-{}.snap", snapshot.meta.index));

        // Write metadata + data as a single JSON envelope.
        let envelope = SnapshotEnvelope {
            meta: snapshot.meta.clone(),
            data: snapshot.data.clone(),
        };
        let bytes = serde_json::to_vec(&envelope)?;

        fs::write(&tmp_path, &bytes).await?;
        fs::rename(&tmp_path, &final_path).await?;

        info!(
            index = snapshot.meta.index,
            term = snapshot.meta.term,
            size = bytes.len(),
            "snapshot saved"
        );

        // Clean up older snapshots.
        self.cleanup(snapshot.meta.index).await;
        Ok(())
    }

    /// Load the latest snapshot, or None if none exists.
    pub async fn load_latest(&self) -> HaResult<Option<Snapshot>> {
        let latest_index = self.find_latest_index().await?;
        match latest_index {
            None => Ok(None),
            Some(index) => self.load(index).await.map(Some),
        }
    }

    /// Load a specific snapshot by log index.
    pub async fn load(&self, index: LogIndex) -> HaResult<Snapshot> {
        let path = self.dir.join(format!("snapshot-{index}.snap"));
        let bytes = fs::read(&path)
            .await
            .map_err(|e| HaError::Snapshot(format!("read snapshot-{index}: {e}")))?;
        let envelope: SnapshotEnvelope = serde_json::from_slice(&bytes)?;
        Ok(Snapshot {
            meta: envelope.meta,
            data: envelope.data,
        })
    }

    /// Find the index of the most recent snapshot file.
    async fn find_latest_index(&self) -> HaResult<Option<LogIndex>> {
        let mut entries = fs::read_dir(&self.dir).await?;
        let mut latest: Option<LogIndex> = None;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(idx) = parse_snap_index(&name) {
                latest = Some(latest.map_or(idx, |l: LogIndex| l.max(idx)));
            }
        }
        Ok(latest)
    }

    /// Delete snapshots older than `keep_index`.
    async fn cleanup(&self, keep_index: LogIndex) {
        let mut entries = match fs::read_dir(&self.dir).await {
            Ok(e) => e,
            Err(_) => return,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(idx) = parse_snap_index(&name) {
                if idx < keep_index {
                    if let Err(e) = fs::remove_file(entry.path()).await {
                        warn!("cleanup snapshot {idx}: {e}");
                    }
                }
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn parse_snap_index(name: &str) -> Option<LogIndex> {
    let name = name.strip_prefix("snapshot-")?.strip_suffix(".snap")?;
    name.parse().ok()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotEnvelope {
    meta: SnapshotMeta,
    data: Vec<u8>,
}
