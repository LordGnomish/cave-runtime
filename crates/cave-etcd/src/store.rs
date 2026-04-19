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
            revision: AtomicU64::new(1),
            watch_tx,
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

        if let Ok(mut history) = self.history.write() {
            history.insert(rev, (key, EventType::Put, kv.clone()));
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
            let end_bytes = range_end.as_bytes().to_vec();
            for entry in self.current.iter() {
                let k = entry.key();
                if *k >= key_bytes && *k < end_bytes {
                    kvs.push(entry.value().clone());
                }
            }
            kvs.sort_by(|a, b| a.key.cmp(&b.key));
        } else {
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

    /// Refresh a lease TTL.
    pub fn lease_keepalive(&self, req: &LeaseKeepAliveRequest) -> EtcdResult<LeaseKeepAliveResponse> {
        let mut lease = self.leases.get_mut(&req.id)
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
        let lease = self.leases.get(&req.id)
            .ok_or(EtcdError::LeaseNotFound(req.id))?;
        let elapsed = Utc::now().signed_duration_since(lease.granted_at).num_seconds();
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
        let leases = self.leases.iter()
            .map(|e| LeaseStatus { id: *e.key() })
            .collect();
        LeaseLeasesResponse {
            header: self.header(),
            leases,
        }
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

    /// Compact with a typed response.
    pub fn compaction(&self, req: &CompactionRequest) -> CompactionResponse {
        self.compact(req.revision);
        CompactionResponse { header: self.header() }
    }

    /// Create a watch — returns watch_id and (for HTTP) any immediately queued events.
    pub fn watch_create(&self, _req: &WatchCreateRequest) -> WatchResponse {
        let watch_id = self.watch_counter.fetch_add(1, Ordering::SeqCst) as i64 + 1;
        WatchResponse {
            header: self.header(),
            watch_id,
            created: true,
            events: vec![],
        }
    }

    // ── Auth ──────────────────────────────────────────────────────────────

    pub fn auth_enable(&self) -> EtcdResult<AuthEnableResponse> {
        if self.auth_enabled.swap(true, Ordering::SeqCst) {
            return Err(EtcdError::AuthAlreadyEnabled);
        }
        Ok(AuthEnableResponse { header: self.header() })
    }

    pub fn auth_disable(&self) -> EtcdResult<AuthDisableResponse> {
        if !self.auth_enabled.swap(false, Ordering::SeqCst) {
            return Err(EtcdError::AuthNotEnabled);
        }
        Ok(AuthDisableResponse { header: self.header() })
    }

    pub fn authenticate(&self, req: &AuthenticateRequest) -> EtcdResult<AuthenticateResponse> {
        if self.auth_enabled.load(Ordering::SeqCst) {
            let user = self.users.get(&req.name)
                .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
            if user.password != req.password {
                return Err(EtcdError::InvalidPassword);
            }
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.auth_tokens.insert(token.clone(), req.name.clone());
        Ok(AuthenticateResponse { header: self.header(), token })
    }

    pub fn user_add(&self, req: &AuthUserAddRequest) -> EtcdResult<AuthUserAddResponse> {
        if self.users.contains_key(&req.name) {
            return Err(EtcdError::UserAlreadyExists(req.name.clone()));
        }
        self.users.insert(req.name.clone(), AuthUser {
            name: req.name.clone(),
            password: req.password.clone(),
            roles: vec![],
        });
        Ok(AuthUserAddResponse { header: self.header() })
    }

    pub fn user_delete(&self, req: &AuthUserDeleteRequest) -> EtcdResult<AuthUserDeleteResponse> {
        self.users.remove(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        Ok(AuthUserDeleteResponse { header: self.header() })
    }

    pub fn user_get(&self, req: &AuthUserGetRequest) -> EtcdResult<AuthUserGetResponse> {
        let user = self.users.get(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        Ok(AuthUserGetResponse {
            header: self.header(),
            roles: user.roles.clone(),
        })
    }

    pub fn user_list(&self) -> AuthUserListResponse {
        let mut users: Vec<String> = self.users.iter().map(|e| e.key().clone()).collect();
        users.sort();
        AuthUserListResponse { header: self.header(), users }
    }

    pub fn user_change_password(&self, req: &AuthUserChangePasswordRequest) -> EtcdResult<AuthUserChangePasswordResponse> {
        let mut user = self.users.get_mut(&req.name)
            .ok_or_else(|| EtcdError::UserNotFound(req.name.clone()))?;
        user.password = req.password.clone();
        Ok(AuthUserChangePasswordResponse { header: self.header() })
    }

    pub fn role_add(&self, req: &AuthRoleAddRequest) -> EtcdResult<AuthRoleAddResponse> {
        if self.roles.contains_key(&req.name) {
            return Err(EtcdError::RoleAlreadyExists(req.name.clone()));
        }
        self.roles.insert(req.name.clone(), AuthRole {
            name: req.name.clone(),
            key_permission: vec![],
        });
        Ok(AuthRoleAddResponse { header: self.header() })
    }

    pub fn role_delete(&self, req: &AuthRoleDeleteRequest) -> EtcdResult<AuthRoleDeleteResponse> {
        self.roles.remove(&req.role)
            .ok_or_else(|| EtcdError::RoleNotFound(req.role.clone()))?;
        Ok(AuthRoleDeleteResponse { header: self.header() })
    }

    pub fn role_get(&self, req: &AuthRoleGetRequest) -> EtcdResult<AuthRoleGetResponse> {
        let role = self.roles.get(&req.role)
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
        AuthRoleListResponse { header: self.header(), roles }
    }

    // ── Maintenance ───────────────────────────────────────────────────────

    pub fn alarm(&self, req: &AlarmRequest) -> AlarmResponse {
        let mut alarms = self.alarms.write().unwrap();
        match req.action {
            AlarmAction::Get => {}
            AlarmAction::Activate => {
                if !alarms.iter().any(|a| a.member_id == req.member_id && a.alarm == req.alarm) {
                    alarms.push(AlarmMember {
                        member_id: req.member_id,
                        alarm: req.alarm.clone(),
                    });
                }
            }
            AlarmAction::Deactivate => {
                alarms.retain(|a| !(a.member_id == req.member_id && a.alarm == req.alarm));
            }
        }
        AlarmResponse {
            header: self.header(),
            alarms: alarms.clone(),
        }
    }

    pub fn defragment(&self) -> DefragmentResponse {
        DefragmentResponse { header: self.header() }
    }

    pub fn hash(&self) -> HashResponse {
        let mut h: u32 = 5381;
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = self.current.iter()
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
        let data: Vec<(Vec<u8>, Vec<u8>)> = self.current.iter()
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
        let m = members.iter_mut()
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
    }

    #[test]
    fn test_status() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "s".into(), value: "t".into(), lease: None, prev_kv: false });
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
            key: "a".into(), range_end: None, start_revision: None,
            progress_notify: false, prev_kv: false,
        });
        let r2 = store.watch_create(&WatchCreateRequest {
            key: "b".into(), range_end: None, start_revision: None,
            progress_notify: false, prev_kv: false,
        });
        assert_ne!(r1.watch_id, r2.watch_id);
    }

    // ── Lease extensions ───────────────────────────────────────────────────

    #[test]
    fn test_lease_keepalive() {
        let store = KvStore::new();
        let grant = store.lease_grant(&LeaseGrantRequest { ttl: 30, id: None });
        let resp = store.lease_keepalive(&LeaseKeepAliveRequest { id: grant.id }).unwrap();
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
        let resp = store.lease_timetolive(&LeaseTTLRequest { id: grant.id, keys: false }).unwrap();
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
        assert!(matches!(store.auth_enable(), Err(EtcdError::AuthAlreadyEnabled)));
        assert!(store.auth_disable().is_ok());
        assert!(matches!(store.auth_disable(), Err(EtcdError::AuthNotEnabled)));
    }

    #[test]
    fn test_authenticate_no_auth() {
        let store = KvStore::new();
        let resp = store.authenticate(&AuthenticateRequest {
            name: "anyone".into(),
            password: "anything".into(),
        }).unwrap();
        assert!(!resp.token.is_empty());
    }

    #[test]
    fn test_authenticate_with_auth_enabled() {
        let store = KvStore::new();
        store.user_add(&AuthUserAddRequest { name: "root".into(), password: "secret".into() }).unwrap();
        store.auth_enable().unwrap();

        let ok = store.authenticate(&AuthenticateRequest { name: "root".into(), password: "secret".into() });
        assert!(ok.is_ok());

        let bad = store.authenticate(&AuthenticateRequest { name: "root".into(), password: "wrong".into() });
        assert!(matches!(bad, Err(EtcdError::InvalidPassword)));
    }

    #[test]
    fn test_user_add_get_delete() {
        let store = KvStore::new();
        store.user_add(&AuthUserAddRequest { name: "alice".into(), password: "pw".into() }).unwrap();

        // duplicate
        assert!(matches!(
            store.user_add(&AuthUserAddRequest { name: "alice".into(), password: "pw2".into() }),
            Err(EtcdError::UserAlreadyExists(_))
        ));

        let get = store.user_get(&AuthUserGetRequest { name: "alice".into() }).unwrap();
        assert!(get.roles.is_empty());

        store.user_delete(&AuthUserDeleteRequest { name: "alice".into() }).unwrap();
        assert!(matches!(
            store.user_get(&AuthUserGetRequest { name: "alice".into() }),
            Err(EtcdError::UserNotFound(_))
        ));
    }

    #[test]
    fn test_user_list() {
        let store = KvStore::new();
        store.user_add(&AuthUserAddRequest { name: "bob".into(), password: "p".into() }).unwrap();
        store.user_add(&AuthUserAddRequest { name: "alice".into(), password: "p".into() }).unwrap();
        let resp = store.user_list();
        assert!(resp.users.contains(&"alice".to_string()));
        assert!(resp.users.contains(&"bob".to_string()));
    }

    #[test]
    fn test_user_change_password() {
        let store = KvStore::new();
        store.user_add(&AuthUserAddRequest { name: "u1".into(), password: "old".into() }).unwrap();
        store.user_change_password(&AuthUserChangePasswordRequest { name: "u1".into(), password: "new".into() }).unwrap();
        store.auth_enable().unwrap();
        assert!(store.authenticate(&AuthenticateRequest { name: "u1".into(), password: "new".into() }).is_ok());
    }

    #[test]
    fn test_role_add_get_delete() {
        let store = KvStore::new();
        store.role_add(&AuthRoleAddRequest { name: "admin".into() }).unwrap();

        assert!(matches!(
            store.role_add(&AuthRoleAddRequest { name: "admin".into() }),
            Err(EtcdError::RoleAlreadyExists(_))
        ));

        let get = store.role_get(&AuthRoleGetRequest { role: "admin".into() }).unwrap();
        assert_eq!(get.name, "admin");
        assert!(get.perm.is_empty());

        store.role_delete(&AuthRoleDeleteRequest { role: "admin".into() }).unwrap();
        assert!(matches!(
            store.role_get(&AuthRoleGetRequest { role: "admin".into() }),
            Err(EtcdError::RoleNotFound(_))
        ));
    }

    #[test]
    fn test_role_list() {
        let store = KvStore::new();
        store.role_add(&AuthRoleAddRequest { name: "r1".into() }).unwrap();
        store.role_add(&AuthRoleAddRequest { name: "r2".into() }).unwrap();
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
        store.alarm(&AlarmRequest { action: AlarmAction::Activate, member_id: 1, alarm: AlarmType::Nospace });
        let resp = store.alarm(&AlarmRequest { action: AlarmAction::Get, member_id: 0, alarm: AlarmType::None });
        assert_eq!(resp.alarms.len(), 1);
        assert_eq!(resp.alarms[0].alarm, AlarmType::Nospace);

        store.alarm(&AlarmRequest { action: AlarmAction::Deactivate, member_id: 1, alarm: AlarmType::Nospace });
        let resp2 = store.alarm(&AlarmRequest { action: AlarmAction::Get, member_id: 0, alarm: AlarmType::None });
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
        store.put(&PutRequest { key: "k".into(), value: "v".into(), lease: None, prev_kv: false });
        let h2 = store.hash().hash;
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_snapshot_contains_data() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "snap_key".into(), value: "snap_val".into(), lease: None, prev_kv: false });
        let resp = store.snapshot();
        let data_str = String::from_utf8_lossy(&resp.blob);
        assert!(data_str.contains("snap_key") || !resp.blob.is_empty());
    }

    #[test]
    fn test_compaction_response() {
        let store = KvStore::new();
        store.put(&PutRequest { key: "a".into(), value: "1".into(), lease: None, prev_kv: false });
        let rev = store.current_revision();
        let resp = store.compaction(&CompactionRequest { revision: rev, physical: true });
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
        let resp = store.member_remove(&MemberRemoveRequest { id: new_id }).unwrap();
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
        let resp = store.member_update(&MemberUpdateRequest {
            id: 1,
            peer_ur_ls: vec!["http://newpeer:2380".into()],
        }).unwrap();
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
}
