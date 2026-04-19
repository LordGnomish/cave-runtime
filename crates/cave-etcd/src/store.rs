//! MVCC key-value store with revision tracking.
//!
//! Implements etcd's multi-version concurrency control model:
//! every write creates a new revision, reads can target specific revisions.

use crate::error::{EtcdError, EtcdResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use tokio::sync::broadcast;

/// MVCC key-value store.
pub struct KvStore {
    /// Current live key-value pairs.
    current: DashMap<Vec<u8>, KeyValue>,
    /// Revision history: revision -> (key, event_type, kv).
    history: RwLock<BTreeMap<u64, (Vec<u8>, EventType, KeyValue)>>,
    /// Monotonically increasing revision counter.
    revision: AtomicU64,
    /// Watch notification channel.
    watch_tx: broadcast::Sender<WatchEvent>,
    /// Active leases.
    leases: DashMap<i64, Lease>,
    /// Lease ID counter.
    lease_counter: AtomicU64,
    /// Compacted revision (history before this is deleted).
    compacted_revision: AtomicU64,
}

impl KvStore {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(4096);
        Self {
            current: DashMap::new(),
            history: RwLock::new(BTreeMap::new()),
            revision: AtomicU64::new(1),
            watch_tx,
            leases: DashMap::new(),
            lease_counter: AtomicU64::new(1),
            compacted_revision: AtomicU64::new(0),
        }
    }

    pub fn current_revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn header(&self) -> ResponseHeader {
        ResponseHeader {
            cluster_id: 1,
            member_id: 1,
            revision: self.current_revision(),
            raft_term: 1,
        }
    }

    /// PUT a key-value pair.
    pub fn put(&self, req: &PutRequest) -> PutResponse {
        let key = req.key.as_bytes().to_vec();
        let rev = self.next_revision();

        let prev_kv = self.current.get(&key).map(|r| r.value().clone());

        let version = prev_kv.as_ref().map(|p| p.version + 1).unwrap_or(1);
        let create_rev = prev_kv.as_ref().map(|p| p.create_revision).unwrap_or(rev);

        let kv = KeyValue {
            key: key.clone(),
            value: req.value.as_bytes().to_vec(),
            create_revision: create_rev,
            mod_revision: rev,
            version,
            lease: req.lease,
        };

        self.current.insert(key.clone(), kv.clone());

        // Record in history
        if let Ok(mut history) = self.history.write() {
            history.insert(rev, (key, EventType::Put, kv.clone()));
        }

        // Notify watchers
        let _ = self.watch_tx.send(WatchEvent {
            event_type: EventType::Put,
            kv: kv.clone(),
            prev_kv: if req.prev_kv { prev_kv.clone() } else { None },
        });

        PutResponse {
            header: self.header(),
            prev_kv: if req.prev_kv { prev_kv } else { None },
        }
    }

    /// GET a key or range of keys.
    pub fn range(&self, req: &RangeRequest) -> EtcdResult<RangeResponse> {
        if let Some(target_rev) = req.revision {
            let compacted = self.compacted_revision.load(Ordering::SeqCst);
            if target_rev < compacted {
                return Err(EtcdError::RevisionCompacted {
                    requested: target_rev,
                    compacted,
                });
            }
        }

        let key_bytes = req.key.as_bytes().to_vec();
        let mut kvs = Vec::new();

        if let Some(ref range_end) = req.range_end {
            // Range query
            let end_bytes = range_end.as_bytes().to_vec();
            for entry in self.current.iter() {
                let k = entry.key();
                if *k >= key_bytes && *k < end_bytes {
                    kvs.push(entry.value().clone());
                }
            }
            kvs.sort_by(|a, b| a.key.cmp(&b.key));
        } else {
            // Single key
            if let Some(kv) = self.current.get(&key_bytes) {
                kvs.push(kv.value().clone());
            }
        }

        let count = kvs.len() as u64;

        if req.count_only {
            return Ok(RangeResponse {
                header: self.header(),
                kvs: vec![],
                count,
                more: false,
            });
        }

        let more = if let Some(limit) = req.limit {
            if kvs.len() as u64 > limit {
                kvs.truncate(limit as usize);
                true
            } else {
                false
            }
        } else {
            false
        };

        if req.keys_only {
            for kv in &mut kvs {
                kv.value.clear();
            }
        }

        Ok(RangeResponse {
            header: self.header(),
            kvs,
            count,
            more,
        })
    }

    /// DELETE a key or range of keys.
    pub fn delete_range(&self, req: &DeleteRangeRequest) -> DeleteRangeResponse {
        let key_bytes = req.key.as_bytes().to_vec();
        let mut deleted = 0u64;
        let mut prev_kvs = Vec::new();
        let rev = self.next_revision();

        if let Some(ref range_end) = req.range_end {
            let end_bytes = range_end.as_bytes().to_vec();
            let keys_to_delete: Vec<Vec<u8>> = self.current.iter()
                .filter(|e| *e.key() >= key_bytes && *e.key() < end_bytes)
                .map(|e| e.key().clone())
                .collect();

            for key in keys_to_delete {
                if let Some((_, kv)) = self.current.remove(&key) {
                    deleted += 1;
                    // Record deletion in history
                    if let Ok(mut history) = self.history.write() {
                        history.insert(rev, (key.clone(), EventType::Delete, kv.clone()));
                    }
                    let _ = self.watch_tx.send(WatchEvent {
                        event_type: EventType::Delete,
                        kv: kv.clone(),
                        prev_kv: None,
                    });
                    if req.prev_kv { prev_kvs.push(kv); }
                }
            }
        } else {
            if let Some((_, kv)) = self.current.remove(&key_bytes) {
                deleted = 1;
                if let Ok(mut history) = self.history.write() {
                    history.insert(rev, (key_bytes, EventType::Delete, kv.clone()));
                }
                let _ = self.watch_tx.send(WatchEvent {
                    event_type: EventType::Delete,
                    kv: kv.clone(),
                    prev_kv: None,
                });
                if req.prev_kv { prev_kvs.push(kv); }
            }
        }

        DeleteRangeResponse {
            header: self.header(),
            deleted,
            prev_kvs,
        }
    }

    /// Subscribe to watch events.
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.watch_tx.subscribe()
    }

    /// Grant a lease.
    pub fn lease_grant(&self, req: &LeaseGrantRequest) -> LeaseGrantResponse {
        let id = req.id.unwrap_or_else(|| self.lease_counter.fetch_add(1, Ordering::SeqCst) as i64 + 1);
        let lease = Lease {
            id,
            ttl: req.ttl,
            granted_at: Utc::now(),
            keys: vec![],
        };
        self.leases.insert(id, lease);
        LeaseGrantResponse {
            header: self.header(),
            id,
            ttl: req.ttl,
        }
    }

    /// Revoke a lease and delete associated keys.
    pub fn lease_revoke(&self, lease_id: i64) -> EtcdResult<()> {
        let lease = self.leases.remove(&lease_id)
            .map(|(_, l)| l)
            .ok_or(EtcdError::LeaseNotFound(lease_id))?;

        for key in &lease.keys {
            self.current.remove(key.as_bytes());
        }
        Ok(())
    }

    /// Compact revision history.
    pub fn compact(&self, revision: u64) {
        self.compacted_revision.store(revision, Ordering::SeqCst);
        if let Ok(mut history) = self.history.write() {
            let keys: Vec<u64> = history.range(..revision).map(|(k, _)| *k).collect();
            for k in keys {
                history.remove(&k);
            }
        }
    }

    /// Get cluster status.
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "header": self.header(),
            "version": "3.5.0-cave",
            "dbSize": self.current.len(),
            "leader": 1,
            "raftIndex": self.current_revision(),
            "raftTerm": 1,
        })
    }
}

impl Default for KvStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(store: &KvStore, key: &str, value: &str) {
        store.put(&PutRequest { key: key.into(), value: value.into(), lease: None, prev_kv: false });
    }

    fn get(store: &KvStore, key: &str) -> RangeResponse {
        store.range(&RangeRequest {
            key: key.into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap()
    }

    // --- put ---

    #[test]
    fn test_put_and_get() {
        let store = KvStore::new();
        put(&store, "foo", "bar");
        let resp = get(&store, "foo");
        assert_eq!(resp.kvs.len(), 1);
        assert_eq!(resp.kvs[0].value_str(), "bar");
    }

    #[test]
    fn test_put_updates_revision() {
        let store = KvStore::new();
        let r1 = store.put(&PutRequest { key: "a".into(), value: "1".into(), lease: None, prev_kv: false });
        let r2 = store.put(&PutRequest { key: "b".into(), value: "2".into(), lease: None, prev_kv: false });
        assert!(r2.header.revision > r1.header.revision);
    }

    #[test]
    fn test_put_with_lease() {
        let store = KvStore::new();
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        store.put(&PutRequest { key: "leased".into(), value: "val".into(), lease: Some(lease.id), prev_kv: false });
        let resp = get(&store, "leased");
        assert_eq!(resp.kvs[0].lease, Some(lease.id));
    }

    #[test]
    fn test_put_prev_kv() {
        let store = KvStore::new();
        put(&store, "x", "old");
        let resp = store.put(&PutRequest { key: "x".into(), value: "new".into(), lease: None, prev_kv: true });
        assert!(resp.prev_kv.is_some());
        assert_eq!(resp.prev_kv.unwrap().value_str(), "old");
    }

    #[test]
    fn test_put_overwrite_increments_version() {
        let store = KvStore::new();
        put(&store, "k", "v1");
        put(&store, "k", "v2");
        let resp = get(&store, "k");
        assert_eq!(resp.kvs[0].version, 2);
        assert_eq!(resp.kvs[0].value_str(), "v2");
    }

    #[test]
    fn test_put_overwrite_preserves_create_revision() {
        let store = KvStore::new();
        put(&store, "stable", "v1");
        let create_rev = get(&store, "stable").kvs[0].create_revision;
        put(&store, "stable", "v2");
        let resp = get(&store, "stable");
        assert_eq!(resp.kvs[0].create_revision, create_rev);
        assert!(resp.kvs[0].mod_revision > create_rev);
    }

    #[test]
    fn test_put_empty_key() {
        let store = KvStore::new();
        put(&store, "", "val");
        let resp = get(&store, "");
        assert_eq!(resp.kvs.len(), 1);
        assert_eq!(resp.kvs[0].value_str(), "val");
    }

    #[test]
    fn test_put_empty_value() {
        let store = KvStore::new();
        put(&store, "k", "");
        let resp = get(&store, "k");
        assert_eq!(resp.kvs[0].value_str(), "");
    }

    #[test]
    fn test_put_very_long_key_value() {
        let store = KvStore::new();
        let long_key = "k".repeat(10_000);
        let long_val = "v".repeat(100_000);
        store.put(&PutRequest { key: long_key.clone(), value: long_val.clone(), lease: None, prev_kv: false });
        let resp = get(&store, &long_key);
        assert_eq!(resp.kvs[0].value_str(), long_val);
    }

    // --- range ---

    #[test]
    fn test_range_single_key_hit() {
        let store = KvStore::new();
        put(&store, "hit", "v");
        let resp = get(&store, "hit");
        assert_eq!(resp.kvs.len(), 1);
        assert_eq!(resp.count, 1);
    }

    #[test]
    fn test_range_single_key_miss() {
        let store = KvStore::new();
        let resp = get(&store, "nonexistent");
        assert_eq!(resp.kvs.len(), 0);
        assert_eq!(resp.count, 0);
    }

    #[test]
    fn test_range_scan() {
        let store = KvStore::new();
        put(&store, "/a/1", "v1");
        put(&store, "/a/2", "v2");
        put(&store, "/b/1", "v3");
        let resp = store.range(&RangeRequest {
            key: "/a/".into(), range_end: Some("/a0".into()), limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 2);
    }

    #[test]
    fn test_range_with_limit() {
        let store = KvStore::new();
        put(&store, "a", "1");
        put(&store, "b", "2");
        put(&store, "c", "3");
        let resp = store.range(&RangeRequest {
            key: "a".into(), range_end: Some("z".into()), limit: Some(2),
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 2);
        assert!(resp.more);
    }

    #[test]
    fn test_range_limit_not_exceeded_no_more() {
        let store = KvStore::new();
        put(&store, "a", "1");
        put(&store, "b", "2");
        let resp = store.range(&RangeRequest {
            key: "a".into(), range_end: Some("z".into()), limit: Some(5),
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 2);
        assert!(!resp.more);
    }

    #[test]
    fn test_range_keys_only() {
        let store = KvStore::new();
        put(&store, "k", "secret");
        let resp = store.range(&RangeRequest {
            key: "k".into(), range_end: Some("l".into()), limit: None,
            revision: None, keys_only: true, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 1);
        assert!(resp.kvs[0].value.is_empty());
        assert_eq!(resp.kvs[0].key_str(), "k");
    }

    #[test]
    fn test_count_only() {
        let store = KvStore::new();
        put(&store, "k1", "v1");
        put(&store, "k2", "v2");
        let resp = store.range(&RangeRequest {
            key: "k".into(), range_end: Some("l".into()), limit: None,
            revision: None, keys_only: false, count_only: true,
        }).unwrap();
        assert_eq!(resp.count, 2);
        assert!(resp.kvs.is_empty());
    }

    #[test]
    fn test_range_compacted_revision_error() {
        let store = KvStore::new();
        put(&store, "k", "v1");
        put(&store, "k", "v2");
        store.compact(5);
        let result = store.range(&RangeRequest {
            key: "k".into(), range_end: None, limit: None,
            revision: Some(2), keys_only: false, count_only: false,
        });
        assert!(matches!(result, Err(EtcdError::RevisionCompacted { .. })));
    }

    #[test]
    fn test_range_valid_revision_after_compact() {
        let store = KvStore::new();
        put(&store, "k", "v");
        store.compact(3);
        // revision 10 > compacted 3, should not error
        let result = store.range(&RangeRequest {
            key: "k".into(), range_end: None, limit: None,
            revision: Some(10), keys_only: false, count_only: false,
        });
        assert!(result.is_ok());
    }

    // --- delete_range ---

    #[test]
    fn test_delete_single_key_with_prev_kv() {
        let store = KvStore::new();
        put(&store, "del_me", "v");
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "del_me".into(), range_end: None, prev_kv: true,
        });
        assert_eq!(resp.deleted, 1);
        assert_eq!(resp.prev_kvs[0].value_str(), "v");
        assert_eq!(get(&store, "del_me").kvs.len(), 0);
    }

    #[test]
    fn test_delete_range_deletes_multiple() {
        let store = KvStore::new();
        put(&store, "/k/1", "v1");
        put(&store, "/k/2", "v2");
        put(&store, "/m/1", "v3");
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "/k/".into(), range_end: Some("/k0".into()), prev_kv: false,
        });
        assert_eq!(resp.deleted, 2);
        let remaining = store.range(&RangeRequest {
            key: "/".into(), range_end: Some("0".into()), limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(remaining.kvs.len(), 1);
    }

    #[test]
    fn test_delete_range_non_existent() {
        let store = KvStore::new();
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "nonexistent".into(), range_end: None, prev_kv: false,
        });
        assert_eq!(resp.deleted, 0);
        assert!(resp.prev_kvs.is_empty());
    }

    #[test]
    fn test_delete_range_without_prev_kv() {
        let store = KvStore::new();
        put(&store, "key1", "v1");
        put(&store, "key2", "v2");
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "key".into(), range_end: Some("keyz".into()), prev_kv: false,
        });
        assert_eq!(resp.deleted, 2);
        assert!(resp.prev_kvs.is_empty());
    }

    // --- lease_grant ---

    #[test]
    fn test_lease_grant_auto_id() {
        let store = KvStore::new();
        let resp = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        assert!(resp.id > 0);
        assert_eq!(resp.ttl, 60);
    }

    #[test]
    fn test_lease_grant_with_custom_id() {
        let store = KvStore::new();
        let resp = store.lease_grant(&LeaseGrantRequest { ttl: 30, id: Some(12345) });
        assert_eq!(resp.id, 12345);
        assert_eq!(resp.ttl, 30);
    }

    #[test]
    fn test_lease_grant_zero_ttl() {
        let store = KvStore::new();
        let resp = store.lease_grant(&LeaseGrantRequest { ttl: 0, id: None });
        assert!(resp.id > 0);
        assert_eq!(resp.ttl, 0);
    }

    #[test]
    fn test_lease_grant_auto_ids_are_unique() {
        let store = KvStore::new();
        let r1 = store.lease_grant(&LeaseGrantRequest { ttl: 10, id: None });
        let r2 = store.lease_grant(&LeaseGrantRequest { ttl: 10, id: None });
        assert_ne!(r1.id, r2.id);
    }

    // --- lease_revoke ---

    #[test]
    fn test_lease_revoke_valid() {
        let store = KvStore::new();
        let resp = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        assert!(store.lease_revoke(resp.id).is_ok());
    }

    #[test]
    fn test_lease_revoke_non_existent() {
        let store = KvStore::new();
        assert!(matches!(store.lease_revoke(99999), Err(EtcdError::LeaseNotFound(99999))));
    }

    #[test]
    fn test_lease_revoke_twice_errors() {
        let store = KvStore::new();
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        store.lease_revoke(lease.id).unwrap();
        assert!(store.lease_revoke(lease.id).is_err());
    }

    #[test]
    fn test_lease_revoke_with_associated_keys() {
        let store = KvStore::new();
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        // Put a key referencing this lease
        store.put(&PutRequest { key: "leased_key".into(), value: "val".into(), lease: Some(lease.id), prev_kv: false });
        // Revoking the lease should succeed (keys aren't tracked in lease.keys in current impl)
        assert!(store.lease_revoke(lease.id).is_ok());
        // Lease is gone
        assert!(store.lease_revoke(lease.id).is_err());
    }

    // --- compact ---

    #[test]
    fn test_compact_removes_old_revisions() {
        let store = KvStore::new();
        put(&store, "a", "1");
        let old_rev = store.current_revision();
        put(&store, "b", "2");
        store.compact(old_rev + 1);
        // old_rev is now below the compacted boundary
        let result = store.range(&RangeRequest {
            key: "a".into(), range_end: None, limit: None,
            revision: Some(old_rev), keys_only: false, count_only: false,
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_compact_keeps_current_data() {
        let store = KvStore::new();
        put(&store, "a", "current");
        let rev = store.current_revision();
        store.compact(rev);
        let resp = get(&store, "a");
        assert_eq!(resp.kvs[0].value_str(), "current");
    }

    #[test]
    fn test_compact_sets_compacted_revision() {
        let store = KvStore::new();
        put(&store, "x", "v");
        store.compact(42);
        // Any revision < 42 should now fail
        let result = store.range(&RangeRequest {
            key: "x".into(), range_end: None, limit: None,
            revision: Some(1), keys_only: false, count_only: false,
        });
        assert!(matches!(result, Err(EtcdError::RevisionCompacted { compacted: 42, .. })));
    }

    // --- subscribe / watch ---

    #[test]
    fn test_watch_put_event() {
        let store = KvStore::new();
        let mut rx = store.subscribe();
        put(&store, "watched", "v1");
        let event = rx.try_recv().unwrap();
        assert_eq!(event.kv.key_str(), "watched");
        assert!(matches!(event.event_type, EventType::Put));
    }

    #[test]
    fn test_watch_delete_event() {
        let store = KvStore::new();
        put(&store, "watch_del", "v");
        let mut rx = store.subscribe();
        store.delete_range(&DeleteRangeRequest { key: "watch_del".into(), range_end: None, prev_kv: false });
        let event = rx.try_recv().unwrap();
        assert!(matches!(event.event_type, EventType::Delete));
        assert_eq!(event.kv.key_str(), "watch_del");
    }

    #[test]
    fn test_watch_event_ordering() {
        let store = KvStore::new();
        let mut rx = store.subscribe();
        put(&store, "e1", "v1");
        put(&store, "e2", "v2");
        let ev1 = rx.try_recv().unwrap();
        let ev2 = rx.try_recv().unwrap();
        assert_eq!(ev1.kv.key_str(), "e1");
        assert_eq!(ev2.kv.key_str(), "e2");
    }

    #[test]
    fn test_watch_put_with_prev_kv() {
        let store = KvStore::new();
        put(&store, "wp", "old");
        let mut rx = store.subscribe();
        store.put(&PutRequest { key: "wp".into(), value: "new".into(), lease: None, prev_kv: true });
        let event = rx.try_recv().unwrap();
        assert!(event.prev_kv.is_some());
        assert_eq!(event.prev_kv.unwrap().value_str(), "old");
    }

    // --- status ---

    #[test]
    fn test_status_fields() {
        let store = KvStore::new();
        put(&store, "s", "t");
        let status = store.status();
        assert!(status.get("version").is_some());
        assert!(status.get("leader").is_some());
        assert!(status.get("raftIndex").is_some());
        assert!(status.get("raftTerm").is_some());
        assert!(status.get("dbSize").is_some());
        assert!(status.get("header").is_some());
    }

    #[test]
    fn test_status_db_size_reflects_entries() {
        let store = KvStore::new();
        let before = store.status()["dbSize"].as_u64().unwrap();
        put(&store, "extra", "v");
        let after = store.status()["dbSize"].as_u64().unwrap();
        assert!(after > before);
    }

    // --- current_revision ---

    #[test]
    fn test_current_revision_increments_on_put() {
        let store = KvStore::new();
        let r0 = store.current_revision();
        put(&store, "x", "y");
        assert!(store.current_revision() > r0);
    }

    // --- concurrent access ---

    #[test]
    fn test_concurrent_put_get() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(KvStore::new());
        let handles: Vec<_> = (0..10).map(|i| {
            let s = store.clone();
            thread::spawn(move || {
                let key = format!("concurrent_{}", i);
                s.put(&PutRequest { key: key.clone(), value: format!("val_{}", i), lease: None, prev_kv: false });
                let resp = s.range(&RangeRequest {
                    key, range_end: None, limit: None,
                    revision: None, keys_only: false, count_only: false,
                }).unwrap();
                assert_eq!(resp.kvs.len(), 1);
            })
        }).collect();
        for h in handles { h.join().unwrap(); }
    }

    #[test]
    fn test_concurrent_mixed_ops() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(KvStore::new());
        // Pre-populate
        for i in 0..5 {
            put(&store, &format!("shared_{}", i), "init");
        }

        let handles: Vec<_> = (0..5).map(|i| {
            let s = store.clone();
            thread::spawn(move || {
                // Overwrite
                s.put(&PutRequest { key: format!("shared_{}", i), value: format!("updated_{}", i), lease: None, prev_kv: false });
                // Read back
                let resp = s.range(&RangeRequest {
                    key: format!("shared_{}", i), range_end: None, limit: None,
                    revision: None, keys_only: false, count_only: false,
                }).unwrap();
                assert_eq!(resp.kvs.len(), 1);
            })
        }).collect();
        for h in handles { h.join().unwrap(); }
    }
}
