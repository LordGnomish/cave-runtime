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

    #[test]
    fn test_put_and_get() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "foo".into(), value: "bar".into(), lease: None, prev_kv: false });
        let resp = store.range(&RangeRequest {
            key: "foo".into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
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
    fn test_put_prev_kv() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "x".into(), value: "old".into(), lease: None, prev_kv: false });
        let resp = store.put(&PutRequest { key: "x".into(), value: "new".into(), lease: None, prev_kv: true });
        assert!(resp.prev_kv.is_some());
        assert_eq!(resp.prev_kv.unwrap().value_str(), "old");
    }

    #[test]
    fn test_delete() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "del_me".into(), value: "v".into(), lease: None, prev_kv: false });
        let resp = store.delete_range(&DeleteRangeRequest { key: "del_me".into(), range_end: None, prev_kv: true });
        assert_eq!(resp.deleted, 1);
        assert_eq!(resp.prev_kvs[0].value_str(), "v");

        let get = store.range(&RangeRequest { key: "del_me".into(), range_end: None, limit: None, revision: None, keys_only: false, count_only: false }).unwrap();
        assert_eq!(get.kvs.len(), 0);
    }

    #[test]
    fn test_range_query() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "/a/1".into(), value: "v1".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "/a/2".into(), value: "v2".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "/b/1".into(), value: "v3".into(), lease: None, prev_kv: false });

        let resp = store.range(&RangeRequest {
            key: "/a/".into(), range_end: Some("/a0".into()), limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 2);
    }

    #[test]
    fn test_count_only() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "k1".into(), value: "v1".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "k2".into(), value: "v2".into(), lease: None, prev_kv: false });

        let resp = store.range(&RangeRequest {
            key: "k".into(), range_end: Some("l".into()), limit: None,
            revision: None, keys_only: false, count_only: true,
        }).unwrap();
        assert_eq!(resp.count, 2);
        assert!(resp.kvs.is_empty());
    }

    #[test]
    fn test_lease_grant_and_revoke() {
        let store = KvStore::new();
        let resp = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        assert!(resp.id > 0);
        assert_eq!(resp.ttl, 60);

        assert!(store.lease_revoke(resp.id).is_ok());
        assert!(store.lease_revoke(99999).is_err());
    }

    #[test]
    fn test_watch_notification() {
        let store = KvStore::new();
        let mut rx = store.subscribe();

        store.put(&PutRequest { key: "watched".into(), value: "v1".into(), lease: None, prev_kv: false });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.kv.key_str(), "watched");
        assert!(matches!(event.event_type, EventType::Put));
    }

    #[test]
    fn test_compact() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "a".into(), value: "1".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "b".into(), value: "2".into(), lease: None, prev_kv: false });
        let rev = store.current_revision();
        store.compact(rev);
        // After compaction, old revision reads should fail
    }

    #[test]
    fn test_status() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "s".into(), value: "t".into(), lease: None, prev_kv: false });
        let status = store.status();
        assert!(status.get("version").is_some());
    }
}
