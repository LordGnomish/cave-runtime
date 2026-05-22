// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shipper variants — `pkg/storage/stores/shipper/{boltdb,tsdb}`.
//!
//! Upstream Loki ships index files to object storage on a periodic
//! background loop ("shipper sync"). Each ingester writes table files
//! into a local cache; the shipper observes new tables, uploads them
//! under a per-tenant + per-table key, and downloads remote tables on
//! demand for query-time merges. The boltdb variant uses LMDB-style
//! single-file tables; the tsdb variant uses Prometheus-style block
//! directories.
//!
//! This implementation ports the upstream's *operational shape*:
//! pending-set + uploaded-set + last-modified bookkeeping, with an
//! abstract `ObjectStore` trait so cave-logs can drive it with an
//! in-memory store today and a real S3-shaped backend later.
//!
//! Mapped surfaces:
//! * `pkg/storage/stores/shipper/{boltdb,tsdb}/uploads/table.go`
//! * `pkg/storage/stores/shipper/{boltdb,tsdb}/downloads/table_manager.go`
//! * `pkg/storage/stores/shipper/util/util.go` (table-name helpers)

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Index variant — selects the on-disk table naming convention upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipperVariant {
    BoltDb,
    Tsdb,
}

impl ShipperVariant {
    pub fn table_prefix(self) -> &'static str {
        match self {
            ShipperVariant::BoltDb => "boltdb_index",
            ShipperVariant::Tsdb => "tsdb_index",
        }
    }
}

/// Object-store contract — mirrors the chunk-store interface upstream uses.
pub trait ObjectStore: Send + Sync {
    fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), String>;
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn list(&self, prefix: &str) -> Vec<String>;
}

/// In-memory `ObjectStore` for tests + single-process runtimes.
#[derive(Default)]
pub struct MemObjectStore {
    inner: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl ObjectStore for MemObjectStore {
    fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.inner.lock().unwrap().insert(key.to_string(), bytes);
        Ok(())
    }
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().get(key).cloned()
    }
    fn list(&self, prefix: &str) -> Vec<String> {
        self.inner
            .lock()
            .unwrap()
            .range(prefix.to_string()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// Local-cache entry awaiting (or post) upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Pending,
    Uploaded,
    Downloaded,
}

#[derive(Debug, Clone)]
pub struct TableMeta {
    pub name: String,
    pub tenant: String,
    pub bytes: usize,
    pub state: SyncState,
}

/// Shipper — keeps an in-memory ledger of tables and drives a sync loop.
pub struct Shipper<S: ObjectStore> {
    pub variant: ShipperVariant,
    store: S,
    local: Mutex<HashMap<String, TableMeta>>,
    local_bytes: Mutex<HashMap<String, Vec<u8>>>,
}

impl<S: ObjectStore> Shipper<S> {
    pub fn new(variant: ShipperVariant, store: S) -> Self {
        Self {
            variant,
            store,
            local: Mutex::new(HashMap::new()),
            local_bytes: Mutex::new(HashMap::new()),
        }
    }

    /// Upstream `PeriodicTable.Name(tenant, table_id)`: `<prefix>/<tenant>/<id>`.
    pub fn table_key(&self, tenant: &str, table_id: &str) -> String {
        format!("{}/{}/{}", self.variant.table_prefix(), tenant, table_id)
    }

    /// Stage a freshly-written local table for upload.
    pub fn enqueue(&self, tenant: &str, table_id: &str, bytes: Vec<u8>) {
        let key = self.table_key(tenant, table_id);
        let meta = TableMeta {
            name: table_id.to_string(),
            tenant: tenant.to_string(),
            bytes: bytes.len(),
            state: SyncState::Pending,
        };
        self.local.lock().unwrap().insert(key.clone(), meta);
        self.local_bytes.lock().unwrap().insert(key, bytes);
    }

    /// Run one upload pass — moves every Pending entry to Uploaded.
    /// Returns the count of tables uploaded.
    pub fn sync_uploads(&self) -> usize {
        let mut count = 0;
        let mut local = self.local.lock().unwrap();
        let bytes_map = self.local_bytes.lock().unwrap();
        for (key, meta) in local.iter_mut() {
            if meta.state == SyncState::Pending {
                if let Some(b) = bytes_map.get(key) {
                    if self.store.put(key, b.clone()).is_ok() {
                        meta.state = SyncState::Uploaded;
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Pull a remote table into the local cache.
    pub fn download(&self, tenant: &str, table_id: &str) -> Option<Vec<u8>> {
        let key = self.table_key(tenant, table_id);
        let bytes = self.store.get(&key)?;
        let meta = TableMeta {
            name: table_id.to_string(),
            tenant: tenant.to_string(),
            bytes: bytes.len(),
            state: SyncState::Downloaded,
        };
        self.local.lock().unwrap().insert(key.clone(), meta);
        self.local_bytes
            .lock()
            .unwrap()
            .insert(key.clone(), bytes.clone());
        Some(bytes)
    }

    /// Upstream `TableManager.ListTables(tenant)`.
    pub fn list_tables(&self, tenant: &str) -> Vec<String> {
        let prefix = format!("{}/{}/", self.variant.table_prefix(), tenant);
        let mut tables: Vec<String> = self
            .store
            .list(&prefix)
            .into_iter()
            .map(|k| k[prefix.len()..].to_string())
            .collect();
        tables.sort();
        tables
    }

    pub fn pending_count(&self) -> usize {
        self.local
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.state == SyncState::Pending)
            .count()
    }

    pub fn uploaded_count(&self) -> usize {
        self.local
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.state == SyncState::Uploaded)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boltdb_table_key_layout() {
        let s = Shipper::new(ShipperVariant::BoltDb, MemObjectStore::default());
        assert_eq!(s.table_key("t1", "20260519"), "boltdb_index/t1/20260519");
    }

    #[test]
    fn tsdb_table_key_layout() {
        let s = Shipper::new(ShipperVariant::Tsdb, MemObjectStore::default());
        assert_eq!(s.table_key("t1", "20260519"), "tsdb_index/t1/20260519");
    }

    #[test]
    fn enqueue_then_sync_uploads_changes_state() {
        let s = Shipper::new(ShipperVariant::Tsdb, MemObjectStore::default());
        s.enqueue("t1", "20260519", b"index-bytes".to_vec());
        assert_eq!(s.pending_count(), 1);
        let n = s.sync_uploads();
        assert_eq!(n, 1);
        assert_eq!(s.uploaded_count(), 1);
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn download_populates_local_cache() {
        let store = MemObjectStore::default();
        store.put("tsdb_index/t1/20260519", b"x".to_vec()).unwrap();
        let s = Shipper::new(ShipperVariant::Tsdb, store);
        let got = s.download("t1", "20260519");
        assert_eq!(got.as_deref(), Some(b"x".as_ref()));
    }

    #[test]
    fn list_tables_filters_by_tenant_prefix() {
        let store = MemObjectStore::default();
        store.put("tsdb_index/t1/a", b"".to_vec()).unwrap();
        store.put("tsdb_index/t1/b", b"".to_vec()).unwrap();
        store.put("tsdb_index/t2/z", b"".to_vec()).unwrap();
        let s = Shipper::new(ShipperVariant::Tsdb, store);
        assert_eq!(s.list_tables("t1"), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(s.list_tables("t2"), vec!["z".to_string()]);
    }

    #[test]
    fn sync_uploads_is_idempotent() {
        let s = Shipper::new(ShipperVariant::Tsdb, MemObjectStore::default());
        s.enqueue("t", "a", vec![0]);
        assert_eq!(s.sync_uploads(), 1);
        assert_eq!(s.sync_uploads(), 0, "no new pending entries → 0 uploads");
    }
}
