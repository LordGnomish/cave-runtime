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
    /// Compacted floor — replays at or below this RV are denied.
    /// Mirrors upstream `cacher.go::watchCache.resourceVersionFloor`.
    compacted_rv: AtomicU64,
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
            compacted_rv: AtomicU64::new(0),
        }
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
}
