//! Watch cache with bookmark events.
//!
//! Upstream: kubernetes/kubernetes v1.30.0
//!   * `staging/src/k8s.io/apiserver/pkg/storage/cacher/cacher.go` (`watchCache`)
//!   * `staging/src/k8s.io/apiserver/pkg/storage/cacher/watch_cache.go`
//!   * KEP-365 (Watch bookmarks).
//!
//! Bookmarks are periodic events emitted by the apiserver on long-running
//! watches so clients can resume from a known resourceVersion without a full
//! relist. Per upstream contract, bookmarks carry only `metadata.resourceVersion`.
//!
//! Tenant invariant: each bookmark carries the tenant_id of the watcher's
//! scoped channel — bookmarks for tenant A's watch MUST NOT leak across to
//! tenant B's channel.

use crate::resources::Resource;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub enum WatchCacheEvent {
    Added {
        resource_version: u64,
        tenant_id: String,
        resource: Resource,
    },
    Modified {
        resource_version: u64,
        tenant_id: String,
        resource: Resource,
    },
    Deleted {
        resource_version: u64,
        tenant_id: String,
        resource: Resource,
    },
    /// Bookmark — synthetic event carrying current resourceVersion only.
    /// Upstream: `apimachinery/pkg/watch.Bookmark`.
    Bookmark {
        resource_version: u64,
        tenant_id: String,
    },
}

impl WatchCacheEvent {
    pub fn resource_version(&self) -> u64 {
        match self {
            WatchCacheEvent::Added { resource_version, .. }
            | WatchCacheEvent::Modified { resource_version, .. }
            | WatchCacheEvent::Deleted { resource_version, .. }
            | WatchCacheEvent::Bookmark { resource_version, .. } => *resource_version,
        }
    }

    pub fn tenant_id(&self) -> &str {
        match self {
            WatchCacheEvent::Added { tenant_id, .. }
            | WatchCacheEvent::Modified { tenant_id, .. }
            | WatchCacheEvent::Deleted { tenant_id, .. }
            | WatchCacheEvent::Bookmark { tenant_id, .. } => tenant_id,
        }
    }

    pub fn is_bookmark(&self) -> bool {
        matches!(self, WatchCacheEvent::Bookmark { .. })
    }
}

/// Tenant-scoped, bounded ring buffer of watch events with explicit
/// bookmark emission. Mirrors upstream `watchCache.processEvent` +
/// bookmark interval logic without timer dependency (caller drives it).
pub struct WatchCache {
    capacity: usize,
    /// Emit a bookmark every N non-bookmark events.
    bookmark_interval: u32,
    inner: Mutex<WatchCacheInner>,
    rv: AtomicU64,
}

struct WatchCacheInner {
    events: VecDeque<WatchCacheEvent>,
    events_since_bookmark: u32,
}

impl WatchCache {
    pub fn new(capacity: usize, bookmark_interval: u32) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        assert!(bookmark_interval > 0, "bookmark_interval must be > 0");
        Self {
            capacity,
            bookmark_interval,
            inner: Mutex::new(WatchCacheInner {
                events: VecDeque::with_capacity(capacity),
                events_since_bookmark: 0,
            }),
            rv: AtomicU64::new(0),
        }
    }

    pub fn current_resource_version(&self) -> u64 {
        self.rv.load(Ordering::SeqCst)
    }

    fn next_rv(&self) -> u64 {
        self.rv.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn record_added(&self, tenant_id: &str, resource: Resource) -> u64 {
        let rv = self.next_rv();
        self.push(WatchCacheEvent::Added {
            resource_version: rv, tenant_id: tenant_id.into(), resource,
        });
        rv
    }

    pub fn record_modified(&self, tenant_id: &str, resource: Resource) -> u64 {
        let rv = self.next_rv();
        self.push(WatchCacheEvent::Modified {
            resource_version: rv, tenant_id: tenant_id.into(), resource,
        });
        rv
    }

    pub fn record_deleted(&self, tenant_id: &str, resource: Resource) -> u64 {
        let rv = self.next_rv();
        self.push(WatchCacheEvent::Deleted {
            resource_version: rv, tenant_id: tenant_id.into(), resource,
        });
        rv
    }

    /// Force-emit a bookmark for `tenant_id` at current rv. Used by the
    /// long-poll loop on heartbeat ticks. Mirrors `cacher.processBookmarkEvent`.
    pub fn force_bookmark(&self, tenant_id: &str) -> u64 {
        let rv = self.rv.load(Ordering::SeqCst);
        let mut inner = self.inner.lock().unwrap();
        inner.events.push_back(WatchCacheEvent::Bookmark {
            resource_version: rv, tenant_id: tenant_id.into(),
        });
        if inner.events.len() > self.capacity {
            inner.events.pop_front();
        }
        inner.events_since_bookmark = 0;
        rv
    }

    fn push(&self, event: WatchCacheEvent) {
        let mut inner = self.inner.lock().unwrap();
        let tenant = event.tenant_id().to_string();
        let rv = event.resource_version();
        inner.events.push_back(event);
        if inner.events.len() > self.capacity {
            inner.events.pop_front();
        }
        inner.events_since_bookmark += 1;
        if inner.events_since_bookmark >= self.bookmark_interval {
            inner.events.push_back(WatchCacheEvent::Bookmark {
                resource_version: rv, tenant_id: tenant,
            });
            if inner.events.len() > self.capacity {
                inner.events.pop_front();
            }
            inner.events_since_bookmark = 0;
        }
    }

    /// Replay events strictly newer than `since_rv`, scoped to `tenant_id`.
    /// Mirrors `cacher.GetEvents(rv)` + per-tenant filter.
    pub fn replay_for_tenant(&self, tenant_id: &str, since_rv: u64) -> Vec<WatchCacheEvent> {
        let inner = self.inner.lock().unwrap();
        inner.events.iter()
            .filter(|e| e.resource_version() > since_rv && e.tenant_id() == tenant_id)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().events.len()
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::{ConfigMap, ObjectMeta};
    use std::collections::HashMap;

    fn cm(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns), data: HashMap::new(),
        })
    }

    /// Upstream parity: `TestWatchCache_RecordAdded` (storage/cacher/watch_cache_test.go).
    #[test]
    fn test_record_added_increments_rv() {
        let wc = WatchCache::new(64, 100);
        let rv1 = wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        assert!(rv2 > rv1);
        assert_eq!(wc.current_resource_version(), rv2);
        let evs = wc.replay_for_tenant("acme", 0);
        assert_eq!(evs.len(), 2);
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: replay returns only matching tenant events");
    }

    /// Upstream parity: `TestWatchCache_BookmarkInterval` (KEP-365).
    #[test]
    fn test_bookmark_emitted_on_interval() {
        let wc = WatchCache::new(64, 3);
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("acme", cm("b", "default"));
        // 3rd event triggers bookmark emission.
        wc.record_added("acme", cm("c", "default"));
        let evs = wc.replay_for_tenant("acme", 0);
        let bookmarks: Vec<_> = evs.iter().filter(|e| e.is_bookmark()).collect();
        assert_eq!(bookmarks.len(), 1, "exactly one bookmark after interval");
        assert_eq!(bookmarks[0].tenant_id(), "acme",
            "tenant_id invariant: bookmark carries tenant_id");
    }

    /// Upstream parity: `TestWatchCache_BookmarkResourceVersionMatchesLatest`.
    #[test]
    fn test_bookmark_carries_current_rv() {
        let wc = WatchCache::new(64, 2);
        let _ = wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let evs = wc.replay_for_tenant("acme", 0);
        let bm = evs.iter().find(|e| e.is_bookmark()).expect("bookmark present");
        assert_eq!(bm.resource_version(), rv2,
            "bookmark rv equals latest event rv");
        assert_eq!(bm.tenant_id(), "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestWatchCache_TenantIsolation`.
    #[test]
    fn test_replay_isolates_tenants() {
        let wc = WatchCache::new(64, 100);
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("globex", cm("b", "default"));
        wc.record_added("acme", cm("c", "default"));
        let acme = wc.replay_for_tenant("acme", 0);
        let globex = wc.replay_for_tenant("globex", 0);
        assert_eq!(acme.len(), 2);
        assert_eq!(globex.len(), 1);
        assert!(acme.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: no cross-tenant leak");
        assert!(globex.iter().all(|e| e.tenant_id() == "globex"),
            "tenant_id invariant: no cross-tenant leak");
    }

    /// Upstream parity: `TestWatchCache_ForceBookmarkOnHeartbeat`.
    #[test]
    fn test_force_bookmark_emits_immediately() {
        let wc = WatchCache::new(64, 1000); // interval high → only force triggers
        wc.record_added("acme", cm("a", "default"));
        let rv = wc.force_bookmark("acme");
        let evs = wc.replay_for_tenant("acme", 0);
        let bookmarks: Vec<_> = evs.iter().filter(|e| e.is_bookmark()).collect();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].resource_version(), rv);
        assert_eq!(bookmarks[0].tenant_id(), "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestWatchCache_SinceRvFilter`.
    #[test]
    fn test_replay_since_rv_skips_old_events() {
        let wc = WatchCache::new(64, 1000);
        let rv1 = wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let after_rv1 = wc.replay_for_tenant("acme", rv1);
        assert_eq!(after_rv1.len(), 1);
        assert_eq!(after_rv1[0].resource_version(), rv2);
        assert_eq!(after_rv1[0].tenant_id(), "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestWatchCache_RingBufferEvictsOldest`.
    #[test]
    fn test_ring_buffer_evicts_oldest_when_full() {
        let wc = WatchCache::new(2, 1000);
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("acme", cm("b", "default"));
        wc.record_added("acme", cm("c", "default"));
        assert_eq!(wc.len(), 2, "capacity bounded");
        let evs = wc.replay_for_tenant("acme", 0);
        // Oldest evicted, only 'b' and 'c' remain.
        let names: Vec<_> = evs.iter().filter_map(|e| match e {
            WatchCacheEvent::Added { resource, .. } => Some(resource.name().to_string()),
            _ => None,
        }).collect();
        assert!(!names.contains(&"a".to_string()));
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"), "tenant_id invariant");
    }

    /// Upstream parity: `TestWatchCache_ModifyAndDelete`.
    #[test]
    fn test_modified_and_deleted_recorded() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        wc.record_modified("acme", cm("a", "default"));
        wc.record_deleted("acme", cm("a", "default"));
        let evs = wc.replay_for_tenant("acme", 0);
        assert!(matches!(evs[0], WatchCacheEvent::Added { .. }));
        assert!(matches!(evs[1], WatchCacheEvent::Modified { .. }));
        assert!(matches!(evs[2], WatchCacheEvent::Deleted { .. }));
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"), "tenant_id invariant");
    }
}
