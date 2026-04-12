//! Write-Ahead Log (WAL) for crash recovery.
//!
//! Each entry is length-prefixed (4 bytes BE) + JSON payload.
//! On startup, all entries are replayed to reconstruct state.

use crate::error::{StoreError, StoreResult};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use tracing::{debug, warn};

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
        granted_at: i64, // unix timestamp
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

pub struct WalWriter {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

impl WalWriter {
    pub fn open(dir: &Path) -> StoreResult<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("store.wal");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub async fn append(&self, entry: &WalEntry) -> StoreResult<()> {
        let data = serde_json::to_vec(entry)?;
        let len = (data.len() as u32).to_be_bytes();
        let mut file = self.file.lock().await;
        file.write_all(&len)?;
        file.write_all(&data)?;
        file.flush()?;
        debug!("WAL append: {} bytes", data.len());
        Ok(())
    }

    /// Sync to disk (fsync).
    pub async fn sync(&self) -> StoreResult<()> {
        let file = self.file.lock().await;
        file.sync_data()?;
        Ok(())
    }
}

/// Read all WAL entries from the log file for replay on startup.
pub fn read_wal(dir: &Path) -> StoreResult<Vec<WalEntry>> {
    let path = dir.join("store.wal");
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut file = std::fs::File::open(&path)?;
    let mut entries = Vec::new();
    let mut len_buf = [0u8; 4];

    loop {
        match file.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(StoreError::Io(e)),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut data = vec![0u8; len];
        match file.read_exact(&mut data) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                warn!("WAL truncated entry, stopping replay");
                break;
            }
            Err(e) => return Err(StoreError::Io(e)),
        }
        match serde_json::from_slice::<WalEntry>(&data) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                warn!("WAL corrupt entry (skipping): {e}");
            }
        }
    }

    Ok(entries)
}

/// Compact the WAL by rewriting only current-state entries.
/// Takes a snapshot serialized as a series of WalEntry items.
pub async fn compact_wal(dir: &Path, snapshot: Vec<WalEntry>) -> StoreResult<()> {
    let path = dir.join("store.wal");
    let tmp = dir.join("store.wal.tmp");

    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        for entry in &snapshot {
            let data = serde_json::to_vec(entry)?;
            let len = (data.len() as u32).to_be_bytes();
            file.write_all(&len)?;
            file.write_all(&data)?;
        }
        file.sync_data()?;
    }

    std::fs::rename(&tmp, &path)?;
    Ok(())
}
