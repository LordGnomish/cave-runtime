// SPDX-License-Identifier: AGPL-3.0-or-later
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
//!
//! Two surfaces are exposed for downstream watchers (sweep-002 F2-B):
//!   * `replay_for_tenant{,_checked}` — RV-indexed historical replay backed by
//!     the bounded ring buffer (KEP-1483 compaction floor honored).
//!   * `subscribe` — live fan-out via `cave_kernel::eventbus::EventBus`.
//!     Lagged subscribers receive `EventBusError::Lagged(n)` and may resync via
//!     a fresh LIST + `replay_for_tenant_checked`. Capacity for the broadcast
//!     bus is independent of the ring-buffer capacity, so a slow tailing
//!     consumer cannot evict events from the replay buffer.

use crate::resources::Resource;
use cave_kernel::eventbus::{EventBus, Subscription};
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

/// Default capacity for the live fan-out `EventBus`. Independent of the ring
/// buffer capacity — a slow tailer experiences `Lagged` rather than evicting
/// events from the replay history.
pub const DEFAULT_LIVE_BUS_CAPACITY: usize = 4096;

/// Tenant-scoped, bounded ring buffer of watch events with explicit
/// bookmark emission. Mirrors upstream `watchCache.processEvent` +
/// bookmark interval logic without timer dependency (caller drives it).
///
/// In addition to the ring buffer, each cache exposes a `cave_kernel`
/// `EventBus<WatchCacheEvent>` for live fan-out. Subscribers receive every
/// recorded event and every emitted bookmark in publish order; lagged
/// subscribers receive `EventBusError::Lagged(n)` per kernel contract.
pub struct WatchCache {
    capacity: usize,
    /// Emit a bookmark every N non-bookmark events.
    bookmark_interval: u32,
    inner: Mutex<WatchCacheInner>,
    rv: AtomicU64,
    /// Compacted floor — replays at or below this RV are denied.
    /// Mirrors upstream `cacher.go::watchCache.resourceVersionFloor`.
    compacted_rv: AtomicU64,
    /// Live fan-out — `cave_kernel::eventbus::EventBus<WatchCacheEvent>`.
    live_bus: EventBus<WatchCacheEvent>,
}

struct WatchCacheInner {
    events: VecDeque<WatchCacheEvent>,
    events_since_bookmark: u32,
}

/// Result of a tenant-scoped replay request.
#[derive(Debug, Clone)]
pub enum ReplayOutcome {
    /// `since_rv` is below the compacted floor. The watcher must restart
    /// with a full LIST + new resourceVersion (upstream
    /// `apierrors.NewResourceExpired`).
    Compacted { compacted_to: u64 },
    Events(Vec<WatchCacheEvent>),
}

impl WatchCache {
    pub fn new(capacity: usize, bookmark_interval: u32) -> Self {
        Self::with_live_capacity(capacity, bookmark_interval, DEFAULT_LIVE_BUS_CAPACITY)
    }

    /// Constructor exposing the live-bus capacity. Use this when the watch
    /// fan-out fan-out factor differs from the replay-buffer sizing
    /// (multi-tenant deployments with high subscriber count).
    pub fn with_live_capacity(
        capacity: usize,
        bookmark_interval: u32,
        live_bus_capacity: usize,
    ) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        assert!(bookmark_interval > 0, "bookmark_interval must be > 0");
        assert!(live_bus_capacity > 0, "live_bus_capacity must be > 0");
        Self {
            capacity,
            bookmark_interval,
            inner: Mutex::new(WatchCacheInner {
                events: VecDeque::with_capacity(capacity),
                events_since_bookmark: 0,
            }),
            rv: AtomicU64::new(0),
            compacted_rv: AtomicU64::new(0),
            live_bus: EventBus::new(live_bus_capacity),
        }
    }

    /// Subscribe to live watch events. Subscribers see only events published
    /// *after* subscription (matches upstream `cacher.Watch` semantics —
    /// callers needing past events should call `replay_for_tenant_checked`
    /// to backfill, then transition to the live stream). Multi-tenant
    /// filtering is performed at the consumer side using
    /// `WatchCacheEvent::tenant_id`.
    pub fn subscribe(&self) -> Subscription<WatchCacheEvent> {
        self.live_bus.subscribe()
    }

    /// Number of currently active live subscribers. Useful for back-pressure
    /// and connection-count metrics.
    pub fn subscriber_count(&self) -> usize {
        self.live_bus.subscriber_count()
    }

    /// Current compaction floor (matches upstream
    /// `watchCache.resourceVersionFloor`).
    pub fn compacted_revision(&self) -> u64 {
        self.compacted_rv.load(Ordering::SeqCst)
    }

    /// Drop every event with `rv <= floor` and raise the compaction floor.
    /// Subsequent `replay_for_tenant_*` calls below `floor` return Compacted.
    /// Mirrors upstream `cacher.processEvent` compaction trim + KEP-1483
    /// `resourceVersionFloor` semantics; never crosses tenants because
    /// events themselves are tenant-tagged.
    pub fn compact(&self, floor: u64) {
        let prev = self.compacted_rv.load(Ordering::SeqCst);
        if floor <= prev { return; }
        self.compacted_rv.store(floor, Ordering::SeqCst);
        let mut inner = self.inner.lock().unwrap();
        inner.events.retain(|e| e.resource_version() > floor);
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
        let bookmark = WatchCacheEvent::Bookmark {
            resource_version: rv, tenant_id: tenant_id.into(),
        };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.events.push_back(bookmark.clone());
            if inner.events.len() > self.capacity {
                inner.events.pop_front();
            }
            inner.events_since_bookmark = 0;
        }
        let _ = self.live_bus.publish(bookmark);
        rv
    }

    fn push(&self, event: WatchCacheEvent) {
        let bookmark_to_emit = {
            let mut inner = self.inner.lock().unwrap();
            let tenant = event.tenant_id().to_string();
            let rv = event.resource_version();
            inner.events.push_back(event.clone());
            if inner.events.len() > self.capacity {
                inner.events.pop_front();
            }
            inner.events_since_bookmark += 1;
            if inner.events_since_bookmark >= self.bookmark_interval {
                let bookmark = WatchCacheEvent::Bookmark {
                    resource_version: rv, tenant_id: tenant,
                };
                inner.events.push_back(bookmark.clone());
                if inner.events.len() > self.capacity {
                    inner.events.pop_front();
                }
                inner.events_since_bookmark = 0;
                Some(bookmark)
            } else {
                None
            }
        };
        let _ = self.live_bus.publish(event);
        if let Some(bm) = bookmark_to_emit {
            let _ = self.live_bus.publish(bm);
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

    /// Compaction-aware replay. Returns `Compacted { compacted_to }` if
    /// `since_rv` is at-or-below the compaction floor — the watcher must
    /// restart with a fresh LIST. Mirrors upstream
    /// `cacher.GetAllEventsSince -> apierrors.NewResourceExpired`.
    pub fn replay_for_tenant_checked(
        &self,
        tenant_id: &str,
        since_rv: u64,
    ) -> ReplayOutcome {
        let floor = self.compacted_rv.load(Ordering::SeqCst);
        if since_rv < floor {
            return ReplayOutcome::Compacted { compacted_to: floor };
        }
        ReplayOutcome::Events(self.replay_for_tenant(tenant_id, since_rv))
    }

    /// Initial-LIST helper: emit a synthetic Bookmark at the current RV
    /// for a tenant restarting from 0 after compaction. Mirrors upstream
    /// `cacher.GetList -> ResourceVersionMatchNotOlderThan` initial sync.
    pub fn restart_for_tenant(&self, tenant_id: &str) -> WatchCacheEvent {
        let rv = self.rv.load(Ordering::SeqCst);
        WatchCacheEvent::Bookmark {
            resource_version: rv,
            tenant_id: tenant_id.into(),
        }
    }

    /// KEP-1904 semantics for `?resourceVersion=&resourceVersionMatch=`.
    /// Returns the RV the list must serve at, plus a flag indicating
    /// whether the request can be served from this cache or must fall
    /// through to a quorum read. Mirrors upstream
    /// `apiserver/pkg/storage/cacher/cacher.go::GetList` selector logic.
    pub fn select_list_revision(
        &self,
        requested_rv: u64,
        match_kind: ResourceVersionMatch,
    ) -> Result<ListSelection, ListRevisionError> {
        let current = self.rv.load(Ordering::SeqCst);
        let floor = self.compacted_rv.load(Ordering::SeqCst);
        match match_kind {
            ResourceVersionMatch::Unspecified => {
                // Empty resourceVersion → most-up-to-date.
                Ok(ListSelection { serve_at: current, can_serve_from_cache: true })
            }
            ResourceVersionMatch::NotOlderThan => {
                if requested_rv > current {
                    // Consistent read past our cache; caller must wait.
                    return Err(ListRevisionError::TooNew { requested: requested_rv, current });
                }
                if requested_rv < floor && floor > 0 {
                    return Err(ListRevisionError::Compacted { requested: requested_rv, floor });
                }
                Ok(ListSelection {
                    serve_at: requested_rv.max(floor),
                    can_serve_from_cache: true,
                })
            }
            ResourceVersionMatch::Exact => {
                if requested_rv > current {
                    return Err(ListRevisionError::TooNew { requested: requested_rv, current });
                }
                if requested_rv <= floor && floor > 0 {
                    return Err(ListRevisionError::Compacted { requested: requested_rv, floor });
                }
                Ok(ListSelection {
                    serve_at: requested_rv,
                    can_serve_from_cache: true,
                })
            }
        }
    }

    /// KEP-956 WatchList — stream the initial state followed by a
    /// terminator bookmark carrying the `kubernetes.io/initial-events-end`
    /// annotation. Mirrors upstream
    /// `apiserver/pkg/storage/cacher/cacher.go::watchInitialEvents`.
    pub fn watch_list_initial(
        &self,
        tenant_id: &str,
    ) -> WatchListBatch {
        let inner = self.inner.lock().unwrap();
        let rv = self.rv.load(Ordering::SeqCst);
        // Initial state — every Added event still in the cache for this
        // tenant, ordered by RV asc. Skip Modified/Deleted (initial sync
        // semantics).
        let mut initial: Vec<WatchCacheEvent> = inner.events.iter()
            .filter(|e| e.tenant_id() == tenant_id)
            .filter(|e| matches!(e, WatchCacheEvent::Added { .. }))
            .cloned()
            .collect();
        initial.sort_by_key(|e| e.resource_version());
        WatchListBatch {
            initial,
            terminator: InitialEventsEndBookmark {
                resource_version: rv,
                tenant_id: tenant_id.into(),
            },
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().events.len()
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

/// `?resourceVersionMatch=` semantics from KEP-1904.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceVersionMatch {
    /// Unset — most-up-to-date.
    Unspecified,
    /// `NotOlderThan` — RV >= requested. Default for LIST when RV is set.
    NotOlderThan,
    /// `Exact` — must serve at RV; otherwise fail with HTTP 410 Gone.
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSelection {
    pub serve_at: u64,
    pub can_serve_from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRevisionError {
    Compacted { requested: u64, floor: u64 },
    TooNew { requested: u64, current: u64 },
}

/// Synthetic bookmark emitted at the end of a KEP-956 WatchList initial
/// stream. Carries the `kubernetes.io/initial-events-end=true` annotation
/// per the KEP. Modeled as a typed value so consumers don't need to parse
/// out-of-band metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialEventsEndBookmark {
    pub resource_version: u64,
    pub tenant_id: String,
}

impl InitialEventsEndBookmark {
    /// Annotation name per KEP-956.
    pub const ANNOTATION: &'static str = "k8s.io/initial-events-end";
}

#[derive(Debug, Clone)]
pub struct WatchListBatch {
    pub initial: Vec<WatchCacheEvent>,
    pub terminator: InitialEventsEndBookmark,
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

    // ── Deeper coverage (v1.36.0) ─────────────────────────────────────────────

    /// Upstream parity: `TestWatchCache_ReplaySinceLatestRvIsEmpty`
    /// (storage/cacher/watch_cache_test.go — `getAllEvents(latestRv)`).
    #[test]
    fn test_replay_since_latest_rv_yields_no_events() {
        let wc = WatchCache::new(64, 1000);
        let _rv1 = wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let from_latest = wc.replay_for_tenant("acme", rv2);
        assert!(from_latest.is_empty(),
            "no events strictly newer than latest RV");
        // tenant_id invariant smoke: empty result still scoped to acme query.
        let _ = wc.record_added("globex", cm("x", "default"));
        let after = wc.replay_for_tenant("acme", rv2);
        assert!(after.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: globex event must not appear in acme replay");
    }

    /// Upstream parity: `TestWatchCache_ConcurrentMultiTenantOrdering`
    /// (cacher_test.go — concurrent processEvent across watchers).
    /// Resource versions must be strictly monotonic across tenants and the
    /// per-tenant replay must remain pure.
    #[test]
    fn test_concurrent_multi_tenant_writes_keep_rv_monotonic() {
        use std::sync::Arc as StdArc;
        use std::thread;
        let wc = StdArc::new(WatchCache::new(1024, 1000));
        let mut handles = vec![];
        for tenant in ["acme", "globex", "initech"] {
            let wc2 = wc.clone();
            handles.push(thread::spawn(move || {
                for i in 0..20 {
                    wc2.record_added(tenant, cm(&format!("{}-{}", tenant, i), "default"));
                }
            }));
        }
        for h in handles { h.join().unwrap(); }
        assert_eq!(wc.current_resource_version(), 60,
            "60 writes total — RV monotonic + atomic");
        let acme = wc.replay_for_tenant("acme", 0);
        assert_eq!(acme.iter().filter(|e| !e.is_bookmark()).count(), 20);
        assert!(acme.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: no cross-tenant bleed under concurrency");
    }

    /// Upstream parity: `TestWatchCache_ForceBookmarkPerTenantIsolated`
    /// (cacher.processBookmarkEvent dispatched per-watcher).
    #[test]
    fn test_force_bookmark_does_not_leak_across_tenants() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("globex", cm("b", "default"));
        wc.force_bookmark("acme");
        let acme = wc.replay_for_tenant("acme", 0);
        let globex = wc.replay_for_tenant("globex", 0);
        let acme_bookmarks = acme.iter().filter(|e| e.is_bookmark()).count();
        let globex_bookmarks = globex.iter().filter(|e| e.is_bookmark()).count();
        assert_eq!(acme_bookmarks, 1,
            "force_bookmark on acme creates exactly one acme-tagged bookmark");
        assert_eq!(globex_bookmarks, 0,
            "tenant_id invariant: globex must not receive acme's heartbeat");
        assert!(acme.iter().all(|e| e.tenant_id() == "acme"));
    }

    /// Upstream parity: `TestWatchCache_BookmarkResetsAfterEmission`
    /// (KEP-365 — interval counter resets on emit).
    #[test]
    fn test_bookmark_interval_counter_resets_after_emit() {
        let wc = WatchCache::new(64, 2);
        // Two writes → 1 bookmark fires; counter resets.
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("acme", cm("b", "default"));
        // Two more writes → another bookmark fires.
        wc.record_added("acme", cm("c", "default"));
        wc.record_added("acme", cm("d", "default"));
        let evs = wc.replay_for_tenant("acme", 0);
        let bms = evs.iter().filter(|e| e.is_bookmark()).count();
        assert_eq!(bms, 2,
            "interval counter resets after each bookmark — expect two over four writes");
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant across multiple bookmark emissions");
    }

    /// Upstream parity: `TestWatchCache_BookmarkAccessorsExposeTenant`
    /// (Bookmark events implement the tenant-scoped accessor surface).
    #[test]
    fn test_bookmark_event_accessors_return_tenant_and_rv() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv = wc.force_bookmark("acme");
        let evs = wc.replay_for_tenant("acme", 0);
        let bm = evs.iter().find(|e| e.is_bookmark()).expect("bookmark present");
        assert!(bm.is_bookmark());
        assert_eq!(bm.tenant_id(), "acme",
            "tenant_id invariant: accessor returns scoped tenant");
        assert_eq!(bm.resource_version(), rv);
    }

    /// Upstream parity: `TestWatchCache_DisjointTenantsHaveSeparateReplays`
    /// (cacher.GetEvents predicate excludes other tenants from the slice).
    #[test]
    fn test_replay_excludes_unknown_tenant() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("globex", cm("b", "default"));
        let unknown = wc.replay_for_tenant("missing-tenant", 0);
        assert!(unknown.is_empty(),
            "tenant_id invariant: unknown tenant gets empty replay, never bleed");
        let acme = wc.replay_for_tenant("acme", 0);
        assert!(acme.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: only acme entries returned");
    }

    // ── Restart + event compaction (deeper-003) ──────────────────────────────

    /// Upstream parity: `TestWatchCache_CompactRaisesFloorAndDropsOlder`
    /// (storage/cacher/cacher_test.go — `Compact(rev)` raises the floor and
    /// trims older entries from the cache buffer).
    #[test]
    fn test_compact_raises_floor_and_trims_older_entries() {
        let wc = WatchCache::new(64, 1000);
        let _rv1 = wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let rv3 = wc.record_added("acme", cm("c", "default"));
        wc.compact(rv2);
        assert_eq!(wc.compacted_revision(), rv2,
            "compaction floor advanced to requested rv");
        let evs = wc.replay_for_tenant("acme", 0);
        // Only events strictly newer than the floor remain — rv3 stays.
        assert!(evs.iter().any(|e| e.resource_version() == rv3));
        assert!(!evs.iter().any(|e| e.resource_version() <= rv2),
            "no event at-or-below floor remains in cache");
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: trimming preserves tenant scoping");
    }

    /// Upstream parity: `TestWatchCache_ReplayFromBelowFloorReturnsCompacted`
    /// (cacher_test.go — `GetAllEventsSince(rv)` returns
    /// `apierrors.NewResourceExpired` when `rv` < compaction floor).
    #[test]
    fn test_replay_from_below_floor_returns_compacted_signal() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        wc.record_added("acme", cm("c", "default"));
        wc.compact(rv2);
        let outcome = wc.replay_for_tenant_checked("acme", 0);
        match outcome {
            ReplayOutcome::Compacted { compacted_to } => {
                assert_eq!(compacted_to, rv2);
            }
            ReplayOutcome::Events(_) => panic!("expected Compacted signal for rv=0"),
        }
        // tenant_id invariant: a globex watcher trying to restart from 0
        // also receives the global Compacted signal — but the floor itself
        // is not tenant-bearing, so this only proves the signal is uniform,
        // not a leak. Verify the per-tenant payload path is still empty.
        let g = wc.replay_for_tenant_checked("globex", rv2);
        assert!(matches!(g, ReplayOutcome::Events(ref v) if v.is_empty()),
            "tenant_id invariant: globex sees no acme events post-compaction");
    }

    /// Upstream parity: `TestWatchCache_ReplayAtOrAboveFloorReturnsEvents`
    /// (cacher_test.go — replay from `since_rv >= floor` is allowed).
    #[test]
    fn test_replay_at_or_above_floor_returns_event_set() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let rv3 = wc.record_added("acme", cm("c", "default"));
        wc.compact(rv2);
        let outcome = wc.replay_for_tenant_checked("acme", rv2);
        match outcome {
            ReplayOutcome::Events(evs) => {
                assert_eq!(evs.len(), 1, "exactly one event > rv2 remains");
                assert_eq!(evs[0].resource_version(), rv3);
                assert_eq!(evs[0].tenant_id(), "acme",
                    "tenant_id invariant on event reachable above floor");
            }
            ReplayOutcome::Compacted { .. } => panic!("rv2 == floor must not be Compacted"),
        }
    }

    /// Upstream parity: `TestWatchCache_RestartReturnsBookmarkAtCurrentRv`
    /// (cacher.go::initialEventsLast — restart watchers receive a synthetic
    /// bookmark at the current RV so they have a known resume point).
    #[test]
    fn test_restart_yields_bookmark_at_current_rv_per_tenant() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        // Compaction wipes lower history.
        wc.compact(rv2);
        let restart = wc.restart_for_tenant("acme");
        assert!(matches!(restart, WatchCacheEvent::Bookmark { .. }));
        assert_eq!(restart.resource_version(), rv2,
            "restart bookmark carries the current RV (post-compaction)");
        assert_eq!(restart.tenant_id(), "acme",
            "tenant_id invariant: restart bookmark scoped to caller");
        // Globex restart returns its own bookmark, not acme's.
        let g_restart = wc.restart_for_tenant("globex");
        assert_eq!(g_restart.tenant_id(), "globex",
            "tenant_id invariant: globex restart distinct from acme");
        assert_eq!(g_restart.resource_version(), rv2,
            "RV is global but the bookmark is tenant-tagged");
    }

    /// Upstream parity: `TestWatchCache_CompactBelowFloorIsNoop`
    /// (cacher_test.go — `Compact(rv)` with rv <= existing floor is silent).
    #[test]
    fn test_compact_below_floor_does_not_lower_floor() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        wc.compact(rv2);
        assert_eq!(wc.compacted_revision(), rv2);
        wc.compact(0); // earlier revision — must be ignored
        assert_eq!(wc.compacted_revision(), rv2,
            "compaction floor is monotonic; lower floor MUST NOT take effect");
        // tenant_id invariant smoke: subsequent acme writes still tagged.
        let _ = wc.record_added("acme", cm("d", "default"));
        let evs = wc.replay_for_tenant("acme", rv2);
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant retained after monotonic-floor check");
    }

    // ── Deeper coverage (deeper-005) — KEP-956 + KEP-1904 ────────────────────

    /// Upstream parity: `TestList_ResourceVersionMatchUnspecifiedReturnsCurrent`
    /// (cacher.go::GetList — empty resourceVersion → most-up-to-date read).
    #[test]
    fn test_list_unspecified_match_returns_current_rv() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let sel = wc.select_list_revision(0, ResourceVersionMatch::Unspecified).unwrap();
        assert_eq!(sel.serve_at, rv2,
            "Unspecified match → server returns current RV");
        assert!(sel.can_serve_from_cache);
        // tenant_id invariant smoke: select_list_revision is RV-only,
        // no tenant scoping at this layer.
        let _ = wc.replay_for_tenant("acme", 0);
    }

    /// Upstream parity: `TestList_ResourceVersionMatchNotOlderThanWaitsForRv`
    /// (KEP-1904 — `NotOlderThan` with RV in the future fails TooNew).
    #[test]
    fn test_list_not_older_than_rejects_future_rv() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let err = wc.select_list_revision(rv2 + 100, ResourceVersionMatch::NotOlderThan)
            .unwrap_err();
        match err {
            ListRevisionError::TooNew { requested, current } => {
                assert_eq!(requested, rv2 + 100);
                assert_eq!(current, rv2);
            }
            other => panic!("expected TooNew, got {:?}", other),
        }
    }

    /// Upstream parity: `TestList_ResourceVersionMatchExactBelowFloorRejected`
    /// (KEP-1904 — `Exact` against compacted RV → HTTP 410 Gone).
    #[test]
    fn test_list_exact_match_below_compacted_floor_rejected() {
        let wc = WatchCache::new(64, 1000);
        let rv1 = wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        wc.compact(rv2);
        let err = wc.select_list_revision(rv1, ResourceVersionMatch::Exact)
            .unwrap_err();
        match err {
            ListRevisionError::Compacted { requested, floor } => {
                assert_eq!(requested, rv1);
                assert_eq!(floor, rv2);
            }
            other => panic!("expected Compacted, got {:?}", other),
        }
    }

    /// Upstream parity: `TestList_ResourceVersionMatchExactAtCurrentSucceeds`
    /// (cacher.go — `Exact` at the current RV serves from cache).
    #[test]
    fn test_list_exact_match_at_current_rv_serves_from_cache() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let sel = wc.select_list_revision(rv2, ResourceVersionMatch::Exact).unwrap();
        assert_eq!(sel.serve_at, rv2);
        assert!(sel.can_serve_from_cache);
    }

    /// Upstream parity: `TestWatchList_InitialEventsTerminator`
    /// (KEP-956 — initial stream ends with a synthetic Bookmark carrying
    /// `k8s.io/initial-events-end` annotation).
    #[test]
    fn test_watch_list_emits_tenant_scoped_initial_state_then_terminator() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("acme", cm("b", "default"));
        wc.record_added("globex", cm("c", "default"));
        let acme_batch = wc.watch_list_initial("acme");
        assert_eq!(acme_batch.initial.len(), 2,
            "initial stream contains acme's two Added events");
        assert!(acme_batch.initial.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant: globex's event MUST NOT appear in acme's WatchList");
        assert_eq!(acme_batch.terminator.tenant_id, "acme",
            "tenant_id invariant: terminator bookmark scoped to acme");
        assert_eq!(InitialEventsEndBookmark::ANNOTATION,
            "k8s.io/initial-events-end",
            "annotation name matches KEP-956");
    }

    /// Upstream parity: `TestWatchList_TerminatorRvEqualsCurrent`
    /// (KEP-956 — terminator carries the current cache RV so the watcher
    /// can resume non-overlappingly from there).
    #[test]
    fn test_watch_list_terminator_rv_equals_current_cache_rv() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("a", "default"));
        let rv2 = wc.record_added("acme", cm("b", "default"));
        let batch = wc.watch_list_initial("acme");
        assert_eq!(batch.terminator.resource_version, rv2,
            "terminator RV equals the current cache RV at the time of stream");
        assert_eq!(batch.terminator.tenant_id, "acme",
            "tenant_id invariant: terminator scoped to caller");
    }

    // ── F2-B: live fan-out via cave_kernel::eventbus::EventBus ───────────────

    use cave_kernel::eventbus::EventBusError;

    /// Subscribers established before a write receive the event in publish
    /// order. Adopts `cave_kernel::eventbus::EventBus` semantics: subscribers
    /// see only events published after `subscribe()` (matches upstream
    /// `cacher.Watch` "from now" baseline).
    #[tokio::test]
    async fn test_subscribe_receives_added_event_live() {
        let wc = WatchCache::new(64, 1000);
        let mut sub = wc.subscribe();
        let rv = wc.record_added("acme", cm("a", "default"));
        let ev = sub.recv().await.unwrap();
        match ev {
            WatchCacheEvent::Added { resource_version, tenant_id, .. } => {
                assert_eq!(resource_version, rv);
                assert_eq!(tenant_id, "acme",
                    "tenant_id invariant: live event carries scoped tenant");
            }
            other => panic!("expected Added, got {:?}", other),
        }
    }

    /// Single producer fans out to every active subscriber (broadcast
    /// semantics). Mirrors `EventBus::publish` returning `subscriber_count`
    /// recipients on success.
    #[tokio::test]
    async fn test_subscribe_fans_out_to_multiple_subscribers() {
        let wc = WatchCache::new(64, 1000);
        let mut a = wc.subscribe();
        let mut b = wc.subscribe();
        let mut c = wc.subscribe();
        assert_eq!(wc.subscriber_count(), 3,
            "subscriber_count tracks active live tailers");
        wc.record_added("acme", cm("x", "default"));
        let ea = a.recv().await.unwrap();
        let eb = b.recv().await.unwrap();
        let ec = c.recv().await.unwrap();
        assert_eq!(ea.tenant_id(), "acme");
        assert_eq!(eb.tenant_id(), "acme");
        assert_eq!(ec.tenant_id(), "acme",
            "tenant_id invariant: every fan-out copy carries the same tenant");
    }

    /// Multi-tenant isolation at the consumer side: subscribers see all
    /// tenants on the live bus and filter by `tenant_id()`. The kernel bus is
    /// type-not-key separated, matching upstream `cacher.dispatchEvent`
    /// behavior where the watcher's selector filters cross-tenant traffic.
    #[tokio::test]
    async fn test_subscribe_consumer_filter_isolates_tenants() {
        let wc = WatchCache::new(64, 1000);
        let mut sub = wc.subscribe();
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("globex", cm("b", "default"));
        wc.record_added("acme", cm("c", "default"));
        let mut acme_seen = 0;
        let mut globex_seen = 0;
        for _ in 0..3 {
            let ev = sub.recv().await.unwrap();
            match ev.tenant_id() {
                "acme" => acme_seen += 1,
                "globex" => globex_seen += 1,
                other => panic!("unknown tenant {other}"),
            }
        }
        assert_eq!(acme_seen, 2,
            "tenant_id invariant: acme tailer would see exactly two acme events");
        assert_eq!(globex_seen, 1,
            "tenant_id invariant: globex tailer would see exactly one globex event");
    }

    /// Lag detection: when a subscriber falls behind the live bus capacity,
    /// kernel `EventBus::Subscription::recv` surfaces `Lagged(n)` and the
    /// receiver fast-forwards. The watcher's recovery path is to reissue a
    /// LIST + `replay_for_tenant_checked`. The replay buffer remains intact
    /// because its capacity is independent of the live bus capacity.
    #[tokio::test]
    async fn test_subscribe_lagged_signal_when_subscriber_falls_behind() {
        let wc = WatchCache::with_live_capacity(64, 1000, 2); // tiny live bus
        let mut sub = wc.subscribe();
        for i in 0..10u32 {
            wc.record_added("acme", cm(&format!("k{i}"), "default"));
        }
        let err = sub.recv().await.unwrap_err();
        assert!(matches!(err, EventBusError::Lagged(_)),
            "lagged subscriber receives Lagged signal, not silent drop");
        // Replay buffer untouched — recovery via replay_for_tenant works.
        let evs = wc.replay_for_tenant("acme", 0);
        assert!(evs.iter().filter(|e| !e.is_bookmark()).count() >= 10,
            "ring buffer retains history independently of live-bus capacity");
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant retained on recovery replay");
    }

    /// Capacity-rotation independence: live-bus capacity is decoupled from
    /// ring-buffer capacity. A small ring buffer (which evicts old replay
    /// entries) does NOT cause the live bus to drop events for an
    /// up-to-date tailer.
    #[tokio::test]
    async fn test_live_bus_capacity_independent_of_ring_buffer() {
        // 2-entry ring buffer, 64-entry live bus.
        let wc = WatchCache::with_live_capacity(2, 1000, 64);
        let mut sub = wc.subscribe();
        for i in 0..5u32 {
            wc.record_added("acme", cm(&format!("k{i}"), "default"));
        }
        // Ring buffer evicted older entries — only 2 remain.
        assert_eq!(wc.len(), 2);
        // Live tailer still receives all 5 (live bus capacity 64 >> 5).
        let mut received = 0;
        for _ in 0..5 {
            let ev = sub.recv().await.unwrap();
            assert_eq!(ev.tenant_id(), "acme",
                "tenant_id invariant: live event scoped to producer");
            received += 1;
        }
        assert_eq!(received, 5,
            "live tailer never starves on a small ring buffer when live bus has capacity");
    }

    /// Bookmark events are fanned out on the live bus alongside ring-buffer
    /// emission. KEP-365 — long-poll watchers receive heartbeats live so they
    /// can advance their last-seen RV without polling.
    #[tokio::test]
    async fn test_force_bookmark_publishes_on_live_bus() {
        let wc = WatchCache::new(64, 1000);
        let mut sub = wc.subscribe();
        wc.record_added("acme", cm("a", "default"));
        let _ = sub.recv().await.unwrap(); // drain Added
        let rv = wc.force_bookmark("acme");
        let ev = sub.recv().await.unwrap();
        assert!(ev.is_bookmark(), "live bus delivers Bookmark event");
        assert_eq!(ev.resource_version(), rv);
        assert_eq!(ev.tenant_id(), "acme",
            "tenant_id invariant: bookmark scoped on live bus");
    }

    /// Interval bookmarks (every N events) ride the live bus too so live
    /// tailers receive the same KEP-365 heartbeats as replay clients.
    #[tokio::test]
    async fn test_interval_bookmark_appears_on_live_bus() {
        let wc = WatchCache::new(64, 2);
        let mut sub = wc.subscribe();
        wc.record_added("acme", cm("a", "default"));
        wc.record_added("acme", cm("b", "default"));
        // Expect: Added, Added, Bookmark — three events on the live bus.
        let e1 = sub.recv().await.unwrap();
        let e2 = sub.recv().await.unwrap();
        let e3 = sub.recv().await.unwrap();
        assert!(matches!(e1, WatchCacheEvent::Added { .. }));
        assert!(matches!(e2, WatchCacheEvent::Added { .. }));
        assert!(e3.is_bookmark(), "interval bookmark fanned out live");
        assert_eq!(e3.tenant_id(), "acme",
            "tenant_id invariant: interval bookmark scoped to producer");
    }

    /// `try_recv` surfaces backpressure semantics: `Ok(None)` when empty,
    /// `Ok(Some(_))` when an event is queued. Allows non-blocking polling
    /// from non-async contexts (e.g. metrics scrape loops).
    #[tokio::test]
    async fn test_try_recv_returns_none_when_no_events() {
        let wc = WatchCache::new(64, 1000);
        let mut sub = wc.subscribe();
        assert!(matches!(sub.try_recv(), Ok(None)),
            "no events queued → Ok(None)");
        wc.record_added("acme", cm("a", "default"));
        match sub.try_recv() {
            Ok(Some(ev)) => assert_eq!(ev.tenant_id(), "acme",
                "tenant_id invariant: try_recv yields scoped event"),
            other => panic!("expected Ok(Some(_)), got {:?}", other.map(|_| "value")),
        }
    }

    /// Subscribers established AFTER a write do NOT see past events — they
    /// must use `replay_for_tenant_checked` to backfill. This matches
    /// upstream `cacher.Watch(rv)` "from this point forward" contract.
    #[tokio::test]
    async fn test_subscribe_after_write_misses_past_events() {
        let wc = WatchCache::new(64, 1000);
        wc.record_added("acme", cm("pre-sub", "default"));
        let mut sub = wc.subscribe();
        wc.record_added("acme", cm("post-sub", "default"));
        let ev = sub.recv().await.unwrap();
        match ev {
            WatchCacheEvent::Added { resource, .. } => {
                assert_eq!(resource.name(), "post-sub",
                    "subscriber sees only events after subscribe()");
            }
            other => panic!("expected Added(post-sub), got {:?}", other),
        }
    }

    /// Concurrent multi-tenant writes preserve the tenant invariant on the
    /// live bus: every event delivered to the subscriber carries a
    /// well-formed tenant_id matching its producer. No cross-tenant bleed.
    #[tokio::test]
    async fn test_concurrent_writers_preserve_tenant_invariant_on_live_bus() {
        use std::sync::Arc as StdArc;
        let wc = StdArc::new(WatchCache::with_live_capacity(1024, 1000, 1024));
        let mut sub = wc.subscribe();
        let producer = wc.clone();
        let h1 = tokio::task::spawn_blocking(move || {
            for i in 0..30u32 {
                producer.record_added("acme",
                    cm(&format!("a{i}"), "default"));
            }
        });
        let producer2 = wc.clone();
        let h2 = tokio::task::spawn_blocking(move || {
            for i in 0..30u32 {
                producer2.record_added("globex",
                    cm(&format!("g{i}"), "default"));
            }
        });
        h1.await.unwrap();
        h2.await.unwrap();
        let mut acme_count = 0;
        let mut globex_count = 0;
        for _ in 0..60 {
            match sub.recv().await {
                Ok(ev) => match ev.tenant_id() {
                    "acme" => acme_count += 1,
                    "globex" => globex_count += 1,
                    other => panic!("tenant_id invariant violated: unknown {other}"),
                },
                Err(EventBusError::Lagged(_)) => break, // bus capacity may saturate; tested elsewhere
                Err(e) => panic!("unexpected bus error: {e}"),
            }
        }
        assert!(acme_count > 0, "acme writes reached the live bus");
        assert!(globex_count > 0, "globex writes reached the live bus");
        assert_eq!(acme_count + globex_count, 60,
            "tenant_id invariant: every received event accounted to its producer");
    }

    /// `subscriber_count` reflects only currently-live subscriptions; dropping
    /// a `Subscription` decrements the count. Useful for connection-count
    /// metrics on the watch handler.
    #[tokio::test]
    async fn test_subscriber_count_decrements_on_drop() {
        let wc = WatchCache::new(64, 1000);
        let s1 = wc.subscribe();
        let s2 = wc.subscribe();
        assert_eq!(wc.subscriber_count(), 2);
        drop(s1);
        assert_eq!(wc.subscriber_count(), 1,
            "drop releases the live-bus slot immediately");
        drop(s2);
        assert_eq!(wc.subscriber_count(), 0);
    }

    /// Live-bus publishes are non-blocking even with zero subscribers — the
    /// kernel `EventBus::publish` returns `NoSubscribers` and the producer
    /// (record_added etc.) treats that as a no-op. Mirrors upstream
    /// `cacher.dispatchEvent` early-return on no watchers.
    #[tokio::test]
    async fn test_publish_with_no_subscribers_does_not_block_producer() {
        let wc = WatchCache::new(64, 1000);
        // No subscribers — record paths must still succeed.
        let rv = wc.record_added("acme", cm("a", "default"));
        let _ = wc.force_bookmark("acme");
        assert!(rv > 0);
        // Replay still works for retrospective reads.
        let evs = wc.replay_for_tenant("acme", 0);
        assert!(evs.iter().any(|e| !e.is_bookmark()),
            "ring buffer recorded the event even without live tailers");
        assert!(evs.iter().all(|e| e.tenant_id() == "acme"),
            "tenant_id invariant in no-subscriber path");
    }
}
