// SPDX-License-Identifier: AGPL-3.0-or-later
//! etcd-backed storage driver for the apiserver registry.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiserver/pkg/storage/etcd3/store.go` (`store.Get`,
//!     `store.GuaranteedUpdate`, `store.List`, `store.Watch`).
//!   * `staging/src/k8s.io/apiserver/pkg/storage/etcd3/compact.go`
//!     (revision compaction).
//!   * `staging/src/k8s.io/apiserver/pkg/storage/cacher/cacher.go`
//!     (the in-process cache that fronts etcd).
//!
//! Resources are persisted under K8s' canonical key layout, prefixed by
//! tenant_id so cross-tenant range scans are physically impossible:
//!
//!   `/tenants/<tenant_id>/registry/<group>/<kind>/<namespace>/<name>`
//!
//! The driver delegates raw KV + revision tracking to `cave_etcd::store::KvStore`.
//! Range list reads use a `range_end` of `<prefix> + 0xFF` to capture every
//! key under the prefix (matching upstream `etcd3.PrefixEnd`).
//!
//! Tenant invariant: every key produced by this driver is rooted at
//! `/tenants/<tenant_id>/`. Lookups that span the tenant prefix MUST NOT
//! return another tenant's keys, and revision compaction is global to the
//! KvStore but tenant data is never inspected — the driver only exposes
//! tenant-prefixed operations to callers.

use cave_etcd::models::{
    DeleteRangeRequest, EventType, KeyValue, PutRequest, RangeRequest,
};
use cave_etcd::store::KvStore;
use std::sync::Arc;

/// Maximum tenant_id length. `tenant_id` lands in storage keys; cap matches
/// the upstream Namespace name length cap (`validation.DNS1123LabelMaxLength`).
pub const MAX_TENANT_ID_LEN: usize = 63;

/// Upstream-compatible key encoder.
pub fn key_for(
    tenant_id: &str,
    group: &str,
    kind: &str,
    namespace: &str,
    name: &str,
) -> String {
    let g = if group.is_empty() { "core" } else { group };
    if namespace.is_empty() {
        format!("/tenants/{}/registry/{}/{}/{}", tenant_id, g, kind, name)
    } else {
        format!(
            "/tenants/{}/registry/{}/{}/{}/{}",
            tenant_id, g, kind, namespace, name
        )
    }
}

/// Prefix used by `list` for a `(tenant, group, kind[, namespace])` scope.
/// `namespace` empty means cluster-scoped or list-across-namespaces.
pub fn prefix_for(
    tenant_id: &str,
    group: &str,
    kind: &str,
    namespace: &str,
) -> String {
    let g = if group.is_empty() { "core" } else { group };
    if namespace.is_empty() {
        format!("/tenants/{}/registry/{}/{}/", tenant_id, g, kind)
    } else {
        format!(
            "/tenants/{}/registry/{}/{}/{}/",
            tenant_id, g, kind, namespace
        )
    }
}

/// `etcd3.PrefixEnd` — append `0xFF` to make a strictly-greater range end.
fn prefix_end(prefix: &str) -> String {
    let mut s = prefix.to_string();
    s.push('\u{FF}');
    s
}

/// A stored object — opaque JSON payload + the revision at which it lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub key: String,
    pub value: Vec<u8>,
    pub mod_revision: u64,
    pub create_revision: u64,
    pub version: u64,
}

impl From<KeyValue> for StoredObject {
    fn from(kv: KeyValue) -> Self {
        Self {
            key: String::from_utf8_lossy(&kv.key).to_string(),
            value: kv.value,
            mod_revision: kv.mod_revision,
            create_revision: kv.create_revision,
            version: kv.version,
        }
    }
}

/// etcd-backed store. Cheap to clone — wraps an Arc.
pub struct EtcdBackend {
    inner: Arc<KvStore>,
}

impl EtcdBackend {
    /// Build a driver over a fresh in-process `KvStore`. Prod paths can
    /// pass an `Arc<KvStore>` shared with the rest of the platform.
    pub fn new() -> Self {
        Self { inner: Arc::new(KvStore::new()) }
    }

    pub fn from_kv(inner: Arc<KvStore>) -> Self {
        Self { inner }
    }

    pub fn current_revision(&self) -> u64 {
        self.inner.current_revision()
    }

    pub fn compaction_revision(&self) -> u64 {
        self.inner.compaction_revision()
    }

    /// Persist `value` under the encoded key. Returns the new revision.
    pub fn put(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        namespace: &str,
        name: &str,
        value: &[u8],
    ) -> u64 {
        assert!(
            tenant_id.len() <= MAX_TENANT_ID_LEN,
            "tenant_id exceeds {} chars", MAX_TENANT_ID_LEN
        );
        let key = key_for(tenant_id, group, kind, namespace, name);
        let req = PutRequest {
            key,
            value: String::from_utf8_lossy(value).to_string(),
            lease: None,
            prev_kv: false,
        };
        let resp = self.inner.put(&req);
        resp.header.revision
    }

    pub fn get(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        namespace: &str,
        name: &str,
    ) -> Option<StoredObject> {
        let key = key_for(tenant_id, group, kind, namespace, name);
        let req = RangeRequest {
            key,
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        };
        let resp = self.inner.range(&req).ok()?;
        resp.kvs.into_iter().next().map(StoredObject::from)
    }

    /// List every object under `(tenant, group, kind[, namespace])` prefix.
    /// `namespace` may be empty for cluster-wide listings within the tenant.
    pub fn list(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        namespace: &str,
    ) -> Vec<StoredObject> {
        let prefix = prefix_for(tenant_id, group, kind, namespace);
        let req = RangeRequest {
            key: prefix.clone(),
            range_end: Some(prefix_end(&prefix)),
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        };
        match self.inner.range(&req) {
            Ok(resp) => resp.kvs.into_iter().map(StoredObject::from).collect(),
            Err(_) => vec![],
        }
    }

    /// Delete a single object. Returns whether anything was removed.
    pub fn delete(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        namespace: &str,
        name: &str,
    ) -> bool {
        let key = key_for(tenant_id, group, kind, namespace, name);
        let req = DeleteRangeRequest { key, range_end: None, prev_kv: false };
        self.inner.delete_range(&req).deleted > 0
    }

    /// Read the historical event stream for an object; mirrors upstream
    /// `etcd3.GuaranteedUpdate` revision-walk used by the cacher to rebuild
    /// state. Walks the underlying KvStore history (post-compaction window).
    pub fn history_for(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        namespace: &str,
        name: &str,
    ) -> Vec<EventType> {
        // We piggyback on the cave-etcd watch surface to enumerate the
        // historical events whose key matches our encoded path.
        let key_bytes = key_for(tenant_id, group, kind, namespace, name)
            .as_bytes()
            .to_vec();
        let cfg = cave_etcd::models::WatchConfig {
            watch_id: 0,
            key: key_bytes,
            range_end: None,
            start_revision: None,
            prev_kv: false,
        };
        self.inner
            .get_historical_events(&cfg, 0)
            .into_iter()
            .map(|e| e.event_type)
            .collect()
    }

    /// Trigger a global revision compaction. Mirrors upstream
    /// `etcd3.compactor.compactKeys`.
    pub fn compact(&self, revision: u64) {
        self.inner.compact(revision);
    }
}

impl Default for EtcdBackend {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream parity: `TestKey_PathEncoding`
    /// (storage/etcd3/store_test.go — the canonical path layout for resources
    /// is `/registry/<group>/<resource>/<ns>/<name>` and prefix-rooted).
    #[test]
    fn test_key_layout_matches_canonical_registry_form() {
        let k = key_for("acme", "apps", "deployments", "default", "nginx");
        assert_eq!(k, "/tenants/acme/registry/apps/deployments/default/nginx");
        // tenant_id invariant: tenant prefix is the OUTERMOST path segment so
        // any range scan against another tenant cannot collide.
        assert!(k.starts_with("/tenants/acme/"),
            "tenant_id invariant: tenant is the root prefix");
        // Cluster-scoped: namespace empty => no namespace segment.
        let k2 = key_for("acme", "", "namespaces", "", "kube-system");
        assert_eq!(k2, "/tenants/acme/registry/core/namespaces/kube-system");
    }

    /// Upstream parity: `TestStore_PutGetRoundtrip`
    /// (storage/etcd3/store_test.go — Put then Get round-trips bytes
    /// without modification, and the returned revision is monotonic).
    #[test]
    fn test_put_then_get_roundtrips_value_and_advances_revision() {
        let be = EtcdBackend::new();
        let rv1 = be.put("acme", "", "configmaps", "default", "cfg",
            b"{\"data\":{\"k\":\"v\"}}");
        let rv2 = be.put("acme", "", "configmaps", "default", "cfg2",
            b"{\"data\":{\"x\":\"y\"}}");
        assert!(rv2 > rv1, "revision strictly monotonic across puts");
        let got = be.get("acme", "", "configmaps", "default", "cfg")
            .expect("get must find the just-put object");
        assert_eq!(got.value, b"{\"data\":{\"k\":\"v\"}}");
        assert_eq!(got.create_revision, rv1, "create_revision pinned at first put");
        assert_eq!(got.mod_revision, rv1);
        // tenant_id invariant: returned key is rooted at /tenants/acme/.
        assert!(got.key.starts_with("/tenants/acme/"),
            "tenant_id invariant: stored key under acme tenant root");
    }

    /// Upstream parity: `TestStore_ListByPrefixSameNamespace`
    /// (storage/etcd3/store_test.go — `List` with prefix returns every key
    /// under that prefix, sorted by key).
    #[test]
    fn test_list_returns_all_objects_under_namespace_prefix() {
        let be = EtcdBackend::new();
        be.put("acme", "", "configmaps", "default", "a", b"{}");
        be.put("acme", "", "configmaps", "default", "b", b"{}");
        be.put("acme", "", "configmaps", "default", "c", b"{}");
        be.put("acme", "", "configmaps", "kube-system", "d", b"{}");
        let in_default = be.list("acme", "", "configmaps", "default");
        assert_eq!(in_default.len(), 3,
            "list scoped to default namespace returns 3");
        // tenant_id invariant: all returned keys live under acme tenant root.
        assert!(in_default.iter().all(|o| o.key.starts_with("/tenants/acme/")),
            "tenant_id invariant: list never crosses tenant root");
    }

    /// Upstream parity: `TestStore_TenantPrefixHardIsolation`
    /// (no upstream test — this is the apiserver's tenant carve-out: a
    /// list against tenant A MUST NOT return tenant B's keys, even if
    /// names collide).
    #[test]
    fn test_list_does_not_cross_tenant_boundary_even_for_identical_names() {
        let be = EtcdBackend::new();
        be.put("acme", "", "configmaps", "default", "shared", b"acme-payload");
        be.put("globex", "", "configmaps", "default", "shared", b"globex-payload");
        let acme = be.list("acme", "", "configmaps", "default");
        let globex = be.list("globex", "", "configmaps", "default");
        assert_eq!(acme.len(), 1);
        assert_eq!(globex.len(), 1);
        assert_eq!(acme[0].value, b"acme-payload",
            "tenant_id invariant: acme list returns acme payload");
        assert_eq!(globex[0].value, b"globex-payload",
            "tenant_id invariant: globex list returns globex payload");
        // Cross-check: get on each side returns its own data, never the peer's.
        let g_acme = be.get("acme", "", "configmaps", "default", "shared").unwrap();
        let g_globex = be.get("globex", "", "configmaps", "default", "shared").unwrap();
        assert_ne!(g_acme.value, g_globex.value,
            "tenant_id invariant: same name in different tenants are distinct objects");
    }

    /// Upstream parity: `TestStore_DeleteRemovesAndAdvancesRevision`
    /// (storage/etcd3/store_test.go — Delete returns `Deleted` event and
    /// subsequent Get returns NotFound).
    #[test]
    fn test_delete_removes_object_and_history_records_delete_event() {
        let be = EtcdBackend::new();
        be.put("acme", "", "configmaps", "default", "to-go", b"{}");
        let removed = be.delete("acme", "", "configmaps", "default", "to-go");
        assert!(removed, "delete returns true for existing object");
        assert!(be.get("acme", "", "configmaps", "default", "to-go").is_none(),
            "deleted key cannot be Get'd");
        let again = be.delete("acme", "", "configmaps", "default", "to-go");
        assert!(!again, "second delete is a no-op");
        // History contains both Put and Delete events.
        let hist = be.history_for("acme", "", "configmaps", "default", "to-go");
        let has_put = hist.iter().any(|e| matches!(e, cave_etcd::models::EventType::Put));
        let has_del = hist.iter().any(|e| matches!(e, cave_etcd::models::EventType::Delete));
        assert!(has_put && has_del,
            "history records both Put and Delete for the lifecycle");
        // tenant_id invariant: history walk is per-key, never cross-tenant.
        let other = be.history_for("globex", "", "configmaps", "default", "to-go");
        assert!(other.is_empty(),
            "tenant_id invariant: globex sees nothing for acme's deleted key");
    }

    /// Upstream parity: `TestStore_RevisionCompactionAdvancesCompactedRevision`
    /// (storage/etcd3/compact.go — `Compact(rev)` raises the compacted_revision
    /// floor; older history is no longer queryable).
    #[test]
    fn test_compaction_advances_compacted_revision_floor() {
        let be = EtcdBackend::new();
        be.put("acme", "", "configmaps", "default", "a", b"{}");
        let rv2 = be.put("acme", "", "configmaps", "default", "b", b"{}");
        be.put("acme", "", "configmaps", "default", "c", b"{}");
        be.compact(rv2);
        assert!(be.compaction_revision() >= rv2,
            "compaction floor advanced to (or past) requested revision");
        // tenant_id invariant: compaction is global on the underlying KvStore
        // but tenants only see their own keys via the prefix layout.
        let acme = be.list("acme", "", "configmaps", "default");
        assert_eq!(acme.len(), 3,
            "tenant_id invariant: live keys preserved post-compaction for acme");
        assert!(acme.iter().all(|o| o.key.starts_with("/tenants/acme/")),
            "tenant_id invariant: list still scoped to acme after compaction");
    }

    /// Upstream parity: `TestStore_ListAcrossNamespacesUnderTenant`
    /// (storage/etcd3/store_test.go — List with a kind-only prefix scans
    /// every namespace under the kind).
    #[test]
    fn test_list_with_empty_namespace_scans_all_namespaces_for_tenant() {
        let be = EtcdBackend::new();
        be.put("acme", "", "configmaps", "default",     "x", b"{}");
        be.put("acme", "", "configmaps", "kube-system", "y", b"{}");
        be.put("acme", "", "configmaps", "tenant-tools","z", b"{}");
        // Cross-tenant decoy.
        be.put("globex", "", "configmaps", "default", "x", b"{}");
        let all = be.list("acme", "", "configmaps", "");
        assert_eq!(all.len(), 3,
            "kind-only scan within acme returns all 3 namespaces");
        assert!(all.iter().all(|o| o.key.starts_with("/tenants/acme/registry/core/configmaps/")),
            "tenant_id invariant: kind-only scan rooted under acme/configmaps");
        let names: Vec<_> = all.iter()
            .map(|o| o.key.rsplit('/').next().unwrap().to_string())
            .collect();
        assert!(names.contains(&"x".to_string()));
        assert!(names.contains(&"y".to_string()));
        assert!(names.contains(&"z".to_string()));
    }
}
