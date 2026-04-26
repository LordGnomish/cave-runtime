//! Write-Ahead Log (WAL) for crash recovery — delegates to cave-core::wal.
//!
//! `WalEntry` owns the S3 + etcd domain op shapes.
//! The append/replay machinery is provided by `cave_core::wal::{AppendLog, replay}`.

use cave_core::wal::{AppendLog, WalError};
use crate::error::{StoreError, StoreResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use tracing::debug;

// ── Domain entry type ─────────────────────────────────────────────────────────

/// A single WAL entry — covers both etcd KV ops and S3 object ops.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WalEntry {
    // etcd KV
    KvPut {
        revision: i64,
        key: Vec<u8>,
        value: Vec<u8>,
        lease_id: i64,
    },
    KvDelete {
        revision: i64,
        key: Vec<u8>,
    },
    KvCompact {
        revision: i64,
    },
    // etcd Lease
    LeaseGrant {
        lease_id: i64,
        ttl_secs: i64,
        granted_at: i64,
    },
    LeaseRevoke {
        lease_id: i64,
    },
    LeaseKeepAlive {
        lease_id: i64,
        renewed_at: i64,
    },
    // S3 Bucket
    BucketCreate {
        name: String,
        region: String,
        owner: String,
    },
    BucketDelete {
        name: String,
    },
    BucketVersioning {
        name: String,
        state: String,
    },
    BucketPolicy {
        name: String,
        policy_json: String,
    },
    BucketLifecycle {
        name: String,
        rules_json: String,
    },
    BucketNotification {
        name: String,
        config_json: String,
    },
    // S3 Object
    ObjectPut {
        bucket: String,
        key: String,
        version_id: Option<String>,
        etag: String,
        size: u64,
        content_type: String,
        metadata_json: String,
        storage_path: String,
        lease_id: Option<String>,
    },
    ObjectDelete {
        bucket: String,
        key: String,
        version_id: Option<String>,
        delete_marker: bool,
    },
    ObjectTagging {
        bucket: String,
        key: String,
        version_id: Option<String>,
        tags_json: String,
    },
    // S3 Multipart
    MultipartInit {
        upload_id: String,
        bucket: String,
        key: String,
        metadata_json: String,
    },
    MultipartPart {
        upload_id: String,
        part_number: u32,
        etag: String,
        size: u64,
        storage_path: String,
    },
    MultipartComplete {
        upload_id: String,
        final_etag: String,
        final_path: String,
        version_id: Option<String>,
    },
    MultipartAbort {
        upload_id: String,
    },
}

// ── Async writer ──────────────────────────────────────────────────────────────

/// Async, Arc-shareable wrapper around `cave_core::wal::AppendLog`.
///
/// All mutation is serialised through a `tokio::sync::Mutex` so multiple
/// async tasks can share a single writer.
pub struct WalWriter {
    path: PathBuf,
    log: Mutex<AppendLog>,
}

impl WalWriter {
    /// Open (or create) the WAL file at `<dir>/store.wal`.
    pub fn open(dir: &Path) -> StoreResult<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("store.wal");
        let log = AppendLog::open(&path).map_err(|e| StoreError::Io(map_wal_err(e)))?;
        Ok(Self { path, log: Mutex::new(log) })
    }

    /// Append one entry to the WAL.
    pub async fn append(&self, entry: &WalEntry) -> StoreResult<()> {
        self.log
            .lock()
            .await
            .append(entry)
            .map_err(|e| StoreError::Io(map_wal_err(e)))?;
        debug!("WAL append: {:?}", std::mem::discriminant(entry));
        Ok(())
    }

    /// fsync for full durability.
    pub async fn sync(&self) -> StoreResult<()> {
        self.log
            .lock()
            .await
            .sync()
            .map_err(|e| StoreError::Io(map_wal_err(e)))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ── Replay ────────────────────────────────────────────────────────────────────

/// Read all WAL entries from `<dir>/store.wal` for replay on startup.
pub fn read_wal(dir: &Path) -> StoreResult<Vec<WalEntry>> {
    let path = dir.join("store.wal");
    let mut entries = Vec::new();
    cave_core::wal::replay::<WalEntry, _>(&path, |e| entries.push(e))
        .map_err(|e| StoreError::Io(map_wal_err(e)))?;
    Ok(entries)
}

// ── Compaction ────────────────────────────────────────────────────────────────

/// Compact the WAL by rewriting only current-state entries (atomic rename).
pub async fn compact_wal(dir: &Path, snapshot: Vec<WalEntry>) -> StoreResult<()> {
    let path = dir.join("store.wal");
    let tmp = dir.join("store.wal.tmp");

    // Remove tmp if left over from a previous crashed compaction.
    let _ = std::fs::remove_file(&tmp);

    {
        let mut log =
            AppendLog::open(&tmp).map_err(|e| StoreError::Io(map_wal_err(e)))?;
        for entry in &snapshot {
            log.append(entry).map_err(|e| StoreError::Io(map_wal_err(e)))?;
        }
        log.sync().map_err(|e| StoreError::Io(map_wal_err(e)))?;
    }

    std::fs::rename(&tmp, &path)?;
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn map_wal_err(e: WalError) -> std::io::Error {
    match e {
        WalError::Io(io) => io,
        WalError::Serialize(s) => {
            std::io::Error::new(std::io::ErrorKind::InvalidData, s.to_string())
        }
    }
}
