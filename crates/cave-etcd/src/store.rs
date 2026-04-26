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
use tokio::sync::broadcast;

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
            leases: DashMap::new(),
            lease_counter: AtomicU64::new(1),
            compacted_revision: AtomicU64::new(0),
            auth_enabled: AtomicBool::new(false),
            users: DashMap::new(),
            roles: DashMap::new(),
            auth_tokens: DashMap::new(),
            alarms: RwLock::new(Vec::new()),
            members: RwLock::new(initial_members),
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
                    let _ = self.watch_tx.send(WatchEvent {
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
            let _ = self.watch_tx.send(WatchEvent {
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
                let _ = self.watch_tx.send(WatchEvent {
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
            "version": "3.5.0-cave",
            "dbSize": self.current.len(),
            "leader": 1,
            "raftIndex": self.current_revision(),
            "raftTerm": 1,
        })
    }

    pub fn version(&self) -> VersionResponse {
        VersionResponse {
            etcdserver: "3.5.0-cave".to_string(),
            etcdcluster: "3.5.0".to_string(),
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
}
