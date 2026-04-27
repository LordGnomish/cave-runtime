//! ClusterMesh deepening — KVStore (etcd-style), RemoteCluster lifecycle,
//! global service shadowing, service affinity.
//!
//! Mirrors:
//!
//! * `pkg/kvstore/etcd.go` — etcd-shaped key/value store with prefix
//!   watches, lease-based TTL, compare-and-swap, monotonic revisions.
//! * `pkg/clustermesh/clustermesh.go::RemoteCluster` lifecycle states.
//! * `pkg/service/global.go` — global service announcement + lookup
//!   with affinity (local / remote / none) and the shadowing rule that
//!   a local endpoint takes precedence over remote ones.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

// ── KVStore ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvValue {
    pub value: Vec<u8>,
    pub mod_revision: u64,
    pub create_revision: u64,
    pub lease_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KvEvent {
    Put { key: String, value: KvValue },
    Delete { key: String, prev_value: Option<KvValue> },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KvError {
    #[error("compare-and-swap failed: expected revision {expected}, got {actual}")]
    CasMismatch { expected: u64, actual: u64 },
    #[error("lease {0} not found")]
    LeaseNotFound(u64),
    #[error("tenant {tenant} cannot mutate kvstore owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct KvStore {
    pub tenant: TenantId,
    revision: u64,
    data: BTreeMap<String, KvValue>,
    /// Active leases: id → expires_at.
    leases: BTreeMap<u64, u64>,
    /// Watcher buffers keyed by prefix.
    watchers: Vec<(String, VecDeque<KvEvent>)>,
    next_lease: u64,
}

impl KvStore {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant, revision: 0,
            data: BTreeMap::new(),
            leases: BTreeMap::new(),
            watchers: Vec::new(),
            next_lease: 1,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn put(&mut self, key: impl Into<String>, value: Vec<u8>, lease: Option<u64>) -> KvValue {
        let key = key.into();
        self.revision += 1;
        let create_rev = self.data.get(&key).map(|v| v.create_revision).unwrap_or(self.revision);
        let v = KvValue {
            value, mod_revision: self.revision,
            create_revision: create_rev, lease_id: lease,
        };
        self.data.insert(key.clone(), v.clone());
        self.fan_out(KvEvent::Put { key, value: v.clone() });
        v
    }

    pub fn get(&self, key: &str) -> Option<&KvValue> {
        self.data.get(key)
    }

    pub fn delete(&mut self, key: &str) -> Option<KvValue> {
        let prev = self.data.remove(key);
        if prev.is_some() {
            self.revision += 1;
            self.fan_out(KvEvent::Delete { key: key.to_string(), prev_value: prev.clone() });
        }
        prev
    }

    pub fn list_prefix(&self, prefix: &str) -> Vec<(String, KvValue)> {
        self.data.range(prefix.to_string()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn count_prefix(&self, prefix: &str) -> usize {
        self.list_prefix(prefix).len()
    }

    /// Compare-and-swap: only writes if `key`'s `mod_revision` matches
    /// `expected`. Returns the new value on success.
    pub fn cas(&mut self, key: &str, expected: u64, value: Vec<u8>) -> Result<KvValue, KvError> {
        let actual = self.data.get(key).map(|v| v.mod_revision).unwrap_or(0);
        if actual != expected {
            return Err(KvError::CasMismatch { expected, actual });
        }
        Ok(self.put(key.to_string(), value, None))
    }

    pub fn grant_lease(&mut self, ttl_seconds: u64, now: u64) -> u64 {
        let id = self.next_lease;
        self.next_lease += 1;
        self.leases.insert(id, now + ttl_seconds);
        id
    }

    /// Reap expired leases and the keys attached to them. Returns the
    /// number of removed keys.
    pub fn revoke_expired(&mut self, now: u64) -> usize {
        let expired: Vec<u64> = self.leases.iter()
            .filter(|(_, &exp)| exp <= now)
            .map(|(&id, _)| id)
            .collect();
        let mut removed = 0;
        for id in expired {
            self.leases.remove(&id);
            let keys: Vec<String> = self.data.iter()
                .filter(|(_, v)| v.lease_id == Some(id))
                .map(|(k, _)| k.clone())
                .collect();
            for k in keys {
                self.delete(&k);
                removed += 1;
            }
        }
        removed
    }

    pub fn watch(&mut self, prefix: impl Into<String>) -> usize {
        let id = self.watchers.len();
        self.watchers.push((prefix.into(), VecDeque::new()));
        id
    }

    pub fn drain_watch(&mut self, watch_id: usize) -> Vec<KvEvent> {
        if let Some((_, q)) = self.watchers.get_mut(watch_id) {
            std::mem::take(q).into_iter().collect()
        } else {
            Vec::new()
        }
    }

    fn fan_out(&mut self, event: KvEvent) {
        let key = match &event {
            KvEvent::Put { key, .. } | KvEvent::Delete { key, .. } => key.clone(),
        };
        for (prefix, q) in self.watchers.iter_mut() {
            if key.starts_with(prefix.as_str()) {
                q.push_back(event.clone());
            }
        }
    }
}

// ── RemoteCluster lifecycle ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteClusterState {
    Connecting,
    Synced,
    Failed,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteClusterStatus {
    pub name: String,
    pub state: RemoteClusterState,
    pub last_heartbeat: u64,
    pub last_error: Option<String>,
    pub identities_count: u64,
    pub services_count: u64,
}

impl RemoteClusterStatus {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: RemoteClusterState::Connecting,
            last_heartbeat: 0,
            last_error: None,
            identities_count: 0,
            services_count: 0,
        }
    }
    pub fn is_ready(&self) -> bool {
        matches!(self.state, RemoteClusterState::Synced)
    }

    /// Apply a state transition. Mirrors the upstream
    /// `RemoteCluster.SetState` rules.
    pub fn transition(&mut self, to: RemoteClusterState) -> Result<(), KvError> {
        let from = self.state;
        let ok = matches!(
            (from, to),
            (RemoteClusterState::Connecting, RemoteClusterState::Synced)
                | (RemoteClusterState::Connecting, RemoteClusterState::Failed)
                | (RemoteClusterState::Connecting, RemoteClusterState::Disconnected)
                | (RemoteClusterState::Synced, RemoteClusterState::Failed)
                | (RemoteClusterState::Synced, RemoteClusterState::Disconnected)
                | (RemoteClusterState::Synced, RemoteClusterState::Synced)
                | (RemoteClusterState::Failed, RemoteClusterState::Connecting)
                | (RemoteClusterState::Disconnected, RemoteClusterState::Connecting)
        );
        if !ok {
            return Err(KvError::CasMismatch { expected: 0, actual: 0 });
        }
        self.state = to;
        Ok(())
    }
    pub fn heartbeat(&mut self, now: u64) {
        self.last_heartbeat = now;
    }
    /// Mark the cluster failed if the last heartbeat is older than
    /// `stale_seconds` ago.
    pub fn check_stale(&mut self, now: u64, stale_seconds: u64) {
        if matches!(self.state, RemoteClusterState::Synced)
            && now.saturating_sub(self.last_heartbeat) >= stale_seconds
        {
            let _ = self.transition(RemoteClusterState::Failed);
            self.last_error = Some("heartbeat stale".into());
        }
    }
}

// ── Global service ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceAffinity {
    None,
    /// Prefer endpoints in the same cluster as the source.
    Local,
    /// Prefer endpoints in remote clusters.
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalServiceEndpoint {
    pub cluster: String,
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalService {
    pub name: String,
    pub namespace: String,
    pub affinity: ServiceAffinity,
    pub local_endpoints: Vec<GlobalServiceEndpoint>,
    pub remote_endpoints: Vec<GlobalServiceEndpoint>,
}

impl GlobalService {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>, affinity: ServiceAffinity) -> Self {
        Self {
            name: name.into(), namespace: namespace.into(), affinity,
            local_endpoints: Vec::new(), remote_endpoints: Vec::new(),
        }
    }

    /// Resolve the active backend pool given the requesting cluster
    /// (`Local` keeps local-only when local has at least one backend,
    /// otherwise falls through to remote; `Remote` is the inverse;
    /// `None` returns everything).
    pub fn resolve(&self) -> Vec<&GlobalServiceEndpoint> {
        match self.affinity {
            ServiceAffinity::None => {
                let mut v: Vec<&GlobalServiceEndpoint> = Vec::new();
                v.extend(&self.local_endpoints);
                v.extend(&self.remote_endpoints);
                v
            }
            ServiceAffinity::Local => {
                if !self.local_endpoints.is_empty() {
                    self.local_endpoints.iter().collect()
                } else {
                    self.remote_endpoints.iter().collect()
                }
            }
            ServiceAffinity::Remote => {
                if !self.remote_endpoints.is_empty() {
                    self.remote_endpoints.iter().collect()
                } else {
                    self.local_endpoints.iter().collect()
                }
            }
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/kvstore/etcd.go", "etcdClient");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    // ── KVStore ──────────────────────────────────────────────────────────────

    #[test]
    fn kv_put_then_get_returns_value() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Put", "tenant-kv-pg");
        let mut s = KvStore::new(tenant);
        s.put("/cilium/state/k1", b"v1".to_vec(), None);
        assert_eq!(s.get("/cilium/state/k1").unwrap().value, b"v1");
    }

    #[test]
    fn kv_get_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Get.NotFound", "tenant-kv-nf");
        let s = KvStore::new(tenant);
        assert!(s.get("missing").is_none());
    }

    #[test]
    fn kv_revision_increments_on_each_write() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Put.Revision", "tenant-kv-rev");
        let mut s = KvStore::new(tenant);
        s.put("a", b"1".to_vec(), None);
        s.put("b", b"2".to_vec(), None);
        s.put("a", b"3".to_vec(), None);
        assert_eq!(s.revision(), 3);
    }

    #[test]
    fn kv_create_revision_persists_across_updates() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Put.CreateRevision", "tenant-kv-crev");
        let mut s = KvStore::new(tenant);
        let v1 = s.put("a", b"1".to_vec(), None);
        let v2 = s.put("a", b"2".to_vec(), None);
        assert_eq!(v1.create_revision, v2.create_revision);
        assert_ne!(v1.mod_revision, v2.mod_revision);
    }

    #[test]
    fn kv_delete_removes_key_and_returns_prev() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Delete", "tenant-kv-del");
        let mut s = KvStore::new(tenant);
        s.put("a", b"1".to_vec(), None);
        let prev = s.delete("a").unwrap();
        assert_eq!(prev.value, b"1");
        assert!(s.get("a").is_none());
    }

    #[test]
    fn kv_list_prefix_returns_matching_keys() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "List.Prefix", "tenant-kv-lp");
        let mut s = KvStore::new(tenant);
        s.put("/cilium/identities/256", b"a".to_vec(), None);
        s.put("/cilium/identities/257", b"b".to_vec(), None);
        s.put("/cilium/services/foo", b"c".to_vec(), None);
        let r = s.list_prefix("/cilium/identities/");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn kv_count_prefix() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Count.Prefix", "tenant-kv-cp");
        let mut s = KvStore::new(tenant);
        for i in 0..7u32 {
            s.put(format!("/p/{i}"), b"x".to_vec(), None);
        }
        assert_eq!(s.count_prefix("/p/"), 7);
    }

    #[test]
    fn kv_cas_succeeds_on_correct_revision() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "CAS", "tenant-kv-cas");
        let mut s = KvStore::new(tenant);
        let v = s.put("a", b"1".to_vec(), None);
        let r = s.cas("a", v.mod_revision, b"2".to_vec()).unwrap();
        assert_eq!(r.value, b"2");
    }

    #[test]
    fn kv_cas_fails_on_stale_revision() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "CAS.Stale", "tenant-kv-cas-stale");
        let mut s = KvStore::new(tenant);
        s.put("a", b"1".to_vec(), None);
        s.put("a", b"2".to_vec(), None);
        let err = s.cas("a", 1, b"3".to_vec()).unwrap_err();
        match err {
            KvError::CasMismatch { expected: 1, actual: 2 } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn kv_watch_emits_put_event() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Watch.Put", "tenant-kv-w-put");
        let mut s = KvStore::new(tenant);
        let w = s.watch("/cilium/");
        s.put("/cilium/x", b"v".to_vec(), None);
        let evs = s.drain_watch(w);
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], KvEvent::Put { .. }));
    }

    #[test]
    fn kv_watch_emits_delete_event() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Watch.Delete", "tenant-kv-w-del");
        let mut s = KvStore::new(tenant);
        s.put("/cilium/x", b"v".to_vec(), None);
        let w = s.watch("/cilium/");
        s.delete("/cilium/x");
        let evs = s.drain_watch(w);
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], KvEvent::Delete { .. }));
    }

    #[test]
    fn kv_watch_filters_by_prefix() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Watch.Filter", "tenant-kv-w-flt");
        let mut s = KvStore::new(tenant);
        let w = s.watch("/cilium/identities/");
        s.put("/cilium/identities/256", b"a".to_vec(), None);
        s.put("/cilium/services/foo", b"b".to_vec(), None);
        let evs = s.drain_watch(w);
        assert_eq!(evs.len(), 1);
    }

    #[test]
    fn kv_lease_expiry_revokes_attached_keys() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Lease.Revoke", "tenant-kv-lease");
        let mut s = KvStore::new(tenant);
        let lease = s.grant_lease(60, 100);
        s.put("ephemeral", b"x".to_vec(), Some(lease));
        s.put("permanent", b"y".to_vec(), None);
        let removed = s.revoke_expired(200);
        assert_eq!(removed, 1);
        assert!(s.get("ephemeral").is_none());
        assert!(s.get("permanent").is_some());
    }

    #[test]
    fn kv_lease_not_expired_keeps_keys() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Lease.NotExpired", "tenant-kv-lease-ok");
        let mut s = KvStore::new(tenant);
        let lease = s.grant_lease(60, 100);
        s.put("k", b"v".to_vec(), Some(lease));
        let removed = s.revoke_expired(150);
        assert_eq!(removed, 0);
        assert!(s.get("k").is_some());
    }

    #[test]
    fn kv_drain_watch_returns_empty_after_drain() {
        let (_c, tenant) = cilium_test_ctx!("pkg/kvstore/etcd.go", "Watch.Drain", "tenant-kv-w-drain");
        let mut s = KvStore::new(tenant);
        let w = s.watch("/p/");
        s.put("/p/a", b"x".to_vec(), None);
        let _ = s.drain_watch(w);
        assert_eq!(s.drain_watch(w).len(), 0);
    }

    // ── RemoteCluster lifecycle ──────────────────────────────────────────────

    #[test]
    fn rc_initial_state_is_connecting() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Init", "tenant-rc-init");
        let s = RemoteClusterStatus::new("us-east");
        assert_eq!(s.state, RemoteClusterState::Connecting);
        assert!(!s.is_ready());
    }

    #[test]
    fn rc_advances_to_synced() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Synced", "tenant-rc-sync");
        let mut s = RemoteClusterStatus::new("us-east");
        s.transition(RemoteClusterState::Synced).unwrap();
        assert!(s.is_ready());
    }

    #[test]
    fn rc_advances_to_failed() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Failed", "tenant-rc-fail");
        let mut s = RemoteClusterStatus::new("us-east");
        s.transition(RemoteClusterState::Failed).unwrap();
        assert!(!s.is_ready());
    }

    #[test]
    fn rc_failed_can_recover_to_connecting() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Recover", "tenant-rc-rec");
        let mut s = RemoteClusterStatus::new("us-east");
        s.transition(RemoteClusterState::Failed).unwrap();
        s.transition(RemoteClusterState::Connecting).unwrap();
        assert_eq!(s.state, RemoteClusterState::Connecting);
    }

    #[test]
    fn rc_invalid_transition_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.BadTransition", "tenant-rc-bad");
        let mut s = RemoteClusterStatus::new("us-east");
        s.transition(RemoteClusterState::Synced).unwrap();
        // Synced → Connecting is not allowed.
        assert!(s.transition(RemoteClusterState::Connecting).is_err());
    }

    #[test]
    fn rc_heartbeat_updates_last_seen() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Heartbeat", "tenant-rc-hb");
        let mut s = RemoteClusterStatus::new("us-east");
        s.heartbeat(1234);
        assert_eq!(s.last_heartbeat, 1234);
    }

    #[test]
    fn rc_check_stale_marks_failed_after_threshold() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Stale", "tenant-rc-stale");
        let mut s = RemoteClusterStatus::new("us-east");
        s.transition(RemoteClusterState::Synced).unwrap();
        s.heartbeat(100);
        s.check_stale(200, 30);
        assert_eq!(s.state, RemoteClusterState::Failed);
        assert!(s.last_error.is_some());
    }

    #[test]
    fn rc_check_stale_keeps_synced_within_threshold() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.NotStale", "tenant-rc-fresh");
        let mut s = RemoteClusterStatus::new("us-east");
        s.transition(RemoteClusterState::Synced).unwrap();
        s.heartbeat(100);
        s.check_stale(110, 30);
        assert_eq!(s.state, RemoteClusterState::Synced);
    }

    #[test]
    fn rc_status_round_trips_serde() {
        let (_c, _t) = cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "RemoteCluster.Serde", "tenant-rc-serde");
        let s = RemoteClusterStatus {
            name: "us-east".into(),
            state: RemoteClusterState::Synced,
            last_heartbeat: 100,
            last_error: None,
            identities_count: 5,
            services_count: 3,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: RemoteClusterStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ── Global service ───────────────────────────────────────────────────────

    #[test]
    fn svc_affinity_local_prefers_local_endpoints() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "Affinity.Local", "tenant-svc-aff-loc");
        let mut s = GlobalService::new("api", "default", ServiceAffinity::Local);
        s.local_endpoints.push(GlobalServiceEndpoint { cluster: "self".into(), address: "10.0.1.1".into(), port: 80 });
        s.remote_endpoints.push(GlobalServiceEndpoint { cluster: "us-east".into(), address: "10.0.2.1".into(), port: 80 });
        let r = s.resolve();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cluster, "self");
    }

    #[test]
    fn svc_affinity_local_falls_back_to_remote_when_no_local() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "Affinity.LocalFallback", "tenant-svc-aff-locf");
        let mut s = GlobalService::new("api", "default", ServiceAffinity::Local);
        s.remote_endpoints.push(GlobalServiceEndpoint { cluster: "us-east".into(), address: "10.0.2.1".into(), port: 80 });
        let r = s.resolve();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cluster, "us-east");
    }

    #[test]
    fn svc_affinity_remote_prefers_remote_endpoints() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "Affinity.Remote", "tenant-svc-aff-rem");
        let mut s = GlobalService::new("api", "default", ServiceAffinity::Remote);
        s.local_endpoints.push(GlobalServiceEndpoint { cluster: "self".into(), address: "10.0.1.1".into(), port: 80 });
        s.remote_endpoints.push(GlobalServiceEndpoint { cluster: "us-east".into(), address: "10.0.2.1".into(), port: 80 });
        let r = s.resolve();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cluster, "us-east");
    }

    #[test]
    fn svc_affinity_none_returns_all_endpoints() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "Affinity.None", "tenant-svc-aff-none");
        let mut s = GlobalService::new("api", "default", ServiceAffinity::None);
        s.local_endpoints.push(GlobalServiceEndpoint { cluster: "self".into(), address: "10.0.1.1".into(), port: 80 });
        s.remote_endpoints.push(GlobalServiceEndpoint { cluster: "us-east".into(), address: "10.0.2.1".into(), port: 80 });
        s.remote_endpoints.push(GlobalServiceEndpoint { cluster: "eu-west".into(), address: "10.0.3.1".into(), port: 80 });
        let r = s.resolve();
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn svc_global_service_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "GlobalService.Serde", "tenant-svc-gs-serde");
        let s = GlobalService::new("api", "default", ServiceAffinity::Local);
        let json = serde_json::to_string(&s).unwrap();
        let back: GlobalService = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn svc_affinity_local_with_no_endpoints_returns_empty() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "Affinity.Empty", "tenant-svc-aff-emp");
        let s = GlobalService::new("api", "default", ServiceAffinity::Local);
        let r = s.resolve();
        assert!(r.is_empty());
    }

    #[test]
    fn svc_affinity_remote_falls_back_to_local() {
        let (_c, _t) = cilium_test_ctx!("pkg/service/global.go", "Affinity.RemoteFallback", "tenant-svc-aff-remf");
        let mut s = GlobalService::new("api", "default", ServiceAffinity::Remote);
        s.local_endpoints.push(GlobalServiceEndpoint { cluster: "self".into(), address: "10.0.1.1".into(), port: 80 });
        let r = s.resolve();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cluster, "self");
    }
}
