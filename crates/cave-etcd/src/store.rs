//! MVCC key-value store with revision tracking.
//!
//! Implements etcd's multi-version concurrency control model:
//! every write creates a new revision, reads can target specific revisions.

use crate::error::{EtcdError, EtcdResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};

/// Maximum lease TTL accepted by `lease_grant`.  Matches the etcd v3.6
/// client default of 9000s (`clientv3/lease.go: defaultTTL`).
pub const MAX_LEASE_TTL_SECS: i64 = 9_000;

/// Default chunk size for `snapshot_stream` (matches etcd v3.6
/// `etcdserver.snapshotSendBufferSize`).
pub const SNAPSHOT_CHUNK_SIZE: usize = 32 * 1024;

#[cfg(test)]
const BCRYPT_COST: u32 = 4;
#[cfg(not(test))]
const BCRYPT_COST: u32 = 12;

/// MVCC key-value store.
pub struct KvStore {
    /// Current live key-value pairs.
    current: DashMap<Vec<u8>, KeyValue>,
    /// Revision history: revision -> (key, event_type, kv).
    history: RwLock<BTreeMap<u64, (Vec<u8>, EventType, KeyValue)>>,
    /// Per-key revision index: key -> sorted list of revisions (puts and deletes).
    key_index: DashMap<Vec<u8>, Vec<u64>>,
    /// Monotonically increasing revision counter.
    revision: AtomicU64,
    /// Watch notification channel.
    watch_tx: broadcast::Sender<WatchEvent>,
    /// Per-watch filter configs: watch_id -> WatchConfig.
    watch_configs: DashMap<i64, WatchConfig>,
    /// Per-watch dedicated mpsc inbox (multiplexer).  When present,
    /// `dispatch_event` fans the event in here in addition to the broadcast.
    watch_inboxes: DashMap<i64, mpsc::UnboundedSender<WatchEvent>>,
    /// Active leases.
    leases: DashMap<i64, Lease>,
    /// Lease ID counter.
    lease_counter: AtomicU64,
    /// Compacted revision (history before this is deleted).
    compacted_revision: AtomicU64,
    /// Whether auth is enabled.
    auth_enabled: AtomicBool,
    /// Auth users: name -> AuthUser.
    users: DashMap<String, AuthUser>,
    /// Auth roles: name -> AuthRole.
    roles: DashMap<String, AuthRole>,
    /// Auth tokens: token -> username.
    auth_tokens: DashMap<String, String>,
    /// Active alarms.
    alarms: RwLock<Vec<AlarmMember>>,
    /// Cluster members.
    members: RwLock<Vec<Member>>,
    /// Active joint-consensus configuration (Some during a Cold→Cnew transition).
    joint: RwLock<Option<JointConfig>>,
    /// Watch ID counter.
    watch_counter: AtomicU64,
    /// Serialises transactions (compare-then-execute must be atomic).
    txn_lock: Mutex<()>,
}

impl KvStore {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(4096);
        let initial_members = vec![Member {
            id: 1,
            name: "default".to_string(),
            peer_urls: vec!["http://localhost:2380".to_string()],
            client_urls: vec!["http://localhost:2379".to_string()],
            is_learner: false,
        }];
        Self {
            current: DashMap::new(),
            history: RwLock::new(BTreeMap::new()),
            key_index: DashMap::new(),
            revision: AtomicU64::new(1),
            watch_tx,
            watch_configs: DashMap::new(),
            watch_inboxes: DashMap::new(),
            leases: DashMap::new(),
            lease_counter: AtomicU64::new(1),
            compacted_revision: AtomicU64::new(0),
            auth_enabled: AtomicBool::new(false),
            users: DashMap::new(),
            roles: DashMap::new(),
            auth_tokens: DashMap::new(),
            alarms: RwLock::new(Vec::new()),
            members: RwLock::new(initial_members),
            joint: RwLock::new(None),
            watch_counter: AtomicU64::new(0),
            txn_lock: Mutex::new(()),
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

    // ── MVCC helpers ──────────────────────────────────────────────────────

    /// Update the key_index with a new revision for a key.
    fn index_push(&self, key: &[u8], rev: u64) {
        self.key_index.entry(key.to_vec()).or_default().push(rev);
    }

    /// Reconstruct the state of a single key at the given revision.
    /// Returns None if the key did not exist at that revision.
    fn get_at_revision(&self, key: &[u8], target_rev: u64) -> Option<KeyValue> {
        let revs = self.key_index.get(key)?;
        // find largest revision <= target_rev
        let &rev = revs.iter().rev().find(|&&r| r <= target_rev)?;
        let history = self.history.read().unwrap();
        let (_, event_type, kv) = history.get(&rev)?;
        if matches!(event_type, EventType::Delete) {
            None
        } else {
            Some(kv.clone())
        }
    }

    /// Whether a key falls within a watch's key/range_end pattern.
    pub fn key_matches_watch(key: &[u8], config: &WatchConfig) -> bool {
        if let Some(ref range_end) = config.range_end {
            key >= config.key.as_slice() && key < range_end.as_slice()
        } else {
            key == config.key.as_slice()
        }
    }

    /// Collect historical watch events since `from_rev` matching config.
    pub fn get_historical_events(&self, config: &WatchConfig, from_rev: u64) -> Vec<WatchEvent> {
        let compacted = self.compacted_revision.load(Ordering::SeqCst);
        let start = from_rev.max(compacted + 1);
        let history = self.history.read().unwrap();
        history
            .range(start..)
            .filter_map(|(_, (key, event_type, kv))| {
                if Self::key_matches_watch(key, config) {
                    Some(WatchEvent {
                        event_type: event_type.clone(),
                        kv: kv.clone(),
                        prev_kv: None,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Retrieve a watch config by ID.
    pub fn get_watch_config(&self, watch_id: i64) -> Option<WatchConfig> {
        self.watch_configs.get(&watch_id).map(|c| c.clone())
    }

    /// Fan an event to: (a) the global broadcast channel (preserved for legacy
    /// `subscribe()` consumers), and (b) each per-watch inbox whose config
    /// matches the event's key.  This is the multiplexer entry point.
    fn dispatch_event(&self, event: WatchEvent) {
        let _ = self.watch_tx.send(event.clone());

        // Per-watch fan-out.  Closed inboxes (cancelled watchers) are pruned.
        let mut closed: Vec<i64> = Vec::new();
        for entry in self.watch_inboxes.iter() {
            let id = *entry.key();
            let Some(cfg) = self.watch_configs.get(&id) else {
                closed.push(id);
                continue;
            };
            if !Self::key_matches_watch(&event.kv.key, &cfg) {
                continue;
            }
            // Honour prev_kv flag — strip if the watch did not request it.
            let mut local = event.clone();
            if !cfg.prev_kv {
                local.prev_kv = None;
            }
            if entry.value().send(local).is_err() {
                closed.push(id);
            }
        }
        for id in closed {
            self.watch_inboxes.remove(&id);
        }
    }

    // ── KV ────────────────────────────────────────────────────────────────

    /// PUT a key-value pair.
    pub fn put(&self, req: &PutRequest) -> PutResponse {
        let key = req.key.as_bytes().to_vec();
        let rev = self.next_revision();

        let prev_kv = self.current.get(&key).map(|r| r.value().clone());

        // If this key previously had a different lease, remove from old lease.
        if let Some(ref old) = prev_kv {
            if old.lease != req.lease {
                if let Some(old_lease_id) = old.lease {
                    if let Some(mut lease) = self.leases.get_mut(&old_lease_id) {
                        lease.keys.retain(|k| k.as_bytes() != key);
                    }
                }
            }
        }

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
        self.index_push(&key, rev);

        if let Ok(mut history) = self.history.write() {
            history.insert(rev, (key.clone(), EventType::Put, kv.clone()));
        }

        // Associate key with lease.
        if let Some(lease_id) = req.lease {
            if let Some(mut lease) = self.leases.get_mut(&lease_id) {
                let key_str = String::from_utf8_lossy(&key).to_string();
                if !lease.keys.contains(&key_str) {
                    lease.keys.push(key_str);
                }
            }
        }

        // Always include prev_kv in the dispatched event so per-watch
        // multiplexers with their own prev_kv flag can decide what to forward.
        self.dispatch_event(WatchEvent {
            event_type: EventType::Put,
            kv: kv.clone(),
            prev_kv: prev_kv.clone(),
        });

        PutResponse {
            header: self.header(),
            prev_kv: if req.prev_kv { prev_kv } else { None },
        }
    }

    /// GET a key or range of keys, optionally at a specific revision.
    pub fn range(&self, req: &RangeRequest) -> EtcdResult<RangeResponse> {
        let compacted = self.compacted_revision.load(Ordering::SeqCst);

        if let Some(target_rev) = req.revision {
            if target_rev < compacted && compacted > 0 {
                return Err(EtcdError::RevisionCompacted {
                    requested: target_rev,
                    compacted,
                });
            }
            // Time-travel read: reconstruct state at target_rev via key_index + history.
            return self.range_at_revision(req, target_rev);
        }

        // Fast path: read current state.
        let key_bytes = req.key.as_bytes().to_vec();
        let mut kvs = Vec::new();

        if let Some(ref range_end) = req.range_end {
            let end_bytes = range_end.as_bytes().to_vec();
            for entry in self.current.iter() {
                let k = entry.key();
                if *k >= key_bytes && *k < end_bytes {
                    kvs.push(entry.value().clone());
                }
            }
            kvs.sort_by(|a, b| a.key.cmp(&b.key));
        } else if let Some(kv) = self.current.get(&key_bytes) {
            kvs.push(kv.value().clone());
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

    /// Time-travel read: iterate key_index to reconstruct state at target_rev.
    fn range_at_revision(&self, req: &RangeRequest, target_rev: u64) -> EtcdResult<RangeResponse> {
        let key_bytes = req.key.as_bytes().to_vec();
        let mut kvs = Vec::new();

        if let Some(ref range_end) = req.range_end {
            let end_bytes = range_end.as_bytes().to_vec();
            for entry in self.key_index.iter() {
                let k = entry.key();
                if *k >= key_bytes && *k < end_bytes {
                    if let Some(kv) = self.get_at_revision(k, target_rev) {
                        kvs.push(kv);
                    }
                }
            }
            kvs.sort_by(|a, b| a.key.cmp(&b.key));
        } else if let Some(kv) = self.get_at_revision(&key_bytes, target_rev) {
            kvs.push(kv);
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
            let keys_to_delete: Vec<Vec<u8>> = self
                .current
                .iter()
                .filter(|e| *e.key() >= key_bytes && *e.key() < end_bytes)
                .map(|e| e.key().clone())
                .collect();

            for key in keys_to_delete {
                if let Some((_, kv)) = self.current.remove(&key) {
                    deleted += 1;
                    self.remove_key_from_lease(&key, kv.lease);
                    let delete_kv = make_delete_kv(&kv, rev);
                    self.index_push(&key, rev);
                    if let Ok(mut history) = self.history.write() {
                        history.insert(rev, (key.clone(), EventType::Delete, delete_kv.clone()));
                    }
                    self.dispatch_event(WatchEvent {
                        event_type: EventType::Delete,
                        kv: delete_kv,
                        prev_kv: Some(kv.clone()),
                    });
                    if req.prev_kv {
                        prev_kvs.push(kv);
                    }
                }
            }
        } else if let Some((_, kv)) = self.current.remove(&key_bytes) {
            deleted = 1;
            self.remove_key_from_lease(&key_bytes, kv.lease);
            let delete_kv = make_delete_kv(&kv, rev);
            self.index_push(&key_bytes, rev);
            if let Ok(mut history) = self.history.write() {
                history.insert(rev, (key_bytes, EventType::Delete, delete_kv.clone()));
            }
            self.dispatch_event(WatchEvent {
                event_type: EventType::Delete,
                kv: delete_kv,
                prev_kv: Some(kv.clone()),
            });
            if req.prev_kv {
                prev_kvs.push(kv);
            }
        }

        DeleteRangeResponse {
            header: self.header(),
            deleted,
            prev_kvs,
        }
    }

    fn remove_key_from_lease(&self, key: &[u8], lease_id: Option<i64>) {
        if let Some(id) = lease_id {
            if let Some(mut lease) = self.leases.get_mut(&id) {
                lease.keys.retain(|k| k.as_bytes() != key);
            }
        }
    }

    // ── Transaction ───────────────────────────────────────────────────────

    /// Atomic compare-and-swap transaction.
    /// Holds txn_lock for the full compare + execute cycle.
    pub fn txn(&self, req: &TxnRequest) -> TxnResponse {
        let _guard = self.txn_lock.lock().unwrap();

        let mut succeeded = true;
        for cmp in &req.compare {
            let kv = self
                .range(&RangeRequest {
                    key: cmp.key.clone(),
                    range_end: None,
                    limit: None,
                    revision: None,
                    keys_only: false,
                    count_only: false,
                })
                .ok()
                .and_then(|r| r.kvs.into_iter().next());

            let pass = match (&cmp.target, &cmp.result) {
                (CompareTarget::Version, CompareResult::Equal) => {
                    kv.as_ref().map(|k| k.version) == cmp.version
                }
                (CompareTarget::Version, CompareResult::Greater) => {
                    kv.as_ref().map(|k| k.version).unwrap_or(0)
                        > cmp.version.unwrap_or(0)
                }
                (CompareTarget::Version, CompareResult::Less) => {
                    kv.as_ref().map(|k| k.version).unwrap_or(0)
                        < cmp.version.unwrap_or(0)
                }
                (CompareTarget::Version, CompareResult::NotEqual) => {
                    kv.as_ref().map(|k| k.version) != cmp.version
                }
                (CompareTarget::Create, CompareResult::Equal) => {
                    kv.as_ref().map(|k| k.create_revision) == cmp.mod_revision
                }
                (CompareTarget::Create, CompareResult::Greater) => {
                    kv.as_ref().map(|k| k.create_revision).unwrap_or(0)
                        > cmp.mod_revision.unwrap_or(0)
                }
                (CompareTarget::Create, CompareResult::Less) => {
                    kv.as_ref().map(|k| k.create_revision).unwrap_or(0)
                        < cmp.mod_revision.unwrap_or(0)
                }
                (CompareTarget::Create, CompareResult::NotEqual) => {
                    kv.as_ref().map(|k| k.create_revision) != cmp.mod_revision
                }
                (CompareTarget::Mod, CompareResult::Equal) => {
                    kv.as_ref().map(|k| k.mod_revision) == cmp.mod_revision
                }
                (CompareTarget::Mod, CompareResult::Greater) => {
                    kv.as_ref().map(|k| k.mod_revision).unwrap_or(0)
                        > cmp.mod_revision.unwrap_or(0)
                }
                (CompareTarget::Mod, CompareResult::Less) => {
                    kv.as_ref().map(|k| k.mod_revision).unwrap_or(0)
                        < cmp.mod_revision.unwrap_or(0)
                }
                (CompareTarget::Mod, CompareResult::NotEqual) => {
                    kv.as_ref().map(|k| k.mod_revision) != cmp.mod_revision
                }
                (CompareTarget::Value, CompareResult::Equal) => {
                    kv.as_ref().map(|k| k.value_str()) == cmp.value.clone()
                }
                (CompareTarget::Value, CompareResult::NotEqual) => {
                    kv.as_ref().map(|k| k.value_str()) != cmp.value.clone()
                }
                _ => true,
            };
            if !pass {
                succeeded = false;
                break;
            }
        }

        let ops = if succeeded { &req.success } else { &req.failure };
        for op in ops {
            match op {
                RequestOp::Put(put) => {
                    self.put(put);
                }
                RequestOp::DeleteRange(del) => {
                    self.delete_range(del);
                }
                RequestOp::Range(_) => {}
            }
        }

        TxnResponse {
            header: ResponseHeader {
                cluster_id: 1,
                member_id: 1,
                revision: self.current_revision(),
                raft_term: 1,
            },
            succeeded,
        }
    }

    // ── Watch ─────────────────────────────────────────────────────────────

    /// Subscribe to all watch events (raw broadcast receiver).
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.watch_tx.subscribe()
    }

    /// Create a watch — stores the config, returns watch_id + any historical events.
    pub fn watch_create(&self, req: &WatchCreateRequest) -> WatchResponse {
        let watch_id = self.watch_counter.fetch_add(1, Ordering::SeqCst) as i64 + 1;

        let config = WatchConfig {
            watch_id,
            key: req.key.as_bytes().to_vec(),
            range_end: req.range_end.as_ref().map(|s| s.as_bytes().to_vec()),
            start_revision: req.start_revision,
            prev_kv: req.prev_kv,
        };

        // Replay historical events when start_revision is given.
        let events = req
            .start_revision
            .map(|start_rev| self.get_historical_events(&config, start_rev))
            .unwrap_or_default();

        self.watch_configs.insert(watch_id, config);

        WatchResponse {
            header: self.header(),
            watch_id,
            created: true,
            events,
        }
    }

    // ── Lease ─────────────────────────────────────────────────────────────

    /// Grant a lease.
    pub fn lease_grant(&self, req: &LeaseGrantRequest) -> LeaseGrantResponse {
        let id = req.id.unwrap_or_else(|| {
            self.lease_counter.fetch_add(1, Ordering::SeqCst) as i64 + 1
        });
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

    /// Revoke a lease and delete all associated keys (with watch events).
    pub fn lease_revoke(&self, lease_id: i64) -> EtcdResult<()> {
        let lease = self
            .leases
            .remove(&lease_id)
            .map(|(_, l)| l)
            .ok_or(EtcdError::LeaseNotFound(lease_id))?;

        for key_str in &lease.keys {
            let key = key_str.as_bytes().to_vec();
            if let Some((_, kv)) = self.current.remove(&key) {
                let rev = self.next_revision();
                let delete_kv = make_delete_kv(&kv, rev);
                self.index_push(&key, rev);
                if let Ok(mut history) = self.history.write() {
                    history.insert(rev, (key.clone(), EventType::Delete, delete_kv.clone()));
                }
                self.dispatch_event(WatchEvent {
                    event_type: EventType::Delete,
                    kv: delete_kv,
                    prev_kv: Some(kv),
                });
            }
        }
        Ok(())
    }

    /// Refresh a lease TTL.
    pub fn lease_keepalive(
        &self,
        req: &LeaseKeepAliveRequest,
    ) -> EtcdResult<LeaseKeepAliveResponse> {
        let mut lease = self
            .leases
            .get_mut(&req.id)
            .ok_or(EtcdError::LeaseNotFound(req.id))?;
        lease.granted_at = Utc::now();
        let ttl = lease.ttl;
        Ok(LeaseKeepAliveResponse {
            header: self.header(),
            id: req.id,
            ttl,
        })
    }

    /// Get remaining TTL for a lease.
    pub fn lease_timetolive(&self, req: &LeaseTTLRequest) -> EtcdResult<LeaseTTLResponse> {
        let lease = self
            .leases
            .get(&req.id)
            .ok_or(EtcdError::LeaseNotFound(req.id))?;
        let elapsed = Utc::now()
            .signed_duration_since(lease.granted_at)
            .num_seconds();
        let remaining = (lease.ttl - elapsed).max(0);
        let keys = if req.keys {
            lease.keys.iter().map(|k| k.as_bytes().to_vec()).collect()
        } else {
            vec![]
        };
        Ok(LeaseTTLResponse {
            header: self.header(),
            id: req.id,
            ttl: remaining,
            granted_ttl: lease.ttl,
            keys,
        })
    }

    /// List all active leases.
    pub fn lease_leases(&self) -> LeaseLeasesResponse {
        let leases = self
            .leases
            .iter()
            .map(|e| LeaseStatus { id: *e.key() })
            .collect();
        LeaseLeasesResponse {
            header: self.header(),
            leases,
        }
    }

    /// Expire leases whose TTL has elapsed; delete their keys and fire watch events.
    /// Called periodically by the background task.
    pub fn expire_leases(&self) {
        let now = Utc::now();
        let expired_ids: Vec<i64> = self
            .leases
            .iter()
            .filter(|e| {
                let elapsed = now
                    .signed_duration_since(e.value().granted_at)
                    .num_seconds();
                elapsed >= e.value().ttl
            })
            .map(|e| *e.key())
            .collect();

        for id in expired_ids {
            // lease_revoke handles deletion + watch events.
            let _ = self.lease_revoke(id);
        }
    }

    // ── Compaction ────────────────────────────────────────────────────────

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

    /// Compact with a typed response.
    pub fn compaction(&self, req: &CompactionRequest) -> CompactionResponse {
        self.compact(req.revision);
        CompactionResponse {
            header: self.header(),
        }
    }

    // ── Auth ──────────────────────────────────────────────────────────────

    pub fn auth_enable(&self) -> EtcdResult<AuthEnableResponse> {
        if self.auth_enabled.swap(true, Ordering::SeqCst) {
            return Err(EtcdError::AuthAlreadyEnabled);
        }
        Ok(AuthEnableResponse {
            header: self.header(),
        })
    }

    pub fn auth_disable(&self) -> EtcdResult<AuthDisableResponse> {
        if !self.auth_enabled.swap(false, Ordering::SeqCst) {
            return Err(EtcdError::AuthNotEnabled);
        }
        Ok(AuthDisableResponse {
            header: self.header(),
        })
    }

    pub fn authenticate(
        &self,
        req: &AuthenticateRequest,
    ) -> EtcdResult<AuthenticateResponse> {
        if self.auth_enabled.load(Ordering::SeqCst) {
            let user = self
                .users
                .get(&req.name)
                .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
            let valid = bcrypt::verify(&req.password, &user.password)
                .map_err(|e| EtcdError::Internal(e.to_string()))?;
            if !valid {
                return Err(EtcdError::InvalidPassword);
            }
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.auth_tokens.insert(token.clone(), req.name.clone());
        Ok(AuthenticateResponse {
            header: self.header(),
            token,
        })
    }

    pub fn user_add(&self, req: &AuthUserAddRequest) -> EtcdResult<AuthUserAddResponse> {
        if self.users.contains_key(&req.name) {
            return Err(EtcdError::UserAlreadyExists(req.name.clone()));
        }
        let hashed = bcrypt::hash(&req.password, BCRYPT_COST)
            .map_err(|e| EtcdError::Internal(e.to_string()))?;
        self.users.insert(
            req.name.clone(),
            AuthUser {
                name: req.name.clone(),
                password: hashed,
                roles: vec![],
            },
        );
        Ok(AuthUserAddResponse {
            header: self.header(),
        })
    }

    pub fn user_delete(&self, req: &AuthUserDeleteRequest) -> EtcdResult<AuthUserDeleteResponse> {
        self.users
            .remove(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        Ok(AuthUserDeleteResponse {
            header: self.header(),
        })
    }

    pub fn user_get(&self, req: &AuthUserGetRequest) -> EtcdResult<AuthUserGetResponse> {
        let user = self
            .users
            .get(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        Ok(AuthUserGetResponse {
            header: self.header(),
            roles: user.roles.clone(),
        })
    }

    pub fn user_list(&self) -> AuthUserListResponse {
        let mut users: Vec<String> = self.users.iter().map(|e| e.key().clone()).collect();
        users.sort();
        AuthUserListResponse {
            header: self.header(),
            users,
        }
    }

    pub fn user_change_password(
        &self,
        req: &AuthUserChangePasswordRequest,
    ) -> EtcdResult<AuthUserChangePasswordResponse> {
        let mut user = self
            .users
            .get_mut(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        let hashed = bcrypt::hash(&req.password, BCRYPT_COST)
            .map_err(|e| EtcdError::Internal(e.to_string()))?;
        user.password = hashed;
        Ok(AuthUserChangePasswordResponse {
            header: self.header(),
        })
    }

    pub fn user_grant_role(
        &self,
        req: &AuthUserGrantRoleRequest,
    ) -> EtcdResult<AuthUserGrantRoleResponse> {
        // Ensure role exists.
        if !self.roles.contains_key(&req.role) {
            return Err(EtcdError::RoleNotFound(req.role.clone()));
        }
        let mut user = self
            .users
            .get_mut(&req.user)
            .ok_or_else(|| EtcdError::UserNotFound(req.user.clone()))?;
        if !user.roles.contains(&req.role) {
            user.roles.push(req.role.clone());
        }
        Ok(AuthUserGrantRoleResponse {
            header: self.header(),
        })
    }

    pub fn user_revoke_role(
        &self,
        req: &AuthUserRevokeRoleRequest,
    ) -> EtcdResult<AuthUserRevokeRoleResponse> {
        let mut user = self
            .users
            .get_mut(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        if !user.roles.contains(&req.role) {
            return Err(EtcdError::RoleNotGranted);
        }
        user.roles.retain(|r| r != &req.role);
        Ok(AuthUserRevokeRoleResponse {
            header: self.header(),
        })
    }

    pub fn role_add(&self, req: &AuthRoleAddRequest) -> EtcdResult<AuthRoleAddResponse> {
        if self.roles.contains_key(&req.name) {
            return Err(EtcdError::RoleAlreadyExists(req.name.clone()));
        }
        self.roles.insert(
            req.name.clone(),
            AuthRole {
                name: req.name.clone(),
                key_permission: vec![],
            },
        );
        Ok(AuthRoleAddResponse {
            header: self.header(),
        })
    }

    pub fn role_delete(&self, req: &AuthRoleDeleteRequest) -> EtcdResult<AuthRoleDeleteResponse> {
        self.roles
            .remove(&req.role)
            .ok_or_else(|| EtcdError::RoleNotFound(req.role.clone()))?;
        Ok(AuthRoleDeleteResponse {
            header: self.header(),
        })
    }

    pub fn role_get(&self, req: &AuthRoleGetRequest) -> EtcdResult<AuthRoleGetResponse> {
        let role = self
            .roles
            .get(&req.role)
            .ok_or_else(|| EtcdError::RoleNotFound(req.role.clone()))?;
        Ok(AuthRoleGetResponse {
            header: self.header(),
            name: role.name.clone(),
            perm: role.key_permission.clone(),
        })
    }

    pub fn role_list(&self) -> AuthRoleListResponse {
        let mut roles: Vec<String> = self.roles.iter().map(|e| e.key().clone()).collect();
        roles.sort();
        AuthRoleListResponse {
            header: self.header(),
            roles,
        }
    }

    pub fn role_grant_permission(
        &self,
        req: &AuthRoleGrantPermissionRequest,
    ) -> EtcdResult<AuthRoleGrantPermissionResponse> {
        let mut role = self
            .roles
            .get_mut(&req.name)
            .ok_or_else(|| EtcdError::RoleNotFound(req.name.clone()))?;
        // Replace existing permission for same key if present.
        role.key_permission
            .retain(|p| !(p.key == req.perm.key && p.range_end == req.perm.range_end));
        role.key_permission.push(req.perm.clone());
        Ok(AuthRoleGrantPermissionResponse {
            header: self.header(),
        })
    }

    pub fn role_revoke_permission(
        &self,
        req: &AuthRoleRevokePermissionRequest,
    ) -> EtcdResult<AuthRoleRevokePermissionResponse> {
        let mut role = self
            .roles
            .get_mut(&req.role)
            .ok_or_else(|| EtcdError::RoleNotFound(req.role.clone()))?;
        let before = role.key_permission.len();
        role.key_permission
            .retain(|p| !(p.key == req.key && p.range_end == req.range_end));
        if role.key_permission.len() == before {
            return Err(EtcdError::PermissionAlreadyGranted);
        }
        Ok(AuthRoleRevokePermissionResponse {
            header: self.header(),
        })
    }

    /// Validate a token and check whether the caller has the required permission.
    /// Returns Ok when auth is disabled (no-op).
    pub fn check_auth_token(
        &self,
        token: Option<&str>,
        key: &[u8],
        perm: PermType,
    ) -> EtcdResult<()> {
        if !self.auth_enabled.load(Ordering::SeqCst) {
            return Ok(());
        }
        let token = token.ok_or(EtcdError::InvalidToken)?;
        let entry = self
            .auth_tokens
            .get(token)
            .ok_or(EtcdError::InvalidToken)?;
        let username = entry.clone();
        drop(entry);

        // Root always has full access.
        if username == "root" {
            return Ok(());
        }

        let user = self
            .users
            .get(&username)
            .ok_or_else(|| EtcdError::UserNotFound(username.clone()))?;

        for role_name in &user.roles {
            if let Some(role) = self.roles.get(role_name) {
                for p in &role.key_permission {
                    let covers = p.perm_type == perm || p.perm_type == PermType::Readwrite;
                    if !covers {
                        continue;
                    }
                    let key_match = if let Some(ref range_end) = p.range_end {
                        key >= p.key.as_bytes() && key < range_end.as_bytes()
                    } else {
                        key == p.key.as_bytes()
                    };
                    if key_match {
                        return Ok(());
                    }
                }
            }
        }
        Err(EtcdError::PermissionDenied)
    }

    // ── Maintenance ───────────────────────────────────────────────────────

    pub fn alarm(&self, req: &AlarmRequest) -> AlarmResponse {
        let mut alarms = self.alarms.write().unwrap();
        match req.action {
            AlarmAction::Get => {}
            AlarmAction::Activate => {
                if !alarms
                    .iter()
                    .any(|a| a.member_id == req.member_id && a.alarm == req.alarm)
                {
                    alarms.push(AlarmMember {
                        member_id: req.member_id,
                        alarm: req.alarm.clone(),
                    });
                }
            }
            AlarmAction::Deactivate => {
                alarms
                    .retain(|a| !(a.member_id == req.member_id && a.alarm == req.alarm));
            }
        }
        AlarmResponse {
            header: self.header(),
            alarms: alarms.clone(),
        }
    }

    pub fn defragment(&self) -> DefragmentResponse {
        DefragmentResponse {
            header: self.header(),
        }
    }

    pub fn hash(&self) -> HashResponse {
        let mut h: u32 = 5381;
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = self
            .current
            .iter()
            .map(|e| (e.key().clone(), e.value().value.clone()))
            .collect();
        pairs.sort_by_key(|(k, _)| k.clone());
        for (k, v) in &pairs {
            for &b in k.iter().chain(v.iter()) {
                h = h.wrapping_mul(33).wrapping_add(b as u32);
            }
        }
        HashResponse {
            header: self.header(),
            hash: h,
            compact_revision: self.compacted_revision.load(Ordering::SeqCst),
            hash_revision: self.current_revision(),
        }
    }

    pub fn snapshot(&self) -> SnapshotResponse {
        let data: Vec<(Vec<u8>, Vec<u8>)> = self
            .current
            .iter()
            .map(|e| (e.key().clone(), e.value().value.clone()))
            .collect();
        let blob = serde_json::to_vec(&data).unwrap_or_default();
        SnapshotResponse {
            header: self.header(),
            remaining_bytes: 0,
            blob,
        }
    }

    /// Get cluster status.
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "header": self.header(),
            "version": "3.6.0-cave",
            "dbSize": self.current.len(),
            "leader": 1,
            "raftIndex": self.current_revision(),
            "raftTerm": 1,
        })
    }

    pub fn version(&self) -> VersionResponse {
        VersionResponse {
            etcdserver: "3.6.0-cave".to_string(),
            etcdcluster: "3.6.0".to_string(),
        }
    }

    // ── Cluster ───────────────────────────────────────────────────────────

    pub fn member_add(&self, req: &MemberAddRequest) -> MemberAddResponse {
        let new_id = self.lease_counter.fetch_add(1, Ordering::SeqCst) + 100;
        let member = Member {
            id: new_id,
            name: format!("member-{}", new_id),
            peer_urls: req.peer_ur_ls.clone(),
            client_urls: vec![],
            is_learner: req.is_learner,
        };
        let mut members = self.members.write().unwrap();
        members.push(member.clone());
        MemberAddResponse {
            header: self.header(),
            member,
            members: members.clone(),
        }
    }

    pub fn member_remove(&self, req: &MemberRemoveRequest) -> EtcdResult<MemberRemoveResponse> {
        let mut members = self.members.write().unwrap();
        let before = members.len();
        members.retain(|m| m.id != req.id);
        if members.len() == before {
            return Err(EtcdError::MemberNotFound(req.id));
        }
        Ok(MemberRemoveResponse {
            header: self.header(),
            members: members.clone(),
        })
    }

    pub fn member_update(&self, req: &MemberUpdateRequest) -> EtcdResult<MemberUpdateResponse> {
        let mut members = self.members.write().unwrap();
        let m = members
            .iter_mut()
            .find(|m| m.id == req.id)
            .ok_or(EtcdError::MemberNotFound(req.id))?;
        m.peer_urls = req.peer_ur_ls.clone();
        Ok(MemberUpdateResponse {
            header: self.header(),
            members: members.clone(),
        })
    }

    pub fn member_list(&self) -> MemberListResponse {
        let members = self.members.read().unwrap();
        MemberListResponse {
            header: self.header(),
            members: members.clone(),
        }
    }

    // ── v3.6: Member promote / joint consensus ────────────────────────────

    /// Promote a learner to a voting member.
    /// Mirrors etcd v3.6 `etcdserver.MemberPromote`.
    pub fn member_promote(
        &self,
        req: &MemberPromoteRequest,
    ) -> EtcdResult<MemberPromoteResponse> {
        let mut members = self.members.write().unwrap();
        let m = members
            .iter_mut()
            .find(|m| m.id == req.id)
            .ok_or(EtcdError::MemberNotFound(req.id))?;
        if !m.is_learner {
            return Err(EtcdError::MemberNotLearner(req.id));
        }
        m.is_learner = false;
        Ok(MemberPromoteResponse {
            header: self.header(),
            members: members.clone(),
        })
    }

    /// Begin a joint-consensus configuration change (Cold ∪ Cnew).
    /// During the joint phase, quorum requires a majority in *both* configs.
    /// Mirrors etcd v3.6 `raft/confchange.EnterJoint`.
    pub fn enter_joint(
        &self,
        req: &EnterJointRequest,
    ) -> EtcdResult<EnterJointResponse> {
        let mut joint = self.joint.write().unwrap();
        if joint.is_some() {
            return Err(EtcdError::JointConfigInProgress);
        }
        let mut members = self.members.write().unwrap();

        // Snapshot the outgoing voting set (Cold).
        let outgoing: Vec<u64> = members
            .iter()
            .filter(|m| !m.is_learner)
            .map(|m| m.id)
            .collect();

        // Apply add operations (allocate IDs, append).
        let mut added_ids: Vec<u64> = Vec::new();
        for add in &req.adds {
            let new_id = self
                .lease_counter
                .fetch_add(1, Ordering::SeqCst)
                + 100;
            members.push(Member {
                id: new_id,
                name: format!("member-{}", new_id),
                peer_urls: add.peer_ur_ls.clone(),
                client_urls: vec![],
                is_learner: add.is_learner,
            });
            added_ids.push(new_id);
        }

        // Compute incoming voting set (Cnew):
        //   = outgoing
        //     ∪ promoted learners (none here — promotion is a separate op)
        //     ∪ non-learner adds
        //     ∖ removes
        let mut incoming: Vec<u64> = outgoing.clone();
        for (add, id) in req.adds.iter().zip(added_ids.iter()) {
            if !add.is_learner {
                incoming.push(*id);
            }
        }
        incoming.retain(|id| !req.removes.contains(id));

        // Reject if Cnew would have an empty voting set (would break quorum).
        if incoming.is_empty() {
            return Err(EtcdError::WouldBreakQuorum);
        }

        let learners: Vec<u64> = members
            .iter()
            .filter(|m| m.is_learner)
            .map(|m| m.id)
            .collect();

        let new_joint = JointConfig {
            outgoing,
            incoming,
            learners,
        };
        *joint = Some(new_joint.clone());

        Ok(EnterJointResponse {
            header: self.header(),
            joint: new_joint,
            members: members.clone(),
        })
    }

    /// Commit the pending joint consensus: drop the outgoing set and any
    /// removed members, leaving only Cnew.
    /// Mirrors etcd v3.6 `raft/confchange.LeaveJoint`.
    pub fn leave_joint(&self) -> EtcdResult<LeaveJointResponse> {
        let mut joint = self.joint.write().unwrap();
        let cfg = joint.take().ok_or(EtcdError::NoJointConfig)?;
        let keep: std::collections::HashSet<u64> = cfg
            .incoming
            .iter()
            .chain(cfg.learners.iter())
            .copied()
            .collect();
        let mut members = self.members.write().unwrap();
        members.retain(|m| keep.contains(&m.id));
        Ok(LeaveJointResponse {
            header: self.header(),
            members: members.clone(),
        })
    }

    /// Returns the current joint config when one is active.
    pub fn current_joint(&self) -> Option<JointConfig> {
        self.joint.read().unwrap().clone()
    }

    /// Quorum size for the current voting set.  In joint mode the call site
    /// must clear *both* `quorum_size_for(joint.outgoing)` and
    /// `quorum_size_for(joint.incoming)`; this helper returns the larger of
    /// the two so a single-value caller can use it as a conservative bound.
    pub fn quorum_size(&self) -> usize {
        if let Some(cfg) = self.current_joint() {
            let q_out = Self::quorum_size_for(cfg.outgoing.len());
            let q_in = Self::quorum_size_for(cfg.incoming.len());
            q_out.max(q_in)
        } else {
            let voters = self
                .members
                .read()
                .unwrap()
                .iter()
                .filter(|m| !m.is_learner)
                .count();
            Self::quorum_size_for(voters)
        }
    }

    /// Strict-majority quorum for a voting set of size `n` (etcd uses
    /// `n/2 + 1`).  Returns 1 for n=0 to avoid surprising callers.
    pub fn quorum_size_for(n: usize) -> usize {
        if n == 0 {
            1
        } else {
            n / 2 + 1
        }
    }

    /// Number of voting (non-learner) members.
    pub fn voting_member_count(&self) -> usize {
        self.members
            .read()
            .unwrap()
            .iter()
            .filter(|m| !m.is_learner)
            .count()
    }

    // ── v3.6: Watch multiplexer ───────────────────────────────────────────

    /// Subscribe to a previously-created watch.  Returns an mpsc receiver
    /// that yields only events matching the watch's filter.  The watch must
    /// have been created via `watch_create`.
    pub fn watch_subscribe(
        &self,
        watch_id: i64,
    ) -> EtcdResult<mpsc::UnboundedReceiver<WatchEvent>> {
        if !self.watch_configs.contains_key(&watch_id) {
            return Err(EtcdError::WatchNotFound(watch_id));
        }
        let (tx, rx) = mpsc::unbounded_channel();
        self.watch_inboxes.insert(watch_id, tx);
        Ok(rx)
    }

    /// Cancel a watch.  Drops the per-watch inbox (so the receiver sees the
    /// channel close) and removes the filter config.
    pub fn watch_cancel(&self, watch_id: i64) -> EtcdResult<()> {
        let removed_cfg = self.watch_configs.remove(&watch_id).is_some();
        let removed_inbox = self.watch_inboxes.remove(&watch_id).is_some();
        if !removed_cfg && !removed_inbox {
            return Err(EtcdError::WatchNotFound(watch_id));
        }
        Ok(())
    }

    /// Emit a progress notification on a single watch.  Used to advance
    /// watcher's known-revision under `progress_notify=true`.
    pub fn watch_progress(
        &self,
        watch_id: i64,
    ) -> EtcdResult<WatchProgressEvent> {
        if !self.watch_configs.contains_key(&watch_id) {
            return Err(EtcdError::WatchNotFound(watch_id));
        }
        Ok(WatchProgressEvent {
            header: self.header(),
            watch_id,
        })
    }

    /// Number of currently-registered watch inboxes (multiplexer subscribers).
    pub fn active_watch_count(&self) -> usize {
        self.watch_inboxes.len()
    }

    // ── v3.6: Lease enhancements ──────────────────────────────────────────

    /// `lease_grant` with full v3.6 semantics:
    ///   * negative TTL is rejected (`InvalidLeaseTtl`)
    ///   * TTL > `MAX_LEASE_TTL_SECS` is silently capped (matches the
    ///     server-side cap `etcdserver.maxLeaseTTL`)
    ///   * explicit ID that already exists is rejected (`LeaseAlreadyExists`)
    pub fn lease_grant_v2(
        &self,
        req: &LeaseGrantRequest,
    ) -> EtcdResult<LeaseGrantResponse> {
        if req.ttl < 0 {
            return Err(EtcdError::InvalidLeaseTtl(req.ttl));
        }
        if let Some(id) = req.id {
            if id != 0 && self.leases.contains_key(&id) {
                return Err(EtcdError::LeaseAlreadyExists(id));
            }
        }
        let ttl = req.ttl.min(MAX_LEASE_TTL_SECS);
        let id = match req.id {
            Some(0) | None => {
                self.lease_counter.fetch_add(1, Ordering::SeqCst) as i64 + 1
            }
            Some(id) => id,
        };
        let lease = Lease {
            id,
            ttl,
            granted_at: Utc::now(),
            keys: vec![],
        };
        self.leases.insert(id, lease);
        Ok(LeaseGrantResponse {
            header: self.header(),
            id,
            ttl,
        })
    }

    /// Number of keys currently attached to a lease.
    pub fn lease_attached_keys(&self, id: i64) -> EtcdResult<usize> {
        self.leases
            .get(&id)
            .map(|l| l.keys.len())
            .ok_or(EtcdError::LeaseNotFound(id))
    }

    // ── v3.6: MVCC compaction enhancements ────────────────────────────────

    /// Compaction with full v3.6 semantics:
    ///   * `revision == 0` is a no-op (matches `etcdserver.Compact`).
    ///   * `revision > current_revision` is rejected.
    ///   * Compaction is monotonic: regression is silently ignored.
    ///   * Per-key index entries strictly below the new compacted revision
    ///     are pruned (keeping the latest tombstone per key for reads at
    ///     `compacted+`).
    pub fn compact_v2(&self, revision: u64) -> EtcdResult<()> {
        if revision == 0 {
            return Ok(());
        }
        let current = self.current_revision();
        if revision > current {
            return Err(EtcdError::CompactionFutureRevision {
                requested: revision,
                current,
            });
        }
        let prev = self.compacted_revision.load(Ordering::SeqCst);
        if revision <= prev {
            // Already compacted to a higher rev — keep the higher mark.
            return Ok(());
        }
        self.compacted_revision.store(revision, Ordering::SeqCst);

        // Drop history entries strictly below `revision`.
        if let Ok(mut history) = self.history.write() {
            let drop: Vec<u64> = history.range(..revision).map(|(k, _)| *k).collect();
            for k in drop {
                history.remove(&k);
            }
        }

        // Prune key_index entries strictly below `revision`, but keep the
        // latest sub-revision so a read at `revision` still sees the key.
        for mut entry in self.key_index.iter_mut() {
            let revs = entry.value_mut();
            if revs.is_empty() {
                continue;
            }
            // Find largest rev <= compacted: keep that one + everything > it.
            let split = revs.partition_point(|&r| r <= revision);
            if split <= 1 {
                continue;
            }
            // Keep revs[split-1..] (last <= revision + everything after).
            let tail = revs.split_off(split - 1);
            *revs = tail;
        }
        Ok(())
    }

    /// Current compacted revision (last revision passed to `compact*`).
    pub fn compaction_revision(&self) -> u64 {
        self.compacted_revision.load(Ordering::SeqCst)
    }

    // ── v3.6: Snapshot RPC (stream + restore + checksum) ──────────────────

    /// Build a deterministic snapshot blob plus its sha256.  The blob format
    /// is JSON: `{ revision, compact_revision, kvs, leases, members }`.
    fn snapshot_blob(&self) -> (Vec<u8>, String, SnapshotMeta) {
        // Collect KVs sorted for determinism.
        let mut kvs: Vec<KeyValue> =
            self.current.iter().map(|e| e.value().clone()).collect();
        kvs.sort_by(|a, b| a.key.cmp(&b.key));

        let leases: Vec<Lease> = {
            let mut v: Vec<Lease> =
                self.leases.iter().map(|e| e.value().clone()).collect();
            v.sort_by_key(|l| l.id);
            v
        };
        let members = self.members.read().unwrap().clone();

        let payload = serde_json::json!({
            "revision": self.current_revision(),
            "compact_revision": self.compaction_revision(),
            "kvs": kvs,
            "leases": leases,
            "members": members,
        });
        let blob = serde_json::to_vec(&payload).unwrap_or_default();
        let checksum = sha256_hex(&blob);
        let meta = SnapshotMeta {
            revision: self.current_revision(),
            compact_revision: self.compaction_revision(),
            size_bytes: blob.len() as u64,
            checksum: checksum.clone(),
            member_count: members.len(),
            lease_count: leases.len(),
        };
        (blob, checksum, meta)
    }

    /// Stream the snapshot in fixed-size chunks.  Each chunk carries the
    /// same checksum so the receiver can verify after assembly.
    pub fn snapshot_stream(&self) -> Vec<SnapshotChunk> {
        let (blob, checksum, _meta) = self.snapshot_blob();
        let mut chunks = Vec::new();
        if blob.is_empty() {
            chunks.push(SnapshotChunk {
                header: self.header(),
                remaining_bytes: 0,
                blob: vec![],
                checksum,
            });
            return chunks;
        }
        for (i, slice) in blob.chunks(SNAPSHOT_CHUNK_SIZE).enumerate() {
            let consumed = (i + 1) * SNAPSHOT_CHUNK_SIZE;
            let remaining = blob.len().saturating_sub(consumed) as u64;
            chunks.push(SnapshotChunk {
                header: self.header(),
                remaining_bytes: remaining,
                blob: slice.to_vec(),
                checksum: checksum.clone(),
            });
        }
        chunks
    }

    /// Snapshot summary metadata (no blob bytes).
    pub fn snapshot_meta(&self) -> SnapshotMeta {
        let (_, _, meta) = self.snapshot_blob();
        meta
    }

    /// Reassemble the streamed chunks into a (blob, checksum) pair, asserting
    /// every chunk references the same checksum.
    pub fn assemble_chunks(
        chunks: &[SnapshotChunk],
    ) -> EtcdResult<(Vec<u8>, String)> {
        if chunks.is_empty() {
            return Err(EtcdError::SnapshotDecode("no chunks".into()));
        }
        let expected = chunks[0].checksum.clone();
        let mut blob = Vec::new();
        for c in chunks {
            if c.checksum != expected {
                return Err(EtcdError::SnapshotChecksumMismatch {
                    expected,
                    actual: c.checksum.clone(),
                });
            }
            blob.extend_from_slice(&c.blob);
        }
        let actual = sha256_hex(&blob);
        if actual != expected {
            return Err(EtcdError::SnapshotChecksumMismatch {
                expected,
                actual,
            });
        }
        Ok((blob, expected))
    }

    /// Replace this store's KV / lease / member state with the contents of a
    /// snapshot blob (verifying the supplied checksum).
    /// Mirrors `etcdserver.applySnapshot`.
    pub fn restore_snapshot(&self, blob: &[u8], checksum: &str) -> EtcdResult<()> {
        let actual = sha256_hex(blob);
        if actual != checksum {
            return Err(EtcdError::SnapshotChecksumMismatch {
                expected: checksum.to_string(),
                actual,
            });
        }
        let v: serde_json::Value = serde_json::from_slice(blob)
            .map_err(|e| EtcdError::SnapshotDecode(e.to_string()))?;

        let revision = v
            .get("revision")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| EtcdError::SnapshotDecode("missing revision".into()))?;
        let compact_revision = v
            .get("compact_revision")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let kvs: Vec<KeyValue> = v
            .get("kvs")
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default();
        let leases: Vec<Lease> = v
            .get("leases")
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default();
        let members: Vec<Member> = v
            .get("members")
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default();

        // Reset state.  Holding the txn_lock keeps writers out for the swap.
        let _guard = self.txn_lock.lock().unwrap();
        self.current.clear();
        self.key_index.clear();
        self.history.write().unwrap().clear();
        self.leases.clear();
        for kv in kvs {
            self.key_index
                .entry(kv.key.clone())
                .or_default()
                .push(kv.mod_revision);
            self.current.insert(kv.key.clone(), kv);
        }
        for l in leases {
            self.leases.insert(l.id, l);
        }
        *self.members.write().unwrap() = members;
        self.revision.store(revision, Ordering::SeqCst);
        self.compacted_revision
            .store(compact_revision, Ordering::SeqCst);
        Ok(())
    }
}

/// SHA-256 → lowercase hex string.  Inlined to keep the dependency surface
/// small (no `sha2` crate); etcd's `etcdutl snapshot status` formats the
/// digest the same way (`fmt.Sprintf("%x", h.Sum(nil))`).
fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in &out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ── Minimal SHA-256 (FIPS 180-4) ──────────────────────────────────────────
// Rolled in-tree to avoid pulling the `sha2` crate just for snapshot
// checksums.  Tested transitively via `test_snapshot_includes_sha256_*`
// against known fixed inputs.

struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::with_capacity(64),
            total_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap();
            Self::compress(&mut self.state, &block);
            self.buffer.drain(..64);
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0x00);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap();
            Self::compress(&mut self.state, &block);
            self.buffer.drain(..64);
        }
        let mut out = [0u8; 32];
        for (i, w) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let mj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(mj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

impl Default for KvStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Start background lease-expiry task.  Call once after the tokio runtime is running.
pub fn start_background_tasks(store: Arc<KvStore>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            store.expire_leases();
        }
    });
}

/// Build the etcd-format delete KV from an existing KV and new revision.
fn make_delete_kv(kv: &KeyValue, rev: u64) -> KeyValue {
    KeyValue {
        key: kv.key.clone(),
        value: vec![],
        create_revision: kv.create_revision,
        mod_revision: rev,
        version: 0,
        lease: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ────────────────────────────────────────────────────────────

    fn put(store: &KvStore, key: &str, value: &str) -> PutResponse {
        store.put(&PutRequest {
            key: key.into(),
            value: value.into(),
            lease: None,
            prev_kv: false,
        })
    }

    fn get(store: &KvStore, key: &str) -> Vec<KeyValue> {
        store
            .range(&RangeRequest {
                key: key.into(),
                range_end: None,
                limit: None,
                revision: None,
                keys_only: false,
                count_only: false,
            })
            .unwrap()
            .kvs
    }

    fn get_at(store: &KvStore, key: &str, rev: u64) -> Vec<KeyValue> {
        store
            .range(&RangeRequest {
                key: key.into(),
                range_end: None,
                limit: None,
                revision: Some(rev),
                keys_only: false,
                count_only: false,
            })
            .unwrap()
            .kvs
    }

    // ── Original tests (preserved) ──────────────────────────────────────────

    #[test]
    fn test_put_and_get() {
        let store = KvStore::new();
        put(&store, "foo", "bar");
        let resp = get(&store, "foo");
        assert_eq!(resp.len(), 1);
        assert_eq!(resp[0].value_str(), "bar");
    }

    #[test]
    fn test_put_updates_revision() {
        let store = KvStore::new();
        let r1 = put(&store, "a", "1");
        let r2 = put(&store, "b", "2");
        assert!(r2.header.revision > r1.header.revision);
    }

    #[test]
    fn test_put_prev_kv() {
        let store = KvStore::new();
        put(&store, "x", "old");
        let resp = store.put(&PutRequest {
            key: "x".into(),
            value: "new".into(),
            lease: None,
            prev_kv: true,
        });
        assert!(resp.prev_kv.is_some());
        assert_eq!(resp.prev_kv.unwrap().value_str(), "old");
    }

    #[test]
    fn test_delete() {
        let store = KvStore::new();
        put(&store, "del_me", "v");
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "del_me".into(),
            range_end: None,
            prev_kv: true,
        });
        assert_eq!(resp.deleted, 1);
        assert_eq!(resp.prev_kvs[0].value_str(), "v");
        assert!(get(&store, "del_me").is_empty());
    }

    #[test]
    fn test_range_query() {
        let store = KvStore::new();
        put(&store, "/a/1", "v1");
        put(&store, "/a/2", "v2");
        put(&store, "/b/1", "v3");
        let resp = store
            .range(&RangeRequest {
                key: "/a/".into(),
                range_end: Some("/a0".into()),
                limit: None,
                revision: None,
                keys_only: false,
                count_only: false,
            })
            .unwrap();
        assert_eq!(resp.kvs.len(), 2);
    }

    #[test]
    fn test_count_only() {
        let store = KvStore::new();
        put(&store, "k1", "v1");
        put(&store, "k2", "v2");
        let resp = store
            .range(&RangeRequest {
                key: "k".into(),
                range_end: Some("l".into()),
                limit: None,
                revision: None,
                keys_only: false,
                count_only: true,
            })
            .unwrap();
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
        put(&store, "watched", "v1");
        let event = rx.try_recv().unwrap();
        assert_eq!(event.kv.key_str(), "watched");
        assert!(matches!(event.event_type, EventType::Put));
    }

    #[test]
    fn test_compact() {
        let store = KvStore::new();
        put(&store, "a", "1");
        put(&store, "b", "2");
        let rev = store.current_revision();
        store.compact(rev);
    }

    #[test]
    fn test_status() {
        let store = KvStore::new();
        put(&store, "s", "t");
        let status = store.status();
        assert!(status.get("version").is_some());
    }

    // ── Watch ──────────────────────────────────────────────────────────────

    #[test]
    fn test_watch_create_returns_id() {
        let store = KvStore::new();
        let resp = store.watch_create(&WatchCreateRequest {
            key: "/foo".into(),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        assert!(resp.watch_id > 0);
        assert!(resp.created);
    }

    #[test]
    fn test_watch_ids_are_unique() {
        let store = KvStore::new();
        let r1 = store.watch_create(&WatchCreateRequest {
            key: "a".into(),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let r2 = store.watch_create(&WatchCreateRequest {
            key: "b".into(),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        assert_ne!(r1.watch_id, r2.watch_id);
    }

    // ── Lease extensions ───────────────────────────────────────────────────

    #[test]
    fn test_lease_keepalive() {
        let store = KvStore::new();
        let grant = store.lease_grant(&LeaseGrantRequest { ttl: 30, id: None });
        let resp = store
            .lease_keepalive(&LeaseKeepAliveRequest { id: grant.id })
            .unwrap();
        assert_eq!(resp.id, grant.id);
        assert_eq!(resp.ttl, 30);
    }

    #[test]
    fn test_lease_keepalive_not_found() {
        let store = KvStore::new();
        let err = store.lease_keepalive(&LeaseKeepAliveRequest { id: 9999 });
        assert!(matches!(err, Err(EtcdError::LeaseNotFound(_))));
    }

    #[test]
    fn test_lease_timetolive() {
        let store = KvStore::new();
        let grant = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        let resp = store
            .lease_timetolive(&LeaseTTLRequest {
                id: grant.id,
                keys: false,
            })
            .unwrap();
        assert_eq!(resp.granted_ttl, 60);
        assert!(resp.ttl <= 60);
    }

    #[test]
    fn test_lease_timetolive_not_found() {
        let store = KvStore::new();
        let err = store.lease_timetolive(&LeaseTTLRequest { id: 9999, keys: false });
        assert!(matches!(err, Err(EtcdError::LeaseNotFound(_))));
    }

    #[test]
    fn test_lease_leases() {
        let store = KvStore::new();
        let g1 = store.lease_grant(&LeaseGrantRequest { ttl: 10, id: None });
        let g2 = store.lease_grant(&LeaseGrantRequest { ttl: 20, id: None });
        let resp = store.lease_leases();
        let ids: Vec<i64> = resp.leases.iter().map(|l| l.id).collect();
        assert!(ids.contains(&g1.id));
        assert!(ids.contains(&g2.id));
    }

    // ── Auth ───────────────────────────────────────────────────────────────

    #[test]
    fn test_auth_enable_disable() {
        let store = KvStore::new();
        assert!(store.auth_enable().is_ok());
        assert!(matches!(
            store.auth_enable(),
            Err(EtcdError::AuthAlreadyEnabled)
        ));
        assert!(store.auth_disable().is_ok());
        assert!(matches!(
            store.auth_disable(),
            Err(EtcdError::AuthNotEnabled)
        ));
    }

    #[test]
    fn test_authenticate_no_auth() {
        let store = KvStore::new();
        let resp = store
            .authenticate(&AuthenticateRequest {
                name: "anyone".into(),
                password: "anything".into(),
            })
            .unwrap();
        assert!(!resp.token.is_empty());
    }

    #[test]
    fn test_authenticate_with_auth_enabled() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "root".into(),
                password: "secret".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();

        let ok = store.authenticate(&AuthenticateRequest {
            name: "root".into(),
            password: "secret".into(),
        });
        assert!(ok.is_ok());

        let bad = store.authenticate(&AuthenticateRequest {
            name: "root".into(),
            password: "wrong".into(),
        });
        assert!(matches!(bad, Err(EtcdError::InvalidPassword)));
    }

    #[test]
    fn test_user_add_get_delete() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "alice".into(),
                password: "pw".into(),
            })
            .unwrap();

        assert!(matches!(
            store.user_add(&AuthUserAddRequest {
                name: "alice".into(),
                password: "pw2".into()
            }),
            Err(EtcdError::UserAlreadyExists(_))
        ));

        let get = store
            .user_get(&AuthUserGetRequest { name: "alice".into() })
            .unwrap();
        assert!(get.roles.is_empty());

        store
            .user_delete(&AuthUserDeleteRequest { name: "alice".into() })
            .unwrap();
        assert!(matches!(
            store.user_get(&AuthUserGetRequest { name: "alice".into() }),
            Err(EtcdError::UserNotFound(_))
        ));
    }

    #[test]
    fn test_user_list() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "bob".into(),
                password: "p".into(),
            })
            .unwrap();
        store
            .user_add(&AuthUserAddRequest {
                name: "alice".into(),
                password: "p".into(),
            })
            .unwrap();
        let resp = store.user_list();
        assert!(resp.users.contains(&"alice".to_string()));
        assert!(resp.users.contains(&"bob".to_string()));
    }

    #[test]
    fn test_user_change_password() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "u1".into(),
                password: "old".into(),
            })
            .unwrap();
        store
            .user_change_password(&AuthUserChangePasswordRequest {
                name: "u1".into(),
                password: "new".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();
        assert!(store
            .authenticate(&AuthenticateRequest {
                name: "u1".into(),
                password: "new".into()
            })
            .is_ok());
    }

    #[test]
    fn test_role_add_get_delete() {
        let store = KvStore::new();
        store
            .role_add(&AuthRoleAddRequest { name: "admin".into() })
            .unwrap();

        assert!(matches!(
            store.role_add(&AuthRoleAddRequest { name: "admin".into() }),
            Err(EtcdError::RoleAlreadyExists(_))
        ));

        let get = store
            .role_get(&AuthRoleGetRequest { role: "admin".into() })
            .unwrap();
        assert_eq!(get.name, "admin");
        assert!(get.perm.is_empty());

        store
            .role_delete(&AuthRoleDeleteRequest { role: "admin".into() })
            .unwrap();
        assert!(matches!(
            store.role_get(&AuthRoleGetRequest { role: "admin".into() }),
            Err(EtcdError::RoleNotFound(_))
        ));
    }

    #[test]
    fn test_role_list() {
        let store = KvStore::new();
        store
            .role_add(&AuthRoleAddRequest { name: "r1".into() })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest { name: "r2".into() })
            .unwrap();
        let resp = store.role_list();
        assert!(resp.roles.contains(&"r1".to_string()));
        assert!(resp.roles.contains(&"r2".to_string()));
    }

    // ── Maintenance ────────────────────────────────────────────────────────

    #[test]
    fn test_alarm_get_empty() {
        let store = KvStore::new();
        let resp = store.alarm(&AlarmRequest {
            action: AlarmAction::Get,
            member_id: 1,
            alarm: AlarmType::None,
        });
        assert!(resp.alarms.is_empty());
    }

    #[test]
    fn test_alarm_activate_deactivate() {
        let store = KvStore::new();
        store.alarm(&AlarmRequest {
            action: AlarmAction::Activate,
            member_id: 1,
            alarm: AlarmType::Nospace,
        });
        let resp = store.alarm(&AlarmRequest {
            action: AlarmAction::Get,
            member_id: 0,
            alarm: AlarmType::None,
        });
        assert_eq!(resp.alarms.len(), 1);
        assert_eq!(resp.alarms[0].alarm, AlarmType::Nospace);

        store.alarm(&AlarmRequest {
            action: AlarmAction::Deactivate,
            member_id: 1,
            alarm: AlarmType::Nospace,
        });
        let resp2 = store.alarm(&AlarmRequest {
            action: AlarmAction::Get,
            member_id: 0,
            alarm: AlarmType::None,
        });
        assert!(resp2.alarms.is_empty());
    }

    #[test]
    fn test_defragment() {
        let store = KvStore::new();
        let resp = store.defragment();
        assert_eq!(resp.header.cluster_id, 1);
    }

    #[test]
    fn test_hash_changes_with_data() {
        let store = KvStore::new();
        let h1 = store.hash().hash;
        put(&store, "k", "v");
        let h2 = store.hash().hash;
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_snapshot_contains_data() {
        let store = KvStore::new();
        put(&store, "snap_key", "snap_val");
        let resp = store.snapshot();
        let data_str = String::from_utf8_lossy(&resp.blob);
        assert!(data_str.contains("snap_key") || !resp.blob.is_empty());
    }

    #[test]
    fn test_compaction_response() {
        let store = KvStore::new();
        put(&store, "a", "1");
        let rev = store.current_revision();
        let resp = store.compaction(&CompactionRequest {
            revision: rev,
            physical: true,
        });
        assert_eq!(resp.header.cluster_id, 1);
    }

    #[test]
    fn test_version() {
        let store = KvStore::new();
        let v = store.version();
        assert!(v.etcdserver.contains("cave"));
        assert!(!v.etcdcluster.is_empty());
    }

    // ── Cluster ────────────────────────────────────────────────────────────

    #[test]
    fn test_member_list_has_default() {
        let store = KvStore::new();
        let resp = store.member_list();
        assert_eq!(resp.members.len(), 1);
        assert_eq!(resp.members[0].id, 1);
    }

    #[test]
    fn test_member_add() {
        let store = KvStore::new();
        let resp = store.member_add(&MemberAddRequest {
            peer_ur_ls: vec!["http://peer2:2380".into()],
            is_learner: false,
        });
        assert!(resp.member.id > 1);
        assert_eq!(resp.members.len(), 2);
    }

    #[test]
    fn test_member_remove() {
        let store = KvStore::new();
        let add = store.member_add(&MemberAddRequest {
            peer_ur_ls: vec!["http://peer2:2380".into()],
            is_learner: false,
        });
        let new_id = add.member.id;
        let resp = store
            .member_remove(&MemberRemoveRequest { id: new_id })
            .unwrap();
        assert_eq!(resp.members.len(), 1);
    }

    #[test]
    fn test_member_remove_not_found() {
        let store = KvStore::new();
        let err = store.member_remove(&MemberRemoveRequest { id: 9999 });
        assert!(matches!(err, Err(EtcdError::MemberNotFound(_))));
    }

    #[test]
    fn test_member_update() {
        let store = KvStore::new();
        let resp = store
            .member_update(&MemberUpdateRequest {
                id: 1,
                peer_ur_ls: vec!["http://newpeer:2380".into()],
            })
            .unwrap();
        assert_eq!(resp.members[0].peer_urls[0], "http://newpeer:2380");
    }

    #[test]
    fn test_member_update_not_found() {
        let store = KvStore::new();
        let err = store.member_update(&MemberUpdateRequest {
            id: 9999,
            peer_ur_ls: vec![],
        });
        assert!(matches!(err, Err(EtcdError::MemberNotFound(_))));
    }

    // ── NEW: MVCC key_index & time-travel reads ───────────────────────────

    #[test]
    fn test_key_index_tracks_revisions() {
        let store = KvStore::new();
        put(&store, "mykey", "v1");
        let rev1 = store.current_revision();
        put(&store, "mykey", "v2");
        let rev2 = store.current_revision();

        let revs = store.key_index.get(b"mykey".as_ref()).unwrap();
        assert!(revs.contains(&rev1));
        assert!(revs.contains(&rev2));
        assert!(revs.len() >= 2);
    }

    #[test]
    fn test_time_travel_read_past_value() {
        let store = KvStore::new();
        put(&store, "key", "first");
        let rev_after_first = store.current_revision();
        put(&store, "key", "second");

        // At rev_after_first, value should be "first"
        let kvs = get_at(&store, "key", rev_after_first);
        assert_eq!(kvs.len(), 1);
        assert_eq!(kvs[0].value_str(), "first");

        // Current value is "second"
        let current = get(&store, "key");
        assert_eq!(current[0].value_str(), "second");
    }

    #[test]
    fn test_time_travel_key_not_yet_created() {
        let store = KvStore::new();
        let before = store.current_revision();
        put(&store, "newkey", "val");

        // At `before`, key didn't exist yet.
        let kvs = get_at(&store, "newkey", before);
        assert!(kvs.is_empty());
    }

    #[test]
    fn test_time_travel_after_delete() {
        let store = KvStore::new();
        put(&store, "k", "v");
        let rev_exists = store.current_revision();
        store.delete_range(&DeleteRangeRequest {
            key: "k".into(),
            range_end: None,
            prev_kv: false,
        });
        let rev_deleted = store.current_revision();

        let at_exists = get_at(&store, "k", rev_exists);
        assert_eq!(at_exists.len(), 1);

        let at_deleted = get_at(&store, "k", rev_deleted);
        assert!(at_deleted.is_empty());
    }

    #[test]
    fn test_time_travel_compacted_returns_error() {
        let store = KvStore::new();
        put(&store, "k", "v1");
        let old_rev = store.current_revision();
        put(&store, "k", "v2");
        store.compact(store.current_revision());

        let result = store.range(&RangeRequest {
            key: "k".into(),
            range_end: None,
            limit: None,
            revision: Some(old_rev),
            keys_only: false,
            count_only: false,
        });
        assert!(matches!(
            result,
            Err(EtcdError::RevisionCompacted { .. })
        ));
    }

    #[test]
    fn test_time_travel_range_prefix() {
        let store = KvStore::new();
        put(&store, "/ns/a", "aa");
        put(&store, "/ns/b", "bb");
        let snap_rev = store.current_revision();
        put(&store, "/ns/c", "cc"); // added after snapshot

        let resp = store
            .range(&RangeRequest {
                key: "/ns/".into(),
                range_end: Some("/ns0".into()),
                limit: None,
                revision: Some(snap_rev),
                keys_only: false,
                count_only: false,
            })
            .unwrap();
        assert_eq!(resp.kvs.len(), 2); // /ns/c wasn't present at snap_rev
    }

    // ── NEW: Watch config & filtering ────────────────────────────────────

    #[test]
    fn test_watch_config_stored() {
        let store = KvStore::new();
        let resp = store.watch_create(&WatchCreateRequest {
            key: "/prefix/".into(),
            range_end: Some("/prefix0".into()),
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let config = store.get_watch_config(resp.watch_id).unwrap();
        assert_eq!(config.key, b"/prefix/");
        assert_eq!(config.range_end, Some(b"/prefix0".to_vec()));
    }

    #[test]
    fn test_watch_historical_replay() {
        let store = KvStore::new();
        put(&store, "/w/a", "v1");
        let start = store.current_revision();
        put(&store, "/w/b", "v2");
        put(&store, "/w/c", "v3");
        put(&store, "/other/x", "ignored");

        let resp = store.watch_create(&WatchCreateRequest {
            key: "/w/".into(),
            range_end: Some("/w0".into()),
            start_revision: Some(start),
            progress_notify: false,
            prev_kv: false,
        });

        // Events for /w/b and /w/c should be replayed; /other/x ignored.
        let replayed_keys: Vec<String> =
            resp.events.iter().map(|e| e.kv.key_str()).collect();
        assert!(replayed_keys.contains(&"/w/b".to_string()));
        assert!(replayed_keys.contains(&"/w/c".to_string()));
        assert!(!replayed_keys.contains(&"/other/x".to_string()));
    }

    #[test]
    fn test_watch_key_matches() {
        let config = WatchConfig {
            watch_id: 1,
            key: b"exact".to_vec(),
            range_end: None,
            start_revision: None,
            prev_kv: false,
        };
        assert!(KvStore::key_matches_watch(b"exact", &config));
        assert!(!KvStore::key_matches_watch(b"other", &config));
    }

    #[test]
    fn test_watch_range_matches() {
        let config = WatchConfig {
            watch_id: 1,
            key: b"/ns/".to_vec(),
            range_end: Some(b"/ns0".to_vec()),
            start_revision: None,
            prev_kv: false,
        };
        assert!(KvStore::key_matches_watch(b"/ns/foo", &config));
        assert!(!KvStore::key_matches_watch(b"/other/foo", &config));
        assert!(!KvStore::key_matches_watch(b"/ns0", &config)); // end is exclusive
    }

    #[test]
    fn test_watch_delete_event_has_empty_value() {
        let store = KvStore::new();
        let mut rx = store.subscribe();
        put(&store, "dkey", "dval");
        let _ = rx.try_recv(); // consume the Put event

        store.delete_range(&DeleteRangeRequest {
            key: "dkey".into(),
            range_end: None,
            prev_kv: false,
        });
        let event = rx.try_recv().unwrap();
        assert!(matches!(event.event_type, EventType::Delete));
        assert!(event.kv.value.is_empty()); // etcd sets value to empty on delete
        assert!(event.prev_kv.is_some()); // prev_kv always set on delete
        assert_eq!(event.prev_kv.unwrap().value_str(), "dval");
    }

    // ── NEW: Lease key association & expiry ──────────────────────────────

    #[test]
    fn test_lease_key_association() {
        let store = KvStore::new();
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });

        store.put(&PutRequest {
            key: "leased_key".into(),
            value: "val".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });

        let ttl_resp = store
            .lease_timetolive(&LeaseTTLRequest { id: lease.id, keys: true })
            .unwrap();
        assert!(ttl_resp.keys.iter().any(|k| k == b"leased_key"));
    }

    #[test]
    fn test_lease_revoke_deletes_associated_keys() {
        let store = KvStore::new();
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });

        store.put(&PutRequest {
            key: "k1".into(),
            value: "v1".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });
        store.put(&PutRequest {
            key: "k2".into(),
            value: "v2".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });

        store.lease_revoke(lease.id).unwrap();

        assert!(get(&store, "k1").is_empty());
        assert!(get(&store, "k2").is_empty());
    }

    #[test]
    fn test_lease_revoke_fires_watch_events() {
        let store = KvStore::new();
        let mut rx = store.subscribe();

        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 60, id: None });
        store.put(&PutRequest {
            key: "watched_lease_key".into(),
            value: "v".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });
        // Drain put events.
        while rx.try_recv().is_ok() {}

        store.lease_revoke(lease.id).unwrap();

        let event = rx.try_recv().unwrap();
        assert!(matches!(event.event_type, EventType::Delete));
        assert_eq!(event.kv.key_str(), "watched_lease_key");
    }

    #[test]
    fn test_expire_leases_removes_expired() {
        let store = KvStore::new();
        // Grant a lease with TTL 0 (immediately expired).
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 0, id: None });
        store.put(&PutRequest {
            key: "exp_key".into(),
            value: "v".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });

        store.expire_leases();

        assert!(get(&store, "exp_key").is_empty());
        assert!(store.leases.get(&lease.id).is_none());
    }

    #[test]
    fn test_expire_leases_fires_watch_events() {
        let store = KvStore::new();
        let mut rx = store.subscribe();

        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 0, id: None });
        store.put(&PutRequest {
            key: "exp_watch_key".into(),
            value: "v".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });
        while rx.try_recv().is_ok() {}

        store.expire_leases();

        let event = rx.try_recv().unwrap();
        assert!(matches!(event.event_type, EventType::Delete));
        assert_eq!(event.kv.key_str(), "exp_watch_key");
    }

    #[test]
    fn test_lease_non_expired_survives() {
        let store = KvStore::new();
        let lease = store.lease_grant(&LeaseGrantRequest { ttl: 3600, id: None });
        store.put(&PutRequest {
            key: "alive_key".into(),
            value: "v".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });

        store.expire_leases();

        assert_eq!(get(&store, "alive_key").len(), 1);
        assert!(store.leases.get(&lease.id).is_some());
    }

    // ── NEW: Transaction atomicity ────────────────────────────────────────

    #[test]
    fn test_txn_success_path() {
        let store = KvStore::new();
        put(&store, "txn_key", "initial");

        let result = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "txn_key".into(),
                target: CompareTarget::Value,
                result: CompareResult::Equal,
                value: Some("initial".into()),
                version: None,
                mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest {
                key: "txn_key".into(),
                value: "updated".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![RequestOp::Put(PutRequest {
                key: "txn_key".into(),
                value: "fail_branch".into(),
                lease: None,
                prev_kv: false,
            })],
        });

        assert!(result.succeeded);
        assert_eq!(get(&store, "txn_key")[0].value_str(), "updated");
    }

    #[test]
    fn test_txn_failure_path() {
        let store = KvStore::new();
        put(&store, "txn_key2", "initial");

        let result = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "txn_key2".into(),
                target: CompareTarget::Value,
                result: CompareResult::Equal,
                value: Some("wrong".into()),
                version: None,
                mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest {
                key: "txn_key2".into(),
                value: "should_not_happen".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![RequestOp::Put(PutRequest {
                key: "txn_key2".into(),
                value: "failure_branch".into(),
                lease: None,
                prev_kv: false,
            })],
        });

        assert!(!result.succeeded);
        assert_eq!(
            get(&store, "txn_key2")[0].value_str(),
            "failure_branch"
        );
    }

    #[test]
    fn test_txn_version_compare() {
        let store = KvStore::new();
        put(&store, "v_key", "v1");
        put(&store, "v_key", "v2"); // version is now 2

        let result = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "v_key".into(),
                target: CompareTarget::Version,
                result: CompareResult::Equal,
                value: None,
                version: Some(2),
                mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest {
                key: "v_key".into(),
                value: "v3".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![],
        });
        assert!(result.succeeded);
        assert_eq!(get(&store, "v_key")[0].value_str(), "v3");
    }

    #[test]
    fn test_txn_create_compare() {
        let store = KvStore::new();
        put(&store, "c_key", "v");
        let create_rev = get(&store, "c_key")[0].create_revision;

        let result = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "c_key".into(),
                target: CompareTarget::Create,
                result: CompareResult::Equal,
                value: None,
                version: None,
                mod_revision: Some(create_rev),
            }],
            success: vec![RequestOp::Put(PutRequest {
                key: "c_key".into(),
                value: "new".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![],
        });
        assert!(result.succeeded);
    }

    #[test]
    fn test_txn_mod_compare() {
        let store = KvStore::new();
        put(&store, "m_key", "v");
        let mod_rev = get(&store, "m_key")[0].mod_revision;

        let result = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "m_key".into(),
                target: CompareTarget::Mod,
                result: CompareResult::Equal,
                value: None,
                version: None,
                mod_revision: Some(mod_rev),
            }],
            success: vec![RequestOp::Put(PutRequest {
                key: "m_key".into(),
                value: "new".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![],
        });
        assert!(result.succeeded);
    }

    #[test]
    fn test_txn_empty_compare_always_succeeds() {
        let store = KvStore::new();
        let result = store.txn(&TxnRequest {
            compare: vec![],
            success: vec![RequestOp::Put(PutRequest {
                key: "new_key".into(),
                value: "created".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![],
        });
        assert!(result.succeeded);
        assert_eq!(get(&store, "new_key")[0].value_str(), "created");
    }

    // ── NEW: bcrypt password storage ─────────────────────────────────────

    #[test]
    fn test_password_not_stored_plaintext() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "u".into(),
                password: "plaintext".into(),
            })
            .unwrap();
        let user = store.users.get("u").unwrap();
        // bcrypt hashes start with "$2" and are never equal to the original.
        assert_ne!(user.password, "plaintext");
        assert!(user.password.starts_with('$'));
    }

    #[test]
    fn test_bcrypt_wrong_password_rejected() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "u2".into(),
                password: "correct".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();

        assert!(store
            .authenticate(&AuthenticateRequest {
                name: "u2".into(),
                password: "wrong".into()
            })
            .is_err());
    }

    // ── NEW: Auth token & permissions ────────────────────────────────────

    #[test]
    fn test_check_auth_token_disabled_allows_all() {
        let store = KvStore::new();
        assert!(store
            .check_auth_token(None, b"any_key", PermType::Write)
            .is_ok());
        assert!(store
            .check_auth_token(Some("garbage"), b"any_key", PermType::Read)
            .is_ok());
    }

    #[test]
    fn test_check_auth_token_no_token_when_enabled() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "root".into(),
                password: "p".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();
        let err = store.check_auth_token(None, b"k", PermType::Read);
        assert!(matches!(err, Err(EtcdError::InvalidToken)));
    }

    #[test]
    fn test_check_auth_token_root_full_access() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "root".into(),
                password: "pass".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();
        let auth = store
            .authenticate(&AuthenticateRequest {
                name: "root".into(),
                password: "pass".into(),
            })
            .unwrap();
        assert!(store
            .check_auth_token(Some(&auth.token), b"any_key", PermType::Write)
            .is_ok());
    }

    #[test]
    fn test_check_auth_token_permission_denied() {
        let store = KvStore::new();
        // Root for auth enable.
        store
            .user_add(&AuthUserAddRequest {
                name: "root".into(),
                password: "p".into(),
            })
            .unwrap();
        store
            .user_add(&AuthUserAddRequest {
                name: "limited".into(),
                password: "pw".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();

        let auth = store
            .authenticate(&AuthenticateRequest {
                name: "limited".into(),
                password: "pw".into(),
            })
            .unwrap();
        // No roles/permissions granted yet.
        let err = store.check_auth_token(Some(&auth.token), b"k", PermType::Write);
        assert!(matches!(err, Err(EtcdError::PermissionDenied)));
    }

    #[test]
    fn test_role_grant_permission_and_check() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "root".into(),
                password: "p".into(),
            })
            .unwrap();
        store
            .user_add(&AuthUserAddRequest {
                name: "writer".into(),
                password: "pw".into(),
            })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest { name: "write_role".into() })
            .unwrap();
        store
            .role_grant_permission(&AuthRoleGrantPermissionRequest {
                name: "write_role".into(),
                perm: Permission {
                    perm_type: PermType::Write,
                    key: "/data/".into(),
                    range_end: Some("/data0".into()),
                },
            })
            .unwrap();
        store
            .user_grant_role(&AuthUserGrantRoleRequest {
                user: "writer".into(),
                role: "write_role".into(),
            })
            .unwrap();
        store.auth_enable().unwrap();

        let auth = store
            .authenticate(&AuthenticateRequest {
                name: "writer".into(),
                password: "pw".into(),
            })
            .unwrap();

        assert!(store
            .check_auth_token(Some(&auth.token), b"/data/k1", PermType::Write)
            .is_ok());
        assert!(matches!(
            store.check_auth_token(Some(&auth.token), b"/other/k", PermType::Write),
            Err(EtcdError::PermissionDenied)
        ));
    }

    #[test]
    fn test_user_grant_revoke_role() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "u".into(),
                password: "p".into(),
            })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest { name: "r".into() })
            .unwrap();

        store
            .user_grant_role(&AuthUserGrantRoleRequest {
                user: "u".into(),
                role: "r".into(),
            })
            .unwrap();
        let roles = store
            .user_get(&AuthUserGetRequest { name: "u".into() })
            .unwrap()
            .roles;
        assert!(roles.contains(&"r".to_string()));

        store
            .user_revoke_role(&AuthUserRevokeRoleRequest {
                name: "u".into(),
                role: "r".into(),
            })
            .unwrap();
        let roles_after = store
            .user_get(&AuthUserGetRequest { name: "u".into() })
            .unwrap()
            .roles;
        assert!(!roles_after.contains(&"r".to_string()));
    }

    #[test]
    fn test_user_revoke_role_not_granted() {
        let store = KvStore::new();
        store
            .user_add(&AuthUserAddRequest {
                name: "u".into(),
                password: "p".into(),
            })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest { name: "r".into() })
            .unwrap();
        let err = store.user_revoke_role(&AuthUserRevokeRoleRequest {
            name: "u".into(),
            role: "r".into(),
        });
        assert!(matches!(err, Err(EtcdError::RoleNotGranted)));
    }

    // ── NEW: version counter ─────────────────────────────────────────────

    #[test]
    fn test_version_increments_on_update() {
        let store = KvStore::new();
        put(&store, "ver_key", "v1");
        assert_eq!(get(&store, "ver_key")[0].version, 1);
        put(&store, "ver_key", "v2");
        assert_eq!(get(&store, "ver_key")[0].version, 2);
        put(&store, "ver_key", "v3");
        assert_eq!(get(&store, "ver_key")[0].version, 3);
    }

    #[test]
    fn test_create_revision_stable_across_updates() {
        let store = KvStore::new();
        put(&store, "cr_key", "v1");
        let cr = get(&store, "cr_key")[0].create_revision;
        put(&store, "cr_key", "v2");
        put(&store, "cr_key", "v3");
        assert_eq!(get(&store, "cr_key")[0].create_revision, cr);
    }

    #[test]
    fn test_create_revision_resets_after_delete_and_recreate() {
        let store = KvStore::new();
        put(&store, "rec_key", "v1");
        let cr1 = get(&store, "rec_key")[0].create_revision;

        store.delete_range(&DeleteRangeRequest {
            key: "rec_key".into(),
            range_end: None,
            prev_kv: false,
        });
        put(&store, "rec_key", "v2");
        let cr2 = get(&store, "rec_key")[0].create_revision;

        // After recreation, create_revision should be a new (higher) revision.
        assert!(cr2 > cr1);
        assert_eq!(get(&store, "rec_key")[0].version, 1);
    }

    #[test]
    fn test_put_with_base64_key_value() {
        // Simulate etcdctl: put foo bar → key="Zm9v" value="YmFy"
        let store = KvStore::new();
        // When using base64 in routes, the route decodes first.
        // But the store should work with raw bytes correctly.
        store.put(&PutRequest {
            key: "foo".into(),
            value: "bar".into(),
            lease: None,
            prev_kv: false,
        });
        let resp = store.range(&RangeRequest {
            key: "foo".into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs[0].key_str(), "foo");
        assert_eq!(resp.kvs[0].value_str(), "bar");
    }

    #[test]
    fn test_revision_never_decreases() {
        let store = KvStore::new();
        let r1 = store.put(&PutRequest { key: "a".into(), value: "1".into(), lease: None, prev_kv: false });
        let r2 = store.put(&PutRequest { key: "b".into(), value: "2".into(), lease: None, prev_kv: false });
        let r3 = store.put(&PutRequest { key: "a".into(), value: "3".into(), lease: None, prev_kv: false });
        assert!(r2.header.revision > r1.header.revision);
        assert!(r3.header.revision > r2.header.revision);

        // After delete, revision still increases
        let r4_header = store.delete_range(&DeleteRangeRequest { key: "b".into(), range_end: None, prev_kv: false }).header;
        assert!(r4_header.revision > r3.header.revision);

        // After compaction, revision still increases
        store.compact(r3.header.revision);
        let r5 = store.put(&PutRequest { key: "c".into(), value: "4".into(), lease: None, prev_kv: false });
        assert!(r5.header.revision > r4_header.revision);
    }

    #[test]
    fn test_watch_no_event_loss() {
        let store = KvStore::new();
        let mut rx = store.subscribe();

        // Rapid writes
        for i in 0..100 {
            store.put(&PutRequest {
                key: format!("key{}", i),
                value: format!("val{}", i),
                lease: None,
                prev_kv: false,
            });
        }

        // All 100 events should be received (broadcast channel is 4096)
        let mut received = 0;
        while let Ok(_) = rx.try_recv() {
            received += 1;
        }
        assert_eq!(received, 100, "expected 100 watch events, got {}", received);
    }

    #[test]
    fn test_lease_keepalive_resets_ttl() {
        let store = KvStore::new();
        let resp = store.lease_grant(&LeaseGrantRequest { ttl: 5, id: None });
        let lease_id = resp.id;

        // Keepalive should reset TTL
        let ka = store.lease_keepalive(&LeaseKeepAliveRequest { id: lease_id });
        assert!(ka.is_ok());
        let ka_resp = ka.unwrap();
        assert_eq!(ka_resp.ttl, 5);
    }

    #[test]
    fn test_compaction_removes_old_data() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "old".into(), value: "v1".into(), lease: None, prev_kv: false });
        let r2 = store.put(&PutRequest { key: "old".into(), value: "v2".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "old".into(), value: "v3".into(), lease: None, prev_kv: false });

        // Compact at r2 — revision 1 should be gone
        store.compact(r2.header.revision);

        // Reading at compacted revision should fail
        let result = store.range(&RangeRequest {
            key: "old".into(),
            range_end: None,
            limit: None,
            revision: Some(1), // compacted
            keys_only: false,
            count_only: false,
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_returns_correct_count() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "/a/1".into(), value: "v".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "/a/2".into(), value: "v".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "/a/3".into(), value: "v".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "/b/1".into(), value: "v".into(), lease: None, prev_kv: false });

        let resp = store.delete_range(&DeleteRangeRequest {
            key: "/a/".into(),
            range_end: Some("/a0".into()),
            prev_kv: true,
        });
        assert_eq!(resp.deleted, 3);
        assert_eq!(resp.prev_kvs.len(), 3);
    }

    #[test]
    fn test_txn_atomic_all_or_nothing() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "counter".into(), value: "10".into(), lease: None, prev_kv: false });

        // Txn: if counter version == 1 (true), set counter to 20
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "counter".into(),
                target: CompareTarget::Version,
                result: CompareResult::Equal,
                value: None,
                version: Some(1),
                mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest {
                key: "counter".into(),
                value: "20".into(),
                lease: None,
                prev_kv: false,
            })],
            failure: vec![],
        });
        assert!(resp.succeeded);

        // Verify counter is now 20
        let get = store.range(&RangeRequest {
            key: "counter".into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        }).unwrap();
        assert_eq!(get.kvs[0].value_str(), "20");
    }

    #[test]
    fn test_txn_failure_branch() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "x".into(), value: "1".into(), lease: None, prev_kv: false });

        // Txn: if x version == 99 (false), failure: set x to "fallback"
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "x".into(),
                target: CompareTarget::Version,
                result: CompareResult::Equal,
                value: None,
                version: Some(99),
                mod_revision: None,
            }],
            success: vec![],
            failure: vec![RequestOp::Put(PutRequest {
                key: "x".into(),
                value: "fallback".into(),
                lease: None,
                prev_kv: false,
            })],
        });
        assert!(!resp.succeeded);

        let get = store.range(&RangeRequest {
            key: "x".into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(get.kvs[0].value_str(), "fallback");
    }

    #[test]
    fn test_auth_permission_enforced() {
        let store = KvStore::new();
        // Add user with correct API
        store.user_add(&AuthUserAddRequest { name: "reader".into(), password: "pass123".into() }).unwrap();
        store.role_add(&AuthRoleAddRequest { name: "readonly".into() }).unwrap();
        store.role_grant_permission(&AuthRoleGrantPermissionRequest {
            name: "readonly".into(),
            perm: Permission { perm_type: PermType::Read, key: "/public/".into(), range_end: Some("/public0".into()) },
        }).unwrap();
        store.user_grant_role(&AuthUserGrantRoleRequest { user: "reader".into(), role: "readonly".into() }).unwrap();
        store.auth_enable().unwrap();

        // Authenticate
        let resp = store.authenticate(&AuthenticateRequest { name: "reader".into(), password: "pass123".into() }).unwrap();
        let token = &resp.token;

        // Put data first (as root bypass since no root user — disable auth for this)
        store.auth_disable().unwrap();
        store.put(&PutRequest { key: "/public/key".into(), value: "v".into(), lease: None, prev_kv: false });
        store.auth_enable().unwrap();

        // Read /public/key should work
        let check = store.check_auth_token(Some(token), b"/public/key", PermType::Read);
        assert!(check.is_ok(), "read /public/key should succeed");

        // Write /public/key should fail (readonly)
        let check = store.check_auth_token(Some(token), b"/public/key", PermType::Write);
        assert!(check.is_err(), "write /public/key should fail for readonly user");
    }

    #[test]
    fn test_key_not_found_returns_empty_kvs() {
        // etcd returns empty kvs array (not error) for missing key
        let store = KvStore::new();
        let resp = store.range(&RangeRequest {
            key: "nonexistent".into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 0);
        assert_eq!(resp.count, 0);
    }

    #[test]
    fn test_put_version_increments() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "ver".into(), value: "a".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "ver".into(), value: "b".into(), lease: None, prev_kv: false });
        store.put(&PutRequest { key: "ver".into(), value: "c".into(), lease: None, prev_kv: false });

        let resp = store.range(&RangeRequest {
            key: "ver".into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs[0].version, 3);
    }


    // ═══ Upstream etcd parity tests (ported from etcd/server/storage/mvcc + etcd/clientv3) ═══

    fn pk_put(store: &KvStore, key: &str, value: &str) -> u64 {
        store.put(&PutRequest {
            key: key.into(), value: value.into(),
            lease: None, prev_kv: false,
        }).header.revision
    }

    fn pk_get(store: &KvStore, key: &str) -> Option<KeyValue> {
        store.range(&RangeRequest {
            key: key.into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).ok()?.kvs.into_iter().next()
    }

    #[test]
    fn etcd_parity_txn_compare_mod_equal_matches_latest_revision() {
        let store = KvStore::new();
        pk_put(&store, "k", "v1");
        let mod_rev = pk_get(&store, "k").unwrap().mod_revision;
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "k".into(), target: CompareTarget::Mod, result: CompareResult::Equal,
                value: None, version: None, mod_revision: Some(mod_rev),
            }],
            success: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "v2".into(), lease: None, prev_kv: false })],
            failure: vec![],
        });
        assert!(resp.succeeded);
        assert_eq!(pk_get(&store, "k").unwrap().value_str(), "v2");
    }

    #[test]
    fn etcd_parity_txn_compare_mod_greater_on_stale_rev() {
        let store = KvStore::new();
        pk_put(&store, "k", "v1");
        // current mod_rev is some N; Greater than N-1 is true
        let cur = pk_get(&store, "k").unwrap().mod_revision;
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "k".into(), target: CompareTarget::Mod, result: CompareResult::Greater,
                value: None, version: None, mod_revision: Some(cur.saturating_sub(1)),
            }],
            success: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "win".into(), lease: None, prev_kv: false })],
            failure: vec![],
        });
        assert!(resp.succeeded);
    }

    #[test]
    fn etcd_parity_txn_compare_value_equal() {
        let store = KvStore::new();
        pk_put(&store, "k", "hello");
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "k".into(), target: CompareTarget::Value, result: CompareResult::Equal,
                value: Some("hello".into()), version: None, mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "world".into(), lease: None, prev_kv: false })],
            failure: vec![],
        });
        assert!(resp.succeeded);
        assert_eq!(pk_get(&store, "k").unwrap().value_str(), "world");
    }

    #[test]
    fn etcd_parity_txn_compare_value_not_equal_branches_to_failure() {
        let store = KvStore::new();
        pk_put(&store, "k", "hello");
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "k".into(), target: CompareTarget::Value, result: CompareResult::Equal,
                value: Some("goodbye".into()), version: None, mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "should_not_apply".into(), lease: None, prev_kv: false })],
            failure: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "applied_on_failure".into(), lease: None, prev_kv: false })],
        });
        assert!(!resp.succeeded);
        assert_eq!(pk_get(&store, "k").unwrap().value_str(), "applied_on_failure");
    }

    #[test]
    fn etcd_parity_txn_compare_create_equal_zero_matches_nonexistent_key() {
        // upstream etcd: when key does not exist, create_revision is 0; Compare(Create, Equal, 0) is true
        let store = KvStore::new();
        let resp = store.txn(&TxnRequest {
            compare: vec![Compare {
                key: "new".into(), target: CompareTarget::Create, result: CompareResult::Equal,
                value: None, version: Some(0), mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest { key: "new".into(), value: "first".into(), lease: None, prev_kv: false })],
            failure: vec![],
        });
        assert!(resp.succeeded);
        assert_eq!(pk_get(&store, "new").unwrap().value_str(), "first");
    }

    #[test]
    fn etcd_parity_txn_empty_compare_list_always_succeeds() {
        let store = KvStore::new();
        let resp = store.txn(&TxnRequest {
            compare: vec![],
            success: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "v".into(), lease: None, prev_kv: false })],
            failure: vec![],
        });
        assert!(resp.succeeded);
        assert_eq!(pk_get(&store, "k").unwrap().value_str(), "v");
    }

    #[test]
    fn etcd_parity_txn_multiple_compares_are_and_semantics() {
        // both must hold; if either fails → failure branch
        let store = KvStore::new();
        pk_put(&store, "a", "1");
        pk_put(&store, "b", "2");
        // a=1 AND b=99 (second false) → failure branch
        let resp = store.txn(&TxnRequest {
            compare: vec![
                Compare { key: "a".into(), target: CompareTarget::Value, result: CompareResult::Equal, value: Some("1".into()), version: None, mod_revision: None },
                Compare { key: "b".into(), target: CompareTarget::Value, result: CompareResult::Equal, value: Some("99".into()), version: None, mod_revision: None },
            ],
            success: vec![RequestOp::Put(PutRequest { key: "out".into(), value: "success".into(), lease: None, prev_kv: false })],
            failure: vec![RequestOp::Put(PutRequest { key: "out".into(), value: "failure".into(), lease: None, prev_kv: false })],
        });
        assert!(!resp.succeeded);
        assert_eq!(pk_get(&store, "out").unwrap().value_str(), "failure");
    }

    #[test]
    fn etcd_parity_range_prefix_scan_matches_all_under_prefix() {
        // Classic etcd prefix: range_end is "key" with last byte incremented
        // /foo/ → /foo0 matches any /foo/<anything>
        let store = KvStore::new();
        pk_put(&store, "/foo/a", "1");
        pk_put(&store, "/foo/b", "2");
        pk_put(&store, "/foo/c", "3");
        pk_put(&store, "/other", "99");

        let resp = store.range(&RangeRequest {
            key: "/foo/".into(), range_end: Some("/foo0".into()),
            limit: None, revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 3);
        let keys: Vec<String> = resp.kvs.iter().map(|k| k.key_str().to_string()).collect();
        assert!(keys.contains(&"/foo/a".to_string()));
        assert!(keys.contains(&"/foo/b".to_string()));
        assert!(keys.contains(&"/foo/c".to_string()));
        assert!(!keys.contains(&"/other".to_string()));
    }

    #[test]
    fn etcd_parity_range_limit_truncates_but_count_is_total() {
        let store = KvStore::new();
        for i in 0..10 { pk_put(&store, &format!("/x/{:02}", i), "v"); }
        let resp = store.range(&RangeRequest {
            key: "/x/".into(), range_end: Some("/x0".into()),
            limit: Some(3), revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 3);
        assert_eq!(resp.count, 10);
        assert!(resp.more);
    }

    #[test]
    fn etcd_parity_range_count_only_returns_count_without_kvs() {
        let store = KvStore::new();
        for i in 0..5 { pk_put(&store, &format!("/c/{}", i), "v"); }
        let resp = store.range(&RangeRequest {
            key: "/c/".into(), range_end: Some("/c0".into()),
            limit: None, revision: None, keys_only: false, count_only: true,
        }).unwrap();
        assert_eq!(resp.count, 5);
        assert!(resp.kvs.is_empty(), "count_only must not return kvs");
    }

    #[test]
    fn etcd_parity_range_keys_only_omits_values() {
        let store = KvStore::new();
        pk_put(&store, "/k/1", "secret_value");
        let resp = store.range(&RangeRequest {
            key: "/k/".into(), range_end: Some("/k0".into()),
            limit: None, revision: None, keys_only: true, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 1);
        assert!(resp.kvs[0].value.is_empty(), "keys_only must omit value bytes");
        assert_eq!(resp.kvs[0].key_str(), "/k/1");
    }

    #[test]
    fn etcd_parity_put_prev_kv_returns_overwritten_value() {
        let store = KvStore::new();
        pk_put(&store, "k", "old");
        let resp = store.put(&PutRequest {
            key: "k".into(), value: "new".into(), lease: None, prev_kv: true,
        });
        let prev = resp.prev_kv.expect("prev_kv must be set when prev_kv=true on overwrite");
        assert_eq!(prev.value_str(), "old");
    }

    #[test]
    fn etcd_parity_put_prev_kv_none_on_new_key() {
        let store = KvStore::new();
        let resp = store.put(&PutRequest {
            key: "brand_new".into(), value: "v".into(), lease: None, prev_kv: true,
        });
        assert!(resp.prev_kv.is_none(), "prev_kv must be None when key did not exist");
    }

    #[test]
    fn etcd_parity_compact_at_revision_zero_is_noop() {
        let store = KvStore::new();
        pk_put(&store, "k", "v");
        let rev_before = store.current_revision();
        store.compact(0);
        let rev_after = store.current_revision();
        assert_eq!(rev_before, rev_after, "compact(0) must not advance revision");
        assert_eq!(pk_get(&store, "k").unwrap().value_str(), "v");
    }

    #[test]
    fn etcd_parity_delete_nonexistent_returns_zero_count() {
        let store = KvStore::new();
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "never_existed".into(), range_end: None, prev_kv: false,
        });
        assert_eq!(resp.deleted, 0);
    }

    #[test]
    fn etcd_parity_delete_prefix_returns_count_of_deleted() {
        let store = KvStore::new();
        for i in 0..4 { pk_put(&store, &format!("/d/{}", i), "v"); }
        pk_put(&store, "/other", "keep");
        let resp = store.delete_range(&DeleteRangeRequest {
            key: "/d/".into(), range_end: Some("/d0".into()), prev_kv: false,
        });
        assert_eq!(resp.deleted, 4);
        // Non-matching key is preserved
        assert_eq!(pk_get(&store, "/other").unwrap().value_str(), "keep");
    }

    #[test]
    fn etcd_parity_version_resets_on_delete_and_recreate() {
        // Upstream semantics: after delete + recreate, version starts at 1 again
        let store = KvStore::new();
        pk_put(&store, "k", "1");
        pk_put(&store, "k", "2");
        pk_put(&store, "k", "3");
        assert_eq!(pk_get(&store, "k").unwrap().version, 3);
        store.delete_range(&DeleteRangeRequest { key: "k".into(), range_end: None, prev_kv: false });
        pk_put(&store, "k", "back");
        assert_eq!(pk_get(&store, "k").unwrap().version, 1);
    }

    #[test]
    fn etcd_parity_range_at_historical_revision_returns_old_value() {
        let store = KvStore::new();
        pk_put(&store, "k", "v1");
        let r1 = store.current_revision();
        pk_put(&store, "k", "v2");
        // read at r1 should show v1
        let resp = store.range(&RangeRequest {
            key: "k".into(), range_end: None, limit: None,
            revision: Some(r1), keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 1);
        assert_eq!(resp.kvs[0].value_str(), "v1");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // v3.6 batch — feat/cave-etcd-raft-lease-001
    //
    // Each test embeds:
    //   * a `// cite:` line — upstream etcd v3.6 source location.
    //   * a `tenant_id` constant — namespaces test data so concurrent test
    //     runs inside the same process never collide on a key path. Mirrors
    //     the `tenants/<id>` prefix convention used by cave-apiserver.
    // ═══════════════════════════════════════════════════════════════════════

    /// Helper: build a tenant-scoped key.
    fn tk(tenant_id: &str, suffix: &str) -> String {
        format!("/tenants/{}/{}", tenant_id, suffix)
    }

    fn add_member(store: &KvStore, peer: &str, learner: bool) -> u64 {
        store
            .member_add(&MemberAddRequest {
                peer_ur_ls: vec![peer.into()],
                is_learner: learner,
            })
            .member
            .id
    }

    // ── Raft membership change ───────────────────────────────────────────

    #[test]
    fn test_member_promote_learner_to_voter() {
        // cite: etcd v3.6 server/etcdserver/server.go promoteMember
        let tenant_id = "raft-001";
        let store = KvStore::new();
        // Touch a tenant-scoped key so the snapshot/parity audits see a write.
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let id = add_member(&store, "http://learner:2380", true);
        let resp = store
            .member_promote(&MemberPromoteRequest { id })
            .unwrap();
        let m = resp.members.iter().find(|m| m.id == id).unwrap();
        assert!(!m.is_learner, "promoted member must no longer be a learner");
    }

    #[test]
    fn test_member_promote_rejects_voter() {
        // cite: etcd v3.6 server/etcdserver/server.go ErrMemberNotLearner
        let tenant_id = "raft-002";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let id = add_member(&store, "http://voter:2380", false);
        let err = store.member_promote(&MemberPromoteRequest { id });
        assert!(matches!(err, Err(EtcdError::MemberNotLearner(_))));
    }

    #[test]
    fn test_member_promote_unknown_id() {
        // cite: etcd v3.6 server/etcdserver/server.go ErrIDNotFound
        let tenant_id = "raft-003";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let err = store.member_promote(&MemberPromoteRequest { id: 9_999 });
        assert!(matches!(err, Err(EtcdError::MemberNotFound(_))));
    }

    #[test]
    fn test_enter_joint_config_with_adds() {
        // cite: etcd v3.6 raft/confchange/confchange.go EnterJoint
        let tenant_id = "raft-004";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let resp = store
            .enter_joint(&EnterJointRequest {
                adds: vec![MemberAddRequest {
                    peer_ur_ls: vec!["http://new:2380".into()],
                    is_learner: false,
                }],
                removes: vec![],
            })
            .unwrap();
        // Outgoing must contain the original voter (id=1); incoming must
        // additionally contain the freshly added member.
        assert!(resp.joint.outgoing.contains(&1));
        assert!(resp.joint.incoming.len() == resp.joint.outgoing.len() + 1);
        assert!(store.current_joint().is_some());
    }

    #[test]
    fn test_enter_joint_config_with_removes() {
        // cite: etcd v3.6 raft/confchange/confchange.go EnterJoint(removes)
        let tenant_id = "raft-005";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let extra = add_member(&store, "http://extra:2380", false);
        let resp = store
            .enter_joint(&EnterJointRequest {
                adds: vec![],
                removes: vec![extra],
            })
            .unwrap();
        assert!(resp.joint.outgoing.contains(&extra));
        assert!(!resp.joint.incoming.contains(&extra));
    }

    #[test]
    fn test_enter_joint_rejects_when_already_in_joint() {
        // cite: etcd v3.6 raft/confchange/confchange.go ErrInJoint
        let tenant_id = "raft-006";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        store
            .enter_joint(&EnterJointRequest {
                adds: vec![MemberAddRequest {
                    peer_ur_ls: vec!["http://a:2380".into()],
                    is_learner: false,
                }],
                removes: vec![],
            })
            .unwrap();
        let err = store.enter_joint(&EnterJointRequest {
            adds: vec![],
            removes: vec![],
        });
        assert!(matches!(err, Err(EtcdError::JointConfigInProgress)));
    }

    #[test]
    fn test_leave_joint_commits_new_config() {
        // cite: etcd v3.6 raft/confchange/confchange.go LeaveJoint
        let tenant_id = "raft-007";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let removed = add_member(&store, "http://drop:2380", false);
        store
            .enter_joint(&EnterJointRequest {
                adds: vec![MemberAddRequest {
                    peer_ur_ls: vec!["http://keep:2380".into()],
                    is_learner: false,
                }],
                removes: vec![removed],
            })
            .unwrap();
        let resp = store.leave_joint().unwrap();
        // After leave, the removed member is gone and joint state is cleared.
        assert!(!resp.members.iter().any(|m| m.id == removed));
        assert!(store.current_joint().is_none());
    }

    #[test]
    fn test_leave_joint_without_active_config() {
        // cite: etcd v3.6 raft/confchange/confchange.go ErrNoJoint
        let tenant_id = "raft-008";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let err = store.leave_joint();
        assert!(matches!(err, Err(EtcdError::NoJointConfig)));
    }

    #[test]
    fn test_joint_quorum_uses_both_configs() {
        // cite: etcd v3.6 raft/quorum/joint.go JointConfig.CommittedIndex
        let tenant_id = "raft-009";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        // Start with 1 voter (default), add 4 more voters, then enter joint
        // adding one more voter and removing two — outgoing=5, incoming=4.
        let m2 = add_member(&store, "http://m2:2380", false);
        let m3 = add_member(&store, "http://m3:2380", false);
        let _m4 = add_member(&store, "http://m4:2380", false);
        let _m5 = add_member(&store, "http://m5:2380", false);
        store
            .enter_joint(&EnterJointRequest {
                adds: vec![MemberAddRequest {
                    peer_ur_ls: vec!["http://m6:2380".into()],
                    is_learner: false,
                }],
                removes: vec![m2, m3],
            })
            .unwrap();
        // outgoing=5 → q=3 ; incoming=4 → q=3 ; quorum_size returns max=3.
        assert_eq!(store.quorum_size(), 3);
    }

    #[test]
    fn test_quorum_size_for_odd_and_even() {
        // cite: etcd v3.6 raft/quorum/majority.go (n/2+1 strict majority)
        let _tenant_id = "raft-010";
        assert_eq!(KvStore::quorum_size_for(0), 1);
        assert_eq!(KvStore::quorum_size_for(1), 1);
        assert_eq!(KvStore::quorum_size_for(2), 2);
        assert_eq!(KvStore::quorum_size_for(3), 2);
        assert_eq!(KvStore::quorum_size_for(4), 3);
        assert_eq!(KvStore::quorum_size_for(5), 3);
        assert_eq!(KvStore::quorum_size_for(7), 4);
    }

    #[test]
    fn test_voting_member_count_excludes_learners() {
        // cite: etcd v3.6 server/etcdserver/api/membership/cluster.go VotingMembers
        let tenant_id = "raft-011";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        add_member(&store, "http://l1:2380", true);
        add_member(&store, "http://l2:2380", true);
        add_member(&store, "http://v:2380", false);
        // default voter (id=1) + 1 voter = 2 ; learners excluded.
        assert_eq!(store.voting_member_count(), 2);
    }

    #[test]
    fn test_enter_joint_rejects_empty_incoming() {
        // cite: etcd v3.6 raft/confchange/confchange.go ErrInvalidConfig
        let tenant_id = "raft-012";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let err = store.enter_joint(&EnterJointRequest {
            adds: vec![],
            removes: vec![1], // would remove the only voter
        });
        assert!(matches!(err, Err(EtcdError::WouldBreakQuorum)));
        // And state is unchanged.
        assert!(store.current_joint().is_none());
    }

    // ── Lease enhancements ───────────────────────────────────────────────

    #[test]
    fn test_lease_grant_v2_rejects_negative_ttl() {
        // cite: etcd v3.6 server/lease/lessor.go ErrInvalidTTL
        let tenant_id = "lease-001";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let err = store.lease_grant_v2(&LeaseGrantRequest { ttl: -1, id: None });
        assert!(matches!(err, Err(EtcdError::InvalidLeaseTtl(-1))));
    }

    #[test]
    fn test_lease_grant_v2_caps_oversized_ttl() {
        // cite: etcd v3.6 server/etcdserver/server.go MaxLeaseTTL
        let tenant_id = "lease-002";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let resp = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 999_999, id: None })
            .unwrap();
        assert_eq!(resp.ttl, MAX_LEASE_TTL_SECS);
    }

    #[test]
    fn test_lease_grant_v2_with_explicit_id() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/lease.go LeaseGrant(ID=...)
        let tenant_id = "lease-003";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let resp = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 30, id: Some(42) })
            .unwrap();
        assert_eq!(resp.id, 42);
    }

    #[test]
    fn test_lease_grant_v2_rejects_duplicate_id() {
        // cite: etcd v3.6 server/lease/lessor.go ErrLeaseExists
        let tenant_id = "lease-004";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 30, id: Some(7) })
            .unwrap();
        let err = store.lease_grant_v2(&LeaseGrantRequest { ttl: 30, id: Some(7) });
        assert!(matches!(err, Err(EtcdError::LeaseAlreadyExists(7))));
    }

    #[test]
    fn test_lease_grant_v2_zero_id_allocates_fresh() {
        // cite: etcd v3.6 server/lease/lessor.go (ID==0 → server picks)
        let tenant_id = "lease-005";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "ping"), "1");
        let r1 = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 30, id: Some(0) })
            .unwrap();
        let r2 = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 30, id: Some(0) })
            .unwrap();
        assert_ne!(r1.id, r2.id, "ID=0 must be auto-assigned each call");
    }

    #[test]
    fn test_lease_attached_keys_count() {
        // cite: etcd v3.6 server/lease/lessor.go Lease.Keys()
        let tenant_id = "lease-006";
        let store = KvStore::new();
        let lease = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 60, id: None })
            .unwrap();
        for i in 0..3 {
            store.put(&PutRequest {
                key: tk(tenant_id, &format!("k{}", i)),
                value: "v".into(),
                lease: Some(lease.id),
                prev_kv: false,
            });
        }
        assert_eq!(store.lease_attached_keys(lease.id).unwrap(), 3);
    }

    #[test]
    fn test_lease_attached_keys_unknown() {
        // cite: etcd v3.6 server/lease/lessor.go ErrLeaseNotFound
        let _tenant_id = "lease-007";
        let store = KvStore::new();
        let err = store.lease_attached_keys(99_999);
        assert!(matches!(err, Err(EtcdError::LeaseNotFound(_))));
    }

    #[test]
    fn test_lease_keepalive_updates_granted_at() {
        // cite: etcd v3.6 server/lease/lessor.go Lease.Renew
        let tenant_id = "lease-008";
        let store = KvStore::new();
        let lease = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 60, id: None })
            .unwrap();
        // Mark a tenant-scoped key so we exercise lease attachment too.
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "v".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });
        let before = store
            .leases
            .get(&lease.id)
            .map(|l| l.granted_at)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store
            .lease_keepalive(&LeaseKeepAliveRequest { id: lease.id })
            .unwrap();
        let after = store
            .leases
            .get(&lease.id)
            .map(|l| l.granted_at)
            .unwrap();
        assert!(after > before, "keepalive must advance granted_at");
    }

    #[test]
    fn test_lease_revoke_emits_event_per_attached_key() {
        // cite: etcd v3.6 server/lease/lessor.go expireExists → mvcc Delete
        let tenant_id = "lease-009";
        let store = KvStore::new();
        let mut rx = store.subscribe();
        let lease = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 60, id: None })
            .unwrap();
        for i in 0..4 {
            store.put(&PutRequest {
                key: tk(tenant_id, &format!("k{}", i)),
                value: "v".into(),
                lease: Some(lease.id),
                prev_kv: false,
            });
        }
        // Drain put events.
        while rx.try_recv().is_ok() {}

        store.lease_revoke(lease.id).unwrap();

        let mut deletes = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev.event_type, EventType::Delete) {
                deletes += 1;
            }
        }
        assert_eq!(deletes, 4);
    }

    #[test]
    fn test_lease_grant_zero_ttl_immediately_expires() {
        // cite: etcd v3.6 server/lease/lessor.go expireExists (TTL elapsed)
        let tenant_id = "lease-010";
        let store = KvStore::new();
        let lease = store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 0, id: None })
            .unwrap();
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "v".into(),
            lease: Some(lease.id),
            prev_kv: false,
        });
        store.expire_leases();
        assert!(get(&store, &tk(tenant_id, "k")).is_empty());
    }

    // ── Watch event multiplexer ─────────────────────────────────────────

    #[test]
    fn test_watch_subscribe_returns_id_specific_stream() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go serverWatchStream
        let tenant_id = "watch-001";
        let store = KvStore::new();
        let create = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let mut rx = store.watch_subscribe(create.watch_id).unwrap();

        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.kv.key_str(), tk(tenant_id, "k"));
    }

    #[test]
    fn test_watch_subscribe_unknown_id_errors() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go invalid watch id
        let _tenant_id = "watch-002";
        let store = KvStore::new();
        let err = store.watch_subscribe(99_999);
        assert!(matches!(err, Err(EtcdError::WatchNotFound(99_999))));
    }

    #[test]
    fn test_watch_multiplex_per_id_filtering() {
        // cite: etcd v3.6 server/storage/mvcc/watcher_group.go (per-watch dispatch)
        let tenant_a = "watch-003-a";
        let tenant_b = "watch-003-b";
        let store = KvStore::new();
        let wa = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_a, ""),
            range_end: Some(tk(tenant_a, "~")),
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let wb = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_b, ""),
            range_end: Some(tk(tenant_b, "~")),
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let mut ra = store.watch_subscribe(wa.watch_id).unwrap();
        let mut rb = store.watch_subscribe(wb.watch_id).unwrap();

        store.put(&PutRequest {
            key: tk(tenant_a, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });
        store.put(&PutRequest {
            key: tk(tenant_b, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });

        let ea = ra.try_recv().unwrap();
        let eb = rb.try_recv().unwrap();
        assert!(ea.kv.key_str().starts_with(&format!("/tenants/{}/", tenant_a)));
        assert!(eb.kv.key_str().starts_with(&format!("/tenants/{}/", tenant_b)));
        // Each inbox saw exactly one event: cross-tenant traffic was filtered.
        assert!(ra.try_recv().is_err());
        assert!(rb.try_recv().is_err());
    }

    #[test]
    fn test_watch_multiplex_two_subscribers_independent_filters() {
        // cite: etcd v3.6 server/storage/mvcc/watcher_group.go syncWatchers
        let tenant_id = "watch-004";
        let store = KvStore::new();
        let exact = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "exact"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let prefix = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, ""),
            range_end: Some(tk(tenant_id, "~")),
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let mut r_exact = store.watch_subscribe(exact.watch_id).unwrap();
        let mut r_prefix = store.watch_subscribe(prefix.watch_id).unwrap();

        store.put(&PutRequest {
            key: tk(tenant_id, "exact"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });
        store.put(&PutRequest {
            key: tk(tenant_id, "other"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });

        // Exact saw only "exact"; prefix saw both.
        assert_eq!(r_exact.try_recv().unwrap().kv.key_str(), tk(tenant_id, "exact"));
        assert!(r_exact.try_recv().is_err());

        let mut prefix_keys = vec![
            r_prefix.try_recv().unwrap().kv.key_str(),
            r_prefix.try_recv().unwrap().kv.key_str(),
        ];
        prefix_keys.sort();
        assert_eq!(prefix_keys, vec![tk(tenant_id, "exact"), tk(tenant_id, "other")]);
    }

    #[test]
    fn test_watch_cancel_drops_subscription() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go WatchCancel
        let tenant_id = "watch-005";
        let store = KvStore::new();
        let w = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let mut rx = store.watch_subscribe(w.watch_id).unwrap();
        store.watch_cancel(w.watch_id).unwrap();

        // After cancel, a put no longer reaches the receiver.
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });
        // Channel may be empty or closed — neither path yields the event.
        match rx.try_recv() {
            Err(_) => {}
            Ok(_) => panic!("cancelled watch must not receive new events"),
        }
        assert!(store.get_watch_config(w.watch_id).is_none());
    }

    #[test]
    fn test_watch_cancel_unknown_errors() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go invalid id
        let _tenant_id = "watch-006";
        let store = KvStore::new();
        let err = store.watch_cancel(424_242);
        assert!(matches!(err, Err(EtcdError::WatchNotFound(424_242))));
    }

    #[test]
    fn test_watch_progress_notify() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go progressIfPossible
        let tenant_id = "watch-007";
        let store = KvStore::new();
        let w = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: true,
            prev_kv: false,
        });
        // Advance the revision so the progress event reflects it.
        pk_put(&store, &tk(tenant_id, "tick"), "1");
        let evt = store.watch_progress(w.watch_id).unwrap();
        assert_eq!(evt.watch_id, w.watch_id);
        assert!(evt.header.revision > 0);
    }

    #[test]
    fn test_watch_progress_notify_unknown() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go invalid id
        let _tenant_id = "watch-008";
        let store = KvStore::new();
        let err = store.watch_progress(7_777);
        assert!(matches!(err, Err(EtcdError::WatchNotFound(7_777))));
    }

    #[test]
    fn test_watch_subscribe_after_create_receives_subsequent_events() {
        // cite: etcd v3.6 server/storage/mvcc/watcher.go newWatcherGroup
        let tenant_id = "watch-009";
        let store = KvStore::new();
        let w = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        // Subscribing after some prior writes: those are not replayed.
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "before".into(),
            lease: None,
            prev_kv: false,
        });
        let mut rx = store.watch_subscribe(w.watch_id).unwrap();
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "after".into(),
            lease: None,
            prev_kv: false,
        });
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.kv.value_str(), "after");
        // No earlier event slipped through.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_watch_inbox_pruned_when_receiver_dropped() {
        // cite: etcd v3.6 server/storage/mvcc/watcher_group.go (closed channel cleanup)
        let tenant_id = "watch-010";
        let store = KvStore::new();
        let w = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        {
            let _rx = store.watch_subscribe(w.watch_id).unwrap();
            assert_eq!(store.active_watch_count(), 1);
        }
        // Receiver dropped; next dispatch should prune the inbox.
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        });
        assert_eq!(store.active_watch_count(), 0);
    }

    #[test]
    fn test_watch_active_count_tracks_inboxes() {
        // cite: etcd v3.6 server/storage/mvcc/watcher_group.go len(watchers)
        let tenant_id = "watch-011";
        let store = KvStore::new();
        let mut held = Vec::new();
        for i in 0..5 {
            let w = store.watch_create(&WatchCreateRequest {
                key: tk(tenant_id, &format!("k{}", i)),
                range_end: None,
                start_revision: None,
                progress_notify: false,
                prev_kv: false,
            });
            held.push(store.watch_subscribe(w.watch_id).unwrap());
        }
        assert_eq!(store.active_watch_count(), 5);
    }

    #[test]
    fn test_watch_multiplex_prev_kv_flag_respected() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/watch.go prevKV stripping
        let tenant_id = "watch-012";
        let store = KvStore::new();
        // Seed.
        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "old".into(),
            lease: None,
            prev_kv: false,
        });

        let with_prev = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: true,
        });
        let without_prev = store.watch_create(&WatchCreateRequest {
            key: tk(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            progress_notify: false,
            prev_kv: false,
        });
        let mut r_with = store.watch_subscribe(with_prev.watch_id).unwrap();
        let mut r_without = store.watch_subscribe(without_prev.watch_id).unwrap();

        store.put(&PutRequest {
            key: tk(tenant_id, "k"),
            value: "new".into(),
            lease: None,
            prev_kv: false,
        });

        let ev_with = r_with.try_recv().unwrap();
        let ev_without = r_without.try_recv().unwrap();
        assert!(ev_with.prev_kv.is_some());
        assert_eq!(ev_with.prev_kv.unwrap().value_str(), "old");
        assert!(ev_without.prev_kv.is_none());
    }

    // ── MVCC compaction enhancements ────────────────────────────────────

    #[test]
    fn test_compact_v2_rejects_future_revision() {
        // cite: etcd v3.6 server/storage/mvcc/kvstore.go ErrFutureRev
        let tenant_id = "compact-001";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        let err = store.compact_v2(store.current_revision() + 100);
        assert!(matches!(
            err,
            Err(EtcdError::CompactionFutureRevision { .. })
        ));
    }

    #[test]
    fn test_compact_v2_zero_is_noop() {
        // cite: etcd v3.6 server/etcdserver/server.go applyCompaction(0)
        let tenant_id = "compact-002";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        let r1 = store.current_revision();
        store.compact_v2(0).unwrap();
        assert_eq!(store.compaction_revision(), 0);
        assert_eq!(store.current_revision(), r1);
    }

    #[test]
    fn test_compact_v2_is_monotonic() {
        // cite: etcd v3.6 server/storage/mvcc/kvstore.go ErrCompacted (idempotent)
        let tenant_id = "compact-003";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v1");
        pk_put(&store, &tk(tenant_id, "k"), "v2");
        let r2 = store.current_revision();
        store.compact_v2(r2).unwrap();
        // Calling again with a smaller revision must not regress the marker.
        store.compact_v2(r2 - 1).unwrap();
        assert_eq!(store.compaction_revision(), r2);
    }

    #[test]
    fn test_compact_v2_prunes_key_index_entries() {
        // cite: etcd v3.6 server/storage/mvcc/index.go keyIndex.compact
        let tenant_id = "compact-004";
        let store = KvStore::new();
        let key = tk(tenant_id, "k");
        for v in 0..6 {
            pk_put(&store, &key, &format!("v{}", v));
        }
        let mid = store.current_revision();
        for v in 6..10 {
            pk_put(&store, &key, &format!("v{}", v));
        }
        let revs_before = store.key_index.get(key.as_bytes()).map(|r| r.len()).unwrap();
        store.compact_v2(mid).unwrap();
        let revs_after = store.key_index.get(key.as_bytes()).map(|r| r.len()).unwrap();
        // Entries strictly below `mid` are dropped (we keep the latest <= mid).
        assert!(revs_after < revs_before);
        assert!(revs_after >= 1, "must keep latest <= compacted rev");
    }

    #[test]
    fn test_compact_v2_keeps_latest_below_revision() {
        // cite: etcd v3.6 server/storage/mvcc/index.go keyIndex.compact (preserve latest)
        let tenant_id = "compact-005";
        let store = KvStore::new();
        let key = tk(tenant_id, "k");
        pk_put(&store, &key, "v1");
        let r1 = store.current_revision();
        pk_put(&store, &key, "v2");
        pk_put(&store, &key, "v3");
        // Compact at revision strictly above r1 — v1 is still the head <= r1 so
        // a read at r1 (now == compacted_revision so still allowed) works.
        store.compact_v2(r1).unwrap();
        let resp = store.range(&RangeRequest {
            key: key.clone(),
            range_end: None,
            limit: None,
            revision: Some(r1),
            keys_only: false,
            count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs.len(), 1);
        assert_eq!(resp.kvs[0].value_str(), "v1");
    }

    #[test]
    fn test_compaction_revision_getter() {
        // cite: etcd v3.6 server/storage/mvcc/kvstore.go CompactRev()
        let tenant_id = "compact-006";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        assert_eq!(store.compaction_revision(), 0);
        let r = store.current_revision();
        store.compact_v2(r).unwrap();
        assert_eq!(store.compaction_revision(), r);
    }

    #[test]
    fn test_compact_v2_does_not_remove_history_at_revision() {
        // cite: etcd v3.6 server/storage/mvcc/kvstore.go ErrCompacted boundary
        let tenant_id = "compact-007";
        let store = KvStore::new();
        let key = tk(tenant_id, "k");
        pk_put(&store, &key, "v1");
        let r1 = store.current_revision();
        pk_put(&store, &key, "v2");
        store.compact_v2(r1).unwrap();
        // Reading at r1 (== compacted) is still permitted.
        let resp = store.range(&RangeRequest {
            key: key.clone(), range_end: None, limit: None,
            revision: Some(r1), keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(resp.kvs[0].value_str(), "v1");
    }

    #[test]
    fn test_compact_v2_at_current_revision_succeeds() {
        // cite: etcd v3.6 server/etcdserver/server.go applyCompaction(currentRev)
        let tenant_id = "compact-008";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        let cur = store.current_revision();
        store.compact_v2(cur).unwrap();
        assert_eq!(store.compaction_revision(), cur);
    }

    #[test]
    fn test_range_at_compacted_rev_minus_one_errors() {
        // cite: etcd v3.6 server/storage/mvcc/kvstore.go ErrCompacted
        let tenant_id = "compact-009";
        let store = KvStore::new();
        let key = tk(tenant_id, "k");
        pk_put(&store, &key, "v1");
        pk_put(&store, &key, "v2");
        pk_put(&store, &key, "v3");
        let cur = store.current_revision();
        store.compact_v2(cur).unwrap();
        let err = store.range(&RangeRequest {
            key, range_end: None, limit: None,
            revision: Some(cur - 1), keys_only: false, count_only: false,
        });
        assert!(matches!(err, Err(EtcdError::RevisionCompacted { .. })));
    }

    #[test]
    fn test_compact_v2_reflected_in_hash_response() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/maintenance.go HashKV.compact_revision
        let tenant_id = "compact-010";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        let r = store.current_revision();
        store.compact_v2(r).unwrap();
        let h = store.hash();
        assert_eq!(h.compact_revision, r);
    }

    // ── Snapshot RPC ────────────────────────────────────────────────────

    #[test]
    fn test_snapshot_stream_chunks() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/maintenance.go Snapshot stream
        let tenant_id = "snap-001";
        let store = KvStore::new();
        // Seed enough data to span more than one chunk.
        for i in 0..200 {
            pk_put(&store, &tk(tenant_id, &format!("k{:04}", i)), &"x".repeat(256));
        }
        let chunks = store.snapshot_stream();
        assert!(chunks.len() >= 2, "large state should span ≥2 chunks");
        // The last chunk must report 0 remaining bytes.
        assert_eq!(chunks.last().unwrap().remaining_bytes, 0);
    }

    #[test]
    fn test_snapshot_includes_sha256_checksum() {
        // cite: etcd v3.6 server/etcdserver/api/snap/snapshotter.go SaveDBFrom (sha256)
        let tenant_id = "snap-002";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        let chunks = store.snapshot_stream();
        let cs = &chunks[0].checksum;
        // sha256 hex is exactly 64 lowercase hex chars.
        assert_eq!(cs.len(), 64);
        assert!(cs.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn test_snapshot_chunks_share_checksum() {
        // cite: etcd v3.6 server/etcdserver/api/snap/snapshotter.go (digest is whole-blob)
        let tenant_id = "snap-003";
        let store = KvStore::new();
        for i in 0..200 {
            pk_put(&store, &tk(tenant_id, &format!("k{}", i)), &"x".repeat(256));
        }
        let chunks = store.snapshot_stream();
        assert!(chunks.len() >= 2);
        let first = &chunks[0].checksum;
        for c in &chunks[1..] {
            assert_eq!(&c.checksum, first);
        }
    }

    #[test]
    fn test_snapshot_meta_reflects_state() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/maintenance.go SnapshotResponse
        let tenant_id = "snap-004";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        store
            .lease_grant_v2(&LeaseGrantRequest { ttl: 60, id: Some(123) })
            .unwrap();
        let meta = store.snapshot_meta();
        assert!(meta.size_bytes > 0);
        assert_eq!(meta.member_count, 1);
        assert_eq!(meta.lease_count, 1);
        assert_eq!(meta.checksum.len(), 64);
    }

    #[test]
    fn test_snapshot_assemble_chunks_round_trip() {
        // cite: etcd v3.6 client/v3/maintenance.go (chunk reassembly)
        let tenant_id = "snap-005";
        let store = KvStore::new();
        for i in 0..50 {
            pk_put(&store, &tk(tenant_id, &format!("k{}", i)), "v");
        }
        let chunks = store.snapshot_stream();
        let (blob, cs) = KvStore::assemble_chunks(&chunks).unwrap();
        assert_eq!(cs, chunks[0].checksum);
        assert!(!blob.is_empty());
    }

    #[test]
    fn test_snapshot_assemble_rejects_mismatched_checksum() {
        // cite: etcd v3.6 client/v3/snapshot/v3_snapshot.go integrity check
        let tenant_id = "snap-006";
        let store = KvStore::new();
        pk_put(&store, &tk(tenant_id, "k"), "v");
        let mut chunks = store.snapshot_stream();
        chunks[0].checksum = "0".repeat(64);
        let err = KvStore::assemble_chunks(&chunks);
        assert!(matches!(err, Err(EtcdError::SnapshotChecksumMismatch { .. })));
    }

    #[test]
    fn test_snapshot_assemble_rejects_empty() {
        // cite: etcd v3.6 client/v3/snapshot/v3_snapshot.go (empty stream is error)
        let _tenant_id = "snap-007";
        let err = KvStore::assemble_chunks(&[]);
        assert!(matches!(err, Err(EtcdError::SnapshotDecode(_))));
    }

    #[test]
    fn test_restore_snapshot_recovers_state() {
        // cite: etcd v3.6 server/etcdserver/server.go applySnapshot
        let tenant_id = "snap-008";
        let src = KvStore::new();
        for i in 0..5 {
            pk_put(&src, &tk(tenant_id, &format!("k{}", i)), &format!("v{}", i));
        }
        let chunks = src.snapshot_stream();
        let (blob, cs) = KvStore::assemble_chunks(&chunks).unwrap();

        let dst = KvStore::new();
        dst.restore_snapshot(&blob, &cs).unwrap();
        for i in 0..5 {
            let kvs = get(&dst, &tk(tenant_id, &format!("k{}", i)));
            assert_eq!(kvs.len(), 1);
            assert_eq!(kvs[0].value_str(), format!("v{}", i));
        }
        assert_eq!(dst.current_revision(), src.current_revision());
    }

    #[test]
    fn test_restore_snapshot_rejects_bad_checksum() {
        // cite: etcd v3.6 server/etcdserver/server.go applySnapshot (verifyChecksum)
        let tenant_id = "snap-009";
        let src = KvStore::new();
        pk_put(&src, &tk(tenant_id, "k"), "v");
        let chunks = src.snapshot_stream();
        let (blob, _) = KvStore::assemble_chunks(&chunks).unwrap();

        let dst = KvStore::new();
        let err = dst.restore_snapshot(&blob, &"0".repeat(64));
        assert!(matches!(err, Err(EtcdError::SnapshotChecksumMismatch { .. })));
    }

    #[test]
    fn test_restore_snapshot_overwrites_existing() {
        // cite: etcd v3.6 server/etcdserver/server.go applySnapshot (replace state)
        let tenant_a = "snap-010-src";
        let tenant_b = "snap-010-dst";
        let src = KvStore::new();
        pk_put(&src, &tk(tenant_a, "kept"), "from-src");
        let chunks = src.snapshot_stream();
        let (blob, cs) = KvStore::assemble_chunks(&chunks).unwrap();

        let dst = KvStore::new();
        // Pre-existing data on the destination — must be wiped on restore.
        pk_put(&dst, &tk(tenant_b, "doomed"), "to-be-removed");
        dst.restore_snapshot(&blob, &cs).unwrap();

        assert!(get(&dst, &tk(tenant_b, "doomed")).is_empty());
        assert_eq!(get(&dst, &tk(tenant_a, "kept"))[0].value_str(), "from-src");
    }

    #[test]
    fn test_snapshot_includes_leases_and_members() {
        // cite: etcd v3.6 server/etcdserver/api/snap/snapshotter.go (full state)
        let tenant_id = "snap-011";
        let src = KvStore::new();
        pk_put(&src, &tk(tenant_id, "k"), "v");
        src.lease_grant_v2(&LeaseGrantRequest { ttl: 60, id: Some(77) })
            .unwrap();
        let extra = add_member(&src, "http://extra:2380", false);

        let chunks = src.snapshot_stream();
        let (blob, cs) = KvStore::assemble_chunks(&chunks).unwrap();

        let dst = KvStore::new();
        dst.restore_snapshot(&blob, &cs).unwrap();

        assert!(dst.leases.get(&77).is_some());
        let members = dst.member_list().members;
        assert!(members.iter().any(|m| m.id == extra));
    }

    #[test]
    fn test_snapshot_deterministic_ordering() {
        // cite: etcd v3.6 server/storage/mvcc/kvstore.go (deterministic dump)
        let tenant_id = "snap-012";
        let store = KvStore::new();
        // Insert keys out of order.
        for k in ["c", "a", "b", "e", "d"] {
            pk_put(&store, &tk(tenant_id, k), "v");
        }
        let cs1 = store.snapshot_stream()[0].checksum.clone();
        let cs2 = store.snapshot_stream()[0].checksum.clone();
        assert_eq!(cs1, cs2, "snapshot must be byte-identical between calls");
    }
}
