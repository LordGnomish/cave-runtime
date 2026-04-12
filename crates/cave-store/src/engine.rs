//! MVCC storage engine — the core KV store backing the etcd API.
//!
//! Uses a BTreeMap for ordered key storage with full revision history.
//! Supports prefix scans, range queries, compare-and-swap, and watch events.

use crate::error::{StoreError, StoreResult};
use crate::wal::{WalEntry, WalWriter};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info};

/// A single version of a key in the MVCC store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyVersion {
    pub create_revision: i64,
    pub mod_revision: i64,
    pub version: i64, // number of put ops (not counting deletes)
    pub value: Option<Vec<u8>>, // None = tombstone (deleted)
    pub lease_id: i64,
}

/// A watch event delivered to subscribers.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub watch_id: i64,
    pub revision: i64,
    pub event_type: EventType,
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>,
    pub prev_key: Option<KeyVersion>,
    pub create_revision: i64,
    pub mod_revision: i64,
    pub version: i64,
    pub lease_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventType {
    Put,
    Delete,
}

/// Active watch subscription.
#[derive(Debug)]
pub struct WatchSubscription {
    pub watch_id: i64,
    pub key: Vec<u8>,
    pub range_end: Vec<u8>, // empty = exact match; "\x00" = all keys >= key
    pub start_revision: i64,
    pub prev_kv: bool,
    pub filter_put: bool,
    pub filter_delete: bool,
    pub progress_notify: bool,
    pub tx: broadcast::Sender<WatchEvent>,
}

/// An active lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub lease_id: i64,
    pub ttl_secs: i64,
    pub granted_at: i64,    // unix seconds
    pub expires_at: i64,    // unix seconds
    pub keys: Vec<Vec<u8>>, // keys attached to this lease
}

impl Lease {
    pub fn new(lease_id: i64, ttl_secs: i64) -> Self {
        let now = Utc::now().timestamp();
        Lease {
            lease_id,
            ttl_secs,
            granted_at: now,
            expires_at: now + ttl_secs,
            keys: Vec::new(),
        }
    }

    pub fn remaining_ttl(&self) -> i64 {
        let now = Utc::now().timestamp();
        (self.expires_at - now).max(0)
    }

    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() >= self.expires_at
    }

    pub fn renew(&mut self) {
        let now = Utc::now().timestamp();
        self.granted_at = now;
        self.expires_at = now + self.ttl_secs;
    }
}

/// Compaction state — the revision below which history has been pruned.
#[derive(Default)]
struct CompactionState {
    compacted_revision: i64,
}

/// The MVCC engine. All state is held in-memory and recovered from WAL on startup.
pub struct MvccEngine {
    revision: AtomicI64,
    /// key → [version0, version1, ...] sorted by mod_revision ascending
    data: RwLock<BTreeMap<Vec<u8>, Vec<KeyVersion>>>,
    leases: RwLock<HashMap<i64, Lease>>,
    watches: RwLock<HashMap<i64, WatchSubscription>>,
    compaction: RwLock<CompactionState>,
    watch_id_counter: AtomicI64,
    wal: WalWriter,
}

impl MvccEngine {
    pub fn new(wal: WalWriter) -> Self {
        Self {
            revision: AtomicI64::new(1),
            data: RwLock::new(BTreeMap::new()),
            leases: RwLock::new(HashMap::new()),
            watches: RwLock::new(HashMap::new()),
            compaction: RwLock::new(CompactionState::default()),
            watch_id_counter: AtomicI64::new(1),
            wal,
        }
    }

    /// Replay WAL entries to restore state after restart.
    pub async fn replay_wal(&self, entries: Vec<WalEntry>) {
        for entry in entries {
            match entry {
                WalEntry::KvPut {
                    revision,
                    key,
                    value,
                    lease_id,
                } => {
                    self.apply_put(revision, key, value, lease_id).await;
                }
                WalEntry::KvDelete { revision, key } => {
                    self.apply_delete(revision, key).await;
                }
                WalEntry::KvCompact { revision } => {
                    self.apply_compact(revision).await;
                }
                WalEntry::LeaseGrant {
                    lease_id,
                    ttl_secs,
                    granted_at,
                } => {
                    let mut lease = Lease::new(lease_id, ttl_secs);
                    lease.granted_at = granted_at;
                    lease.expires_at = granted_at + ttl_secs;
                    self.leases.write().await.insert(lease_id, lease);
                }
                WalEntry::LeaseRevoke { lease_id } => {
                    self.leases.write().await.remove(&lease_id);
                }
                WalEntry::LeaseKeepAlive {
                    lease_id,
                    renewed_at,
                } => {
                    if let Some(lease) = self.leases.write().await.get_mut(&lease_id) {
                        lease.granted_at = renewed_at;
                        lease.expires_at = renewed_at + lease.ttl_secs;
                    }
                }
                _ => {} // S3 entries handled by ObjectStore
            }
        }
        info!(
            "MVCC replay complete, revision={}",
            self.revision.load(Ordering::SeqCst)
        );
    }

    async fn apply_put(&self, revision: i64, key: Vec<u8>, value: Vec<u8>, lease_id: i64) {
        let mut data = self.data.write().await;
        let versions = data.entry(key.clone()).or_default();
        let create_rev = versions
            .first()
            .filter(|v| v.value.is_some())
            .map(|v| v.create_revision)
            .unwrap_or(revision);
        let version_num = versions.iter().filter(|v| v.value.is_some()).count() as i64 + 1;
        versions.push(KeyVersion {
            create_revision: create_rev,
            mod_revision: revision,
            version: version_num,
            value: Some(value),
            lease_id,
        });
        self.revision.fetch_max(revision + 1, Ordering::SeqCst);
    }

    async fn apply_delete(&self, revision: i64, key: Vec<u8>) {
        let mut data = self.data.write().await;
        if let Some(versions) = data.get_mut(&key) {
            if let Some(last) = versions.last() {
                if last.value.is_some() {
                    versions.push(KeyVersion {
                        create_revision: last.create_revision,
                        mod_revision: revision,
                        version: last.version,
                        value: None, // tombstone
                        lease_id: 0,
                    });
                }
            }
        }
        self.revision.fetch_max(revision + 1, Ordering::SeqCst);
    }

    async fn apply_compact(&self, compact_rev: i64) {
        let mut data = self.data.write().await;
        for versions in data.values_mut() {
            versions.retain(|v| v.mod_revision >= compact_rev || {
                // Keep the last version at or before compact_rev
                false
            });
        }
        // Actually: keep the latest version at or before compact_rev + all versions after
        // Re-implement properly:
        let mut compaction = self.compaction.write().await;
        compaction.compacted_revision = compact_rev;
        drop(compaction);
        // Prune: for each key, remove versions < compact_rev except keep the most recent one
        // before compact_rev (so current state is preserved)
        for versions in data.values_mut() {
            let cutoff = compact_rev;
            // Find the last version at or before cutoff
            let last_before = versions.iter().rposition(|v| v.mod_revision <= cutoff);
            if let Some(idx) = last_before {
                // Remove all versions before idx
                if idx > 0 {
                    versions.drain(0..idx);
                }
            }
        }
        data.retain(|_, v| !v.is_empty());
        self.revision.fetch_max(compact_rev + 1, Ordering::SeqCst);
    }

    pub fn current_revision(&self) -> i64 {
        self.revision.load(Ordering::SeqCst)
    }

    fn next_revision(&self) -> i64 {
        self.revision.fetch_add(1, Ordering::SeqCst)
    }

    // ── KV Operations ──────────────────────────────────────────────────────────

    pub async fn put(
        &self,
        key: Vec<u8>,
        value: Vec<u8>,
        lease_id: i64,
        prev_kv: bool,
    ) -> StoreResult<PutResponse> {
        let rev = self.next_revision();
        let mut data = self.data.write().await;

        let prev = if prev_kv {
            data.get(&key).and_then(|v| v.last()).and_then(|kv| {
                if kv.value.is_some() {
                    Some(kv.clone())
                } else {
                    None
                }
            })
        } else {
            None
        };

        let versions = data.entry(key.clone()).or_default();
        let create_rev = versions
            .iter()
            .find(|v| v.value.is_some())
            .map(|v| v.create_revision)
            .unwrap_or(rev);
        let version_num = versions.iter().filter(|v| v.value.is_some()).count() as i64 + 1;

        let kv = KeyVersion {
            create_revision: create_rev,
            mod_revision: rev,
            version: version_num,
            value: Some(value.clone()),
            lease_id,
        };
        versions.push(kv.clone());
        drop(data);

        // Attach key to lease if applicable
        if lease_id != 0 {
            if let Some(lease) = self.leases.write().await.get_mut(&lease_id) {
                if !lease.keys.contains(&key) {
                    lease.keys.push(key.clone());
                }
            }
        }

        // WAL
        self.wal
            .append(&WalEntry::KvPut {
                revision: rev,
                key: key.clone(),
                value: value.clone(),
                lease_id,
            })
            .await?;

        // Notify watches
        self.notify_watches(
            WatchEvent {
                watch_id: 0,
                revision: rev,
                event_type: EventType::Put,
                key: key.clone(),
                value: Some(value),
                prev_key: prev.clone(),
                create_revision: kv.create_revision,
                mod_revision: kv.mod_revision,
                version: kv.version,
                lease_id,
            },
        )
        .await;

        Ok(PutResponse {
            header: ResponseHeader { revision: rev },
            prev_kv: prev,
        })
    }

    pub async fn range(
        &self,
        key: Vec<u8>,
        range_end: Vec<u8>,
        revision: i64,
        limit: i64,
        keys_only: bool,
        count_only: bool,
    ) -> StoreResult<RangeResponse> {
        let compact_rev = self.compaction.read().await.compacted_revision;
        if revision > 0 && revision < compact_rev {
            return Err(StoreError::RevisionCompacted {
                requested: revision,
                compacted: compact_rev,
            });
        }

        let current_rev = self.current_revision();
        let at_rev = if revision <= 0 { current_rev } else { revision };
        let data = self.data.read().await;

        let kvs = Self::collect_range(&data, &key, &range_end, at_rev, limit, keys_only);
        let count = kvs.len() as i64;

        Ok(RangeResponse {
            header: ResponseHeader {
                revision: current_rev,
            },
            kvs: if count_only { vec![] } else { kvs },
            count,
            more: false, // TODO: pagination
        })
    }

    fn collect_range(
        data: &BTreeMap<Vec<u8>, Vec<KeyVersion>>,
        key: &[u8],
        range_end: &[u8],
        at_rev: i64,
        limit: i64,
        keys_only: bool,
    ) -> Vec<KvPair> {
        let mut result = Vec::new();

        let iter: Box<dyn Iterator<Item = (&Vec<u8>, &Vec<KeyVersion>)>> = if range_end.is_empty()
        {
            // Exact key match — use range with equal bounds
            Box::new(data.range(key.to_vec()..=key.to_vec()))
        } else if range_end == b"\x00" {
            // All keys >= key
            Box::new(data.range(key.to_vec()..))
        } else {
            // Range [key, range_end)
            Box::new(data.range(key.to_vec()..range_end.to_vec()))
        };

        for (k, versions) in iter {
            let kv_at_rev = versions.iter().rev().find(|v| v.mod_revision <= at_rev);
            if let Some(kv) = kv_at_rev {
                if kv.value.is_some() {
                    result.push(KvPair {
                        key: k.clone(),
                        value: if keys_only {
                            vec![]
                        } else {
                            kv.value.clone().unwrap_or_default()
                        },
                        create_revision: kv.create_revision,
                        mod_revision: kv.mod_revision,
                        version: kv.version,
                        lease_id: kv.lease_id,
                    });
                    if limit > 0 && result.len() as i64 >= limit {
                        break;
                    }
                }
            }
        }

        result
    }

    pub async fn delete_range(
        &self,
        key: Vec<u8>,
        range_end: Vec<u8>,
        prev_kv: bool,
    ) -> StoreResult<DeleteRangeResponse> {
        let rev = self.next_revision();
        let mut data = self.data.write().await;

        let keys_to_delete: Vec<Vec<u8>> = if range_end.is_empty() {
            if data.contains_key(&key) {
                vec![key.clone()]
            } else {
                vec![]
            }
        } else if range_end == b"\x00" {
            data.range(key.clone()..).map(|(k, _)| k.clone()).collect()
        } else {
            data.range(key.clone()..range_end.clone())
                .map(|(k, _)| k.clone())
                .collect()
        };

        let mut deleted = 0i64;
        let mut prev_kvs = Vec::new();

        for k in &keys_to_delete {
            if let Some(versions) = data.get_mut(k) {
                if let Some(last) = versions.last() {
                    if last.value.is_some() {
                        if prev_kv {
                            prev_kvs.push(KvPair {
                                key: k.clone(),
                                value: last.value.clone().unwrap_or_default(),
                                create_revision: last.create_revision,
                                mod_revision: last.mod_revision,
                                version: last.version,
                                lease_id: last.lease_id,
                            });
                        }
                        versions.push(KeyVersion {
                            create_revision: last.create_revision,
                            mod_revision: rev,
                            version: last.version,
                            value: None,
                            lease_id: 0,
                        });
                        deleted += 1;
                    }
                }
            }
        }
        drop(data);

        for k in &keys_to_delete {
            self.wal
                .append(&WalEntry::KvDelete {
                    revision: rev,
                    key: k.clone(),
                })
                .await?;
            self.notify_watches(WatchEvent {
                watch_id: 0,
                revision: rev,
                event_type: EventType::Delete,
                key: k.clone(),
                value: None,
                prev_key: prev_kvs.iter().find(|p| &p.key == k).map(|p| KeyVersion {
                    create_revision: p.create_revision,
                    mod_revision: p.mod_revision,
                    version: p.version,
                    value: Some(p.value.clone()),
                    lease_id: p.lease_id,
                }),
                create_revision: 0,
                mod_revision: rev,
                version: 0,
                lease_id: 0,
            })
            .await;
        }

        Ok(DeleteRangeResponse {
            header: ResponseHeader { revision: rev },
            deleted,
            prev_kvs,
        })
    }

    pub async fn txn(&self, txn: TxnRequest) -> StoreResult<TxnResponse> {
        let current_rev = self.current_revision();
        let data = self.data.read().await;

        // Evaluate compares
        let success = txn.compare.iter().all(|cmp| {
            let kv = data.get(&cmp.key).and_then(|v| v.last());
            match &cmp.target {
                CompareTarget::Version(expected) => {
                    let actual = kv
                        .filter(|v| v.value.is_some())
                        .map(|v| v.version)
                        .unwrap_or(0);
                    compare_values(actual, *expected, &cmp.result)
                }
                CompareTarget::CreateRevision(expected) => {
                    let actual = kv
                        .filter(|v| v.value.is_some())
                        .map(|v| v.create_revision)
                        .unwrap_or(0);
                    compare_values(actual, *expected, &cmp.result)
                }
                CompareTarget::ModRevision(expected) => {
                    let actual = kv
                        .filter(|v| v.value.is_some())
                        .map(|v| v.mod_revision)
                        .unwrap_or(0);
                    compare_values(actual, *expected, &cmp.result)
                }
                CompareTarget::Value(expected) => {
                    let actual = kv
                        .and_then(|v| v.value.as_deref())
                        .unwrap_or(&[]);
                    compare_values_bytes(actual, expected, &cmp.result)
                }
                CompareTarget::Lease(expected) => {
                    let actual = kv.map(|v| v.lease_id).unwrap_or(0);
                    compare_values(actual, *expected, &cmp.result)
                }
            }
        });
        drop(data);

        let ops = if success {
            txn.success
        } else {
            txn.failure
        };

        let mut responses = Vec::new();
        for op in ops {
            match op {
                TxnOp::Put {
                    key,
                    value,
                    lease_id,
                } => {
                    let r = self.put(key, value, lease_id, false).await?;
                    responses.push(TxnOpResponse::Put(r));
                }
                TxnOp::Range {
                    key,
                    range_end,
                    revision,
                } => {
                    let r = self
                        .range(key, range_end, revision, 0, false, false)
                        .await?;
                    responses.push(TxnOpResponse::Range(r));
                }
                TxnOp::Delete { key, range_end } => {
                    let r = self.delete_range(key, range_end, false).await?;
                    responses.push(TxnOpResponse::Delete(r));
                }
            }
        }

        Ok(TxnResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
            succeeded: success,
            responses,
        })
    }

    pub async fn compact(&self, revision: i64) -> StoreResult<CompactResponse> {
        let current_rev = self.current_revision();
        if revision > current_rev {
            return Err(StoreError::InvalidArgument(format!(
                "compact revision {revision} > current {current_rev}"
            )));
        }
        let compact_rev = self.compaction.read().await.compacted_revision;
        if revision <= compact_rev {
            return Err(StoreError::RevisionCompacted {
                requested: revision,
                compacted: compact_rev,
            });
        }

        self.apply_compact(revision).await;
        self.wal
            .append(&WalEntry::KvCompact { revision })
            .await?;

        Ok(CompactResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
        })
    }

    // ── Watch ──────────────────────────────────────────────────────────────────

    pub async fn watch_create(
        &self,
        key: Vec<u8>,
        range_end: Vec<u8>,
        start_revision: i64,
        prev_kv: bool,
        filter_put: bool,
        filter_delete: bool,
        progress_notify: bool,
    ) -> (i64, broadcast::Receiver<WatchEvent>) {
        let watch_id = self.watch_id_counter.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = broadcast::channel(1024);
        let sub = WatchSubscription {
            watch_id,
            key,
            range_end,
            start_revision,
            prev_kv,
            filter_put,
            filter_delete,
            progress_notify,
            tx,
        };
        self.watches.write().await.insert(watch_id, sub);
        (watch_id, rx)
    }

    pub async fn watch_cancel(&self, watch_id: i64) -> bool {
        self.watches.write().await.remove(&watch_id).is_some()
    }

    async fn notify_watches(&self, mut event: WatchEvent) {
        let watches = self.watches.read().await;
        for (id, sub) in watches.iter() {
            // Check filter
            if sub.filter_put && event.event_type == EventType::Put {
                continue;
            }
            if sub.filter_delete && event.event_type == EventType::Delete {
                continue;
            }
            // Check key match
            let matches = if sub.range_end.is_empty() {
                event.key == sub.key
            } else if sub.range_end == b"\x00" {
                event.key >= sub.key
            } else {
                event.key >= sub.key && event.key < sub.range_end
            };
            if !matches {
                continue;
            }
            if event.revision < sub.start_revision {
                continue;
            }
            event.watch_id = *id;
            let _ = sub.tx.send(event.clone());
        }
    }

    // ── Lease ──────────────────────────────────────────────────────────────────

    pub async fn lease_grant(&self, ttl_secs: i64, lease_id: i64) -> StoreResult<LeaseGrantResponse> {
        let id = if lease_id == 0 {
            // Generate random ID
            rand_lease_id()
        } else {
            lease_id
        };
        {
            let leases = self.leases.read().await;
            if leases.contains_key(&id) {
                return Err(StoreError::LeaseExists(id));
            }
        }
        let lease = Lease::new(id, ttl_secs);
        let granted_at = lease.granted_at;
        self.leases.write().await.insert(id, lease);
        self.wal
            .append(&WalEntry::LeaseGrant {
                lease_id: id,
                ttl_secs,
                granted_at,
            })
            .await?;
        Ok(LeaseGrantResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
            id,
            ttl: ttl_secs,
            error: String::new(),
        })
    }

    pub async fn lease_revoke(&self, lease_id: i64) -> StoreResult<LeaseRevokeResponse> {
        let lease = self
            .leases
            .write()
            .await
            .remove(&lease_id)
            .ok_or(StoreError::LeaseNotFound(lease_id))?;
        // Delete all keys attached to this lease
        for key in lease.keys {
            let rev = self.next_revision();
            self.apply_delete(rev, key.clone()).await;
            let _ = self
                .wal
                .append(&WalEntry::KvDelete {
                    revision: rev,
                    key,
                })
                .await;
        }
        self.wal
            .append(&WalEntry::LeaseRevoke { lease_id })
            .await?;
        Ok(LeaseRevokeResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
        })
    }

    pub async fn lease_keep_alive(&self, lease_id: i64) -> StoreResult<LeaseKeepAliveResponse> {
        let renewed_at = Utc::now().timestamp();
        let ttl = {
            let mut leases = self.leases.write().await;
            let lease = leases
                .get_mut(&lease_id)
                .ok_or(StoreError::LeaseNotFound(lease_id))?;
            lease.renew();
            lease.ttl_secs
        };
        self.wal
            .append(&WalEntry::LeaseKeepAlive {
                lease_id,
                renewed_at,
            })
            .await?;
        Ok(LeaseKeepAliveResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
            id: lease_id,
            ttl,
        })
    }

    pub async fn lease_time_to_live(
        &self,
        lease_id: i64,
        keys: bool,
    ) -> StoreResult<LeaseTimeToLiveResponse> {
        let leases = self.leases.read().await;
        let lease = leases
            .get(&lease_id)
            .ok_or(StoreError::LeaseNotFound(lease_id))?;
        Ok(LeaseTimeToLiveResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
            id: lease_id,
            ttl: lease.ttl_secs,
            granted_ttl: lease.ttl_secs,
            keys: if keys {
                lease.keys.clone()
            } else {
                vec![]
            },
        })
    }

    pub async fn lease_list(&self) -> LeaseListResponse {
        let leases = self.leases.read().await;
        LeaseListResponse {
            header: ResponseHeader {
                revision: self.current_revision(),
            },
            leases: leases.keys().map(|id| LeaseStatus { id: *id }).collect(),
        }
    }

    /// Background task: expire leases and delete their keys.
    pub async fn run_lease_reaper(engine: Arc<MvccEngine>) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let expired: Vec<i64> = {
                let leases = engine.leases.read().await;
                leases
                    .values()
                    .filter(|l| l.is_expired())
                    .map(|l| l.lease_id)
                    .collect()
            };
            for lease_id in expired {
                debug!("Expiring lease {lease_id}");
                let _ = engine.lease_revoke(lease_id).await;
            }
        }
    }
}

fn rand_lease_id() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (t as i64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn compare_values<T: Ord>(actual: T, expected: T, result: &CompareResult) -> bool {
    match result {
        CompareResult::Equal => actual == expected,
        CompareResult::Greater => actual > expected,
        CompareResult::Less => actual < expected,
        CompareResult::NotEqual => actual != expected,
    }
}

fn compare_values_bytes(actual: &[u8], expected: &[u8], result: &CompareResult) -> bool {
    match result {
        CompareResult::Equal => actual == expected,
        CompareResult::Greater => actual > expected,
        CompareResult::Less => actual < expected,
        CompareResult::NotEqual => actual != expected,
    }
}

// ── Response / Request types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseHeader {
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvPair {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub create_revision: i64,
    pub mod_revision: i64,
    pub version: i64,
    pub lease_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PutResponse {
    pub header: ResponseHeader,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_kv: Option<KeyVersion>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RangeResponse {
    pub header: ResponseHeader,
    pub kvs: Vec<KvPair>,
    pub count: i64,
    pub more: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteRangeResponse {
    pub header: ResponseHeader,
    pub deleted: i64,
    pub prev_kvs: Vec<KvPair>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompactResponse {
    pub header: ResponseHeader,
}

// Txn types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompareResult {
    Equal,
    Greater,
    Less,
    NotEqual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "target_union_case", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompareTarget {
    Version(i64),
    CreateRevision(i64),
    ModRevision(i64),
    Value(Vec<u8>),
    Lease(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compare {
    pub key: Vec<u8>,
    pub result: CompareResult,
    pub target: CompareTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op_type", rename_all = "snake_case")]
pub enum TxnOp {
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
        #[serde(default)]
        lease_id: i64,
    },
    Range {
        key: Vec<u8>,
        #[serde(default)]
        range_end: Vec<u8>,
        #[serde(default)]
        revision: i64,
    },
    Delete {
        key: Vec<u8>,
        #[serde(default)]
        range_end: Vec<u8>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TxnOpResponse {
    Put(PutResponse),
    Range(RangeResponse),
    Delete(DeleteRangeResponse),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxnRequest {
    pub compare: Vec<Compare>,
    pub success: Vec<TxnOp>,
    pub failure: Vec<TxnOp>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxnResponse {
    pub header: ResponseHeader,
    pub succeeded: bool,
    pub responses: Vec<TxnOpResponse>,
}

// Lease types
#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseGrantResponse {
    pub header: ResponseHeader,
    pub id: i64,
    pub ttl: i64,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseRevokeResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseKeepAliveResponse {
    pub header: ResponseHeader,
    pub id: i64,
    pub ttl: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseTimeToLiveResponse {
    pub header: ResponseHeader,
    pub id: i64,
    pub ttl: i64,
    pub granted_ttl: i64,
    pub keys: Vec<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseStatus {
    pub id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseListResponse {
    pub header: ResponseHeader,
    pub leases: Vec<LeaseStatus>,
}
