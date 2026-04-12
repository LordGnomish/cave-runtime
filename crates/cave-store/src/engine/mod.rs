//! Storage engine: WAL + MVCC, with crash recovery via WAL replay.

pub mod mvcc;
pub mod wal;
#[cfg(test)]
mod tests;

use std::{path::PathBuf, sync::Arc};

use parking_lot::{Mutex, RwLock};
use tracing::{info, warn};

use crate::error::Result;
pub use mvcc::{Compare, CompareResult, CompareTarget, KeyValue, LeaseInfo, MvccStore, TxnOp, TxnResult, WatchEvent, WatchEventType};
pub use wal::{WalEntry, WalFile, WalOp};

pub struct StorageEngine {
    pub mvcc: Arc<RwLock<MvccStore>>,
    wal: Arc<Mutex<WalFile>>,
}

impl StorageEngine {
    /// Create or open a storage engine, replaying any existing WAL.
    pub fn open(data_dir: impl Into<PathBuf>, wal_sync: bool) -> std::io::Result<Self> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)?;

        let wal_path = data_dir.join("wal.log");
        let entries = WalFile::replay(&wal_path)?;

        let mut mvcc = MvccStore::new();
        let replayed = entries.len();
        for entry in entries {
            apply_wal_entry(&mut mvcc, &entry);
        }
        if replayed > 0 {
            info!("WAL replay: {replayed} entries, revision={}", mvcc.current_revision());
        }

        let wal = WalFile::open(&wal_path, wal_sync)?;
        Ok(Self {
            mvcc: Arc::new(RwLock::new(mvcc)),
            wal: Arc::new(Mutex::new(wal)),
        })
    }

    // ── WAL-backed put ────────────────────────────────────────────────────────

    pub fn put(&self, key: Vec<u8>, value: Vec<u8>, lease_id: i64, prev_kv: bool) -> Result<(i64, Option<KeyValue>)> {
        let mut mvcc = self.mvcc.write();
        let rev = mvcc.current_revision() + 1;
        let entry = WalEntry::new(rev, WalOp::Put { key: key.clone(), value: value.clone(), lease_id });
        self.wal.lock().append(&entry)?;
        let (rev, prev) = mvcc.put(key, value, lease_id, prev_kv);
        Ok((rev, prev))
    }

    pub fn delete(&self, key: Vec<u8>, prev_kv: bool) -> Result<(i64, Option<KeyValue>)> {
        let mut mvcc = self.mvcc.write();
        let rev = mvcc.current_revision() + 1;
        let entry = WalEntry::new(rev, WalOp::Delete { key: key.clone() });
        self.wal.lock().append(&entry)?;
        let (rev, prev) = mvcc.delete(&key, prev_kv);
        Ok((rev, prev))
    }

    pub fn delete_range(&self, key: Vec<u8>, range_end: Vec<u8>, prev_kv: bool) -> Result<(i64, Vec<KeyValue>)> {
        let mut mvcc = self.mvcc.write();
        let rev = mvcc.current_revision() + 1;
        let entry = WalEntry::new(rev, WalOp::Delete { key: key.clone() });
        self.wal.lock().append(&entry)?;
        let (rev, prev) = mvcc.delete_range(&key, &range_end, prev_kv);
        Ok((rev, prev))
    }

    pub fn lease_grant(&self, id: i64, ttl: i64) -> Result<i64> {
        let granted_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut mvcc = self.mvcc.write();
        let rev = mvcc.current_revision() + 1;
        let entry = WalEntry::new(rev, WalOp::LeaseGrant { id, ttl, granted_at });
        self.wal.lock().append(&entry)?;
        Ok(mvcc.lease_grant(id, ttl))
    }

    pub fn lease_revoke(&self, id: i64) -> Result<Vec<Vec<u8>>> {
        let mut mvcc = self.mvcc.write();
        let rev = mvcc.current_revision() + 1;
        let entry = WalEntry::new(rev, WalOp::LeaseRevoke { id });
        self.wal.lock().append(&entry)?;
        mvcc.lease_revoke(id)
    }
}

fn apply_wal_entry(mvcc: &mut MvccStore, entry: &WalEntry) {
    match &entry.op {
        WalOp::Put { key, value, lease_id } => {
            mvcc.put(key.clone(), value.clone(), *lease_id, false);
        }
        WalOp::Delete { key } => {
            mvcc.delete(key, false);
        }
        WalOp::LeaseGrant { id, ttl, granted_at: _ } => {
            mvcc.lease_grant(*id, *ttl);
        }
        WalOp::LeaseRevoke { id } => {
            if let Err(e) = mvcc.lease_revoke(*id) {
                warn!("WAL replay: lease_revoke failed: {e}");
            }
        }
        WalOp::Compact { revision } => {
            if let Err(e) = mvcc.compact(*revision) {
                warn!("WAL replay: compact failed: {e}");
            }
        }
    }
}
