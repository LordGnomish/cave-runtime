//! Client full — Sync (re-discover endpoints from cluster), keep-alive
//! group (multiplex many lease keep-alives over one stream), watch
//! fragmentation handling, RPC metrics.
//!
//! Mirrors etcd v3.6.10
//!   `client/v3/cluster.go#Sync`,
//!   `client/v3/lease.go#Lease.KeepAlive` (group via channel),
//!   `client/v3/watch.go#fragmentation` (ResponseFragment),
//!   `client/v3/client_metrics.go`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

// ── Sync (endpoint re-discovery) ──────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum ClientFullError {
    SyncFailed(String),
    KeepAliveStreamClosed,
    UnknownLease(i64),
    BadFragment(String),
    UnknownWatch(i64),
}

impl std::fmt::Display for ClientFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SyncFailed(m) => write!(f, "sync failed: {m}"),
            Self::KeepAliveStreamClosed => write!(f, "keep-alive stream closed"),
            Self::UnknownLease(id) => write!(f, "unknown lease: {id}"),
            Self::BadFragment(m) => write!(f, "bad fragment: {m}"),
            Self::UnknownWatch(id) => write!(f, "unknown watch: {id}"),
        }
    }
}

impl std::error::Error for ClientFullError {}

/// `Sync` discovers the currently-known endpoints from the cluster
/// member list and merges them into the local pool.  Mirrors
/// `client/v3/cluster.go#Sync`.
pub struct EndpointSyncer {
    /// Last sync timestamp.
    last_sync: RwLock<Option<Instant>>,
    /// Known endpoints (the merged set).
    endpoints: RwLock<BTreeSet<String>>,
    sync_count: AtomicU64,
}

impl EndpointSyncer {
    pub fn new() -> Self {
        Self {
            last_sync: RwLock::new(None),
            endpoints: RwLock::new(BTreeSet::new()),
            sync_count: AtomicU64::new(0),
        }
    }

    pub fn endpoints(&self) -> Vec<String> {
        self.endpoints.read().unwrap().iter().cloned().collect()
    }

    pub fn last_sync(&self) -> Option<Instant> { *self.last_sync.read().unwrap() }
    pub fn sync_count(&self) -> u64 { self.sync_count.load(Ordering::SeqCst) }

    /// Merge the supplied member list into the local endpoint pool.
    /// Mirrors `client/v3/cluster.go#Sync`'s union-then-replace semantics.
    pub fn sync(&self, member_urls: &[String]) -> Result<(), ClientFullError> {
        if member_urls.is_empty() {
            return Err(ClientFullError::SyncFailed("empty member list".into()));
        }
        let new_set: BTreeSet<String> = member_urls.iter().cloned().collect();
        *self.endpoints.write().unwrap() = new_set;
        *self.last_sync.write().unwrap() = Some(Instant::now());
        self.sync_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Snapshot the current endpoint set, then sync against `new`.  Returns
    /// `(added, removed)` URL lists so the caller can rebuild the balancer.
    pub fn diff_sync(&self, new: &[String]) -> Result<(Vec<String>, Vec<String>), ClientFullError> {
        let before: BTreeSet<String> = self.endpoints.read().unwrap().clone();
        let after: BTreeSet<String> = new.iter().cloned().collect();
        let added: Vec<String> = after.difference(&before).cloned().collect();
        let removed: Vec<String> = before.difference(&after).cloned().collect();
        self.sync(new)?;
        Ok((added, removed))
    }
}

impl Default for EndpointSyncer {
    fn default() -> Self { Self::new() }
}

// ── Keep-alive group ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KeepAliveLease {
    pub id: i64,
    pub ttl_secs: i64,
    /// Wall-clock heartbeat counter — used by tests.
    pub heartbeats: u64,
    pub last_heartbeat: Option<Instant>,
}

/// Multiplexes keep-alive pings for many leases over one logical stream.
/// Mirrors `client/v3/lease.go` where `Lease.KeepAlive` runs all alive
/// leases through a single goroutine.
pub struct KeepAliveGroup {
    leases: Mutex<BTreeMap<i64, KeepAliveLease>>,
    pulses_total: AtomicU64,
    stream_open: RwLock<bool>,
}

impl KeepAliveGroup {
    pub fn new() -> Self {
        Self {
            leases: Mutex::new(BTreeMap::new()),
            pulses_total: AtomicU64::new(0),
            stream_open: RwLock::new(true),
        }
    }

    pub fn open_stream(&self) { *self.stream_open.write().unwrap() = true; }
    pub fn close_stream(&self) { *self.stream_open.write().unwrap() = false; }
    pub fn is_open(&self) -> bool { *self.stream_open.read().unwrap() }

    pub fn add(&self, id: i64, ttl_secs: i64) {
        self.leases.lock().unwrap().insert(id, KeepAliveLease {
            id, ttl_secs, heartbeats: 0, last_heartbeat: None,
        });
    }

    pub fn remove(&self, id: i64) -> bool {
        self.leases.lock().unwrap().remove(&id).is_some()
    }

    pub fn ids(&self) -> Vec<i64> {
        self.leases.lock().unwrap().keys().copied().collect()
    }

    pub fn lease(&self, id: i64) -> Option<KeepAliveLease> {
        self.leases.lock().unwrap().get(&id).cloned()
    }

    /// Pulse one lease — fails if the stream is closed or the lease
    /// has been removed.
    pub fn pulse(&self, id: i64) -> Result<(), ClientFullError> {
        if !self.is_open() { return Err(ClientFullError::KeepAliveStreamClosed); }
        let mut g = self.leases.lock().unwrap();
        let l = g.get_mut(&id).ok_or(ClientFullError::UnknownLease(id))?;
        l.heartbeats += 1;
        l.last_heartbeat = Some(Instant::now());
        self.pulses_total.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Pulse every registered lease.  Stops on first error.
    pub fn pulse_all(&self) -> Result<usize, ClientFullError> {
        let ids = self.ids();
        for id in &ids { self.pulse(*id)?; }
        Ok(ids.len())
    }

    pub fn pulses_total(&self) -> u64 { self.pulses_total.load(Ordering::SeqCst) }
    pub fn len(&self) -> usize { self.leases.lock().unwrap().len() }
}

impl Default for KeepAliveGroup {
    fn default() -> Self { Self::new() }
}

// ── Watch fragmentation ──────────────────────────────────────────────────

/// Reassembles a multi-fragment watch event.  Mirrors
/// `client/v3/watch.go#fragmenter`.
pub struct WatchFragmenter {
    inner: Mutex<BTreeMap<i64, WatchFragmentBuf>>,
}

#[derive(Default)]
struct WatchFragmentBuf {
    accumulated: Vec<u8>,
    fragments_seen: usize,
    expected_fragments: Option<usize>,
}

impl WatchFragmenter {
    pub fn new() -> Self { Self { inner: Mutex::new(BTreeMap::new()) } }

    /// Push a fragment.  `is_final` indicates the last piece for this watch.
    /// Returns `Some(reassembled)` on the final fragment, `None` otherwise.
    pub fn push(
        &self,
        watch_id: i64,
        chunk: &[u8],
        is_final: bool,
    ) -> Result<Option<Vec<u8>>, ClientFullError> {
        let mut g = self.inner.lock().unwrap();
        let buf = g.entry(watch_id).or_default();
        buf.accumulated.extend_from_slice(chunk);
        buf.fragments_seen += 1;
        if is_final {
            buf.expected_fragments = Some(buf.fragments_seen);
            let out = std::mem::take(&mut buf.accumulated);
            g.remove(&watch_id);
            return Ok(Some(out));
        }
        Ok(None)
    }

    pub fn pending_watches(&self) -> Vec<i64> {
        self.inner.lock().unwrap().keys().copied().collect()
    }

    pub fn pending_bytes(&self, watch_id: i64) -> usize {
        self.inner.lock().unwrap().get(&watch_id).map(|b| b.accumulated.len()).unwrap_or(0)
    }

    pub fn cancel(&self, watch_id: i64) -> bool {
        self.inner.lock().unwrap().remove(&watch_id).is_some()
    }

    pub fn fragments_seen(&self, watch_id: i64) -> Option<usize> {
        self.inner.lock().unwrap().get(&watch_id).map(|b| b.fragments_seen)
    }
}

impl Default for WatchFragmenter {
    fn default() -> Self { Self::new() }
}

// ── RPC metrics ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RpcMetricsSnapshot {
    pub total: u64,
    pub failures: u64,
    /// Per-method counts.
    pub per_method: BTreeMap<String, u64>,
    /// Latency observations (count of bucketed entries).
    pub latency_count: u64,
    pub latency_sum_micros: u128,
}

impl RpcMetricsSnapshot {
    pub fn average_micros(&self) -> u128 {
        if self.latency_count == 0 { 0 } else { self.latency_sum_micros / self.latency_count as u128 }
    }

    pub fn failure_rate(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.failures as f64 / self.total as f64 }
    }
}

pub struct RpcMetrics {
    inner: RwLock<RpcMetricsSnapshot>,
    recent: Mutex<VecDeque<(String, Duration, bool)>>,
    recent_cap: usize,
}

impl RpcMetrics {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(RpcMetricsSnapshot::default()),
            recent: Mutex::new(VecDeque::new()),
            recent_cap: 32,
        }
    }

    pub fn observe(&self, method: impl Into<String>, dur: Duration, success: bool) {
        let m = method.into();
        let mut s = self.inner.write().unwrap();
        s.total += 1;
        if !success { s.failures += 1; }
        *s.per_method.entry(m.clone()).or_insert(0) += 1;
        s.latency_count += 1;
        s.latency_sum_micros += dur.as_micros();
        drop(s);
        let mut r = self.recent.lock().unwrap();
        if r.len() >= self.recent_cap { r.pop_front(); }
        r.push_back((m, dur, success));
    }

    pub fn snapshot(&self) -> RpcMetricsSnapshot { self.inner.read().unwrap().clone() }

    pub fn recent(&self) -> Vec<(String, Duration, bool)> {
        self.recent.lock().unwrap().iter().cloned().collect()
    }

    pub fn reset(&self) {
        *self.inner.write().unwrap() = RpcMetricsSnapshot::default();
        self.recent.lock().unwrap().clear();
    }
}

impl Default for RpcMetrics {
    fn default() -> Self { Self::new() }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M8 client_full
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── EndpointSyncer ────────────────────────────────────────────────

    #[test]
    fn test_syncer_starts_empty() {
        let s = EndpointSyncer::new();
        assert!(s.endpoints().is_empty());
        assert!(s.last_sync().is_none());
    }

    #[test]
    fn test_syncer_sync_replaces_set() {
        // cite: client/v3/cluster.go Sync
        let s = EndpointSyncer::new();
        s.sync(&["a".into(), "b".into()]).unwrap();
        s.sync(&["b".into(), "c".into()]).unwrap();
        assert_eq!(s.endpoints(), vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn test_syncer_empty_list_errors() {
        // cite: defensive: must not orphan client
        let s = EndpointSyncer::new();
        assert!(matches!(s.sync(&[]).unwrap_err(), ClientFullError::SyncFailed(_)));
    }

    #[test]
    fn test_syncer_diff_sync_reports_added_removed() {
        // cite: cluster.go (compute delta)
        let s = EndpointSyncer::new();
        s.sync(&["a".into(), "b".into()]).unwrap();
        let (added, removed) = s.diff_sync(&["b".into(), "c".into()]).unwrap();
        assert_eq!(added, vec!["c".to_string()]);
        assert_eq!(removed, vec!["a".to_string()]);
    }

    #[test]
    fn test_syncer_count_increments() {
        let s = EndpointSyncer::new();
        s.sync(&["a".into()]).unwrap();
        s.sync(&["b".into()]).unwrap();
        assert_eq!(s.sync_count(), 2);
    }

    #[test]
    fn test_syncer_records_last_sync() {
        let s = EndpointSyncer::new();
        s.sync(&["a".into()]).unwrap();
        assert!(s.last_sync().is_some());
    }

    // ── KeepAliveGroup ────────────────────────────────────────────────

    #[test]
    fn test_keep_alive_group_pulse_increments_heartbeat() {
        let g = KeepAliveGroup::new();
        g.add(7, 30);
        g.pulse(7).unwrap();
        g.pulse(7).unwrap();
        assert_eq!(g.lease(7).unwrap().heartbeats, 2);
    }

    #[test]
    fn test_keep_alive_group_pulse_all() {
        let g = KeepAliveGroup::new();
        g.add(1, 30);
        g.add(2, 30);
        g.add(3, 30);
        assert_eq!(g.pulse_all().unwrap(), 3);
        assert_eq!(g.pulses_total(), 3);
    }

    #[test]
    fn test_keep_alive_group_pulse_unknown_errors() {
        let g = KeepAliveGroup::new();
        assert_eq!(g.pulse(99).unwrap_err(), ClientFullError::UnknownLease(99));
    }

    #[test]
    fn test_keep_alive_group_closed_stream_blocks_pulse() {
        // cite: lease.go (closed stream ⇒ ErrKeepAliveHalted)
        let g = KeepAliveGroup::new();
        g.add(7, 30);
        g.close_stream();
        assert_eq!(g.pulse(7).unwrap_err(), ClientFullError::KeepAliveStreamClosed);
    }

    #[test]
    fn test_keep_alive_group_remove_returns_true() {
        let g = KeepAliveGroup::new();
        g.add(7, 30);
        assert!(g.remove(7));
        assert!(!g.remove(7));
    }

    #[test]
    fn test_keep_alive_group_ids_listing() {
        let g = KeepAliveGroup::new();
        for id in 1..=3i64 { g.add(id, 30); }
        let mut ids = g.ids();
        ids.sort();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn test_keep_alive_group_reopen_resumes_pulses() {
        let g = KeepAliveGroup::new();
        g.add(7, 30);
        g.close_stream();
        assert!(g.pulse(7).is_err());
        g.open_stream();
        assert!(g.pulse(7).is_ok());
    }

    // ── WatchFragmenter ───────────────────────────────────────────────

    #[test]
    fn test_fragmenter_assembles_on_final() {
        // cite: watch.go ResponseFragment
        let f = WatchFragmenter::new();
        assert!(f.push(7, b"hel", false).unwrap().is_none());
        assert!(f.push(7, b"lo ", false).unwrap().is_none());
        let r = f.push(7, b"world", true).unwrap().unwrap();
        assert_eq!(r, b"hello world");
    }

    #[test]
    fn test_fragmenter_clears_buffer_after_finish() {
        let f = WatchFragmenter::new();
        f.push(7, b"x", false).unwrap();
        f.push(7, b"y", true).unwrap();
        assert!(!f.pending_watches().contains(&7));
    }

    #[test]
    fn test_fragmenter_pending_bytes() {
        let f = WatchFragmenter::new();
        f.push(7, b"AAAA", false).unwrap();
        f.push(7, b"BB", false).unwrap();
        assert_eq!(f.pending_bytes(7), 6);
    }

    #[test]
    fn test_fragmenter_cancel_drops_buffer() {
        let f = WatchFragmenter::new();
        f.push(7, b"x", false).unwrap();
        assert!(f.cancel(7));
        assert!(!f.pending_watches().contains(&7));
    }

    #[test]
    fn test_fragmenter_fragments_seen_counter() {
        let f = WatchFragmenter::new();
        f.push(7, b"a", false).unwrap();
        f.push(7, b"b", false).unwrap();
        assert_eq!(f.fragments_seen(7), Some(2));
    }

    #[test]
    fn test_fragmenter_two_watches_isolated() {
        // cite: watch.go (fragmenters keyed by watch_id)
        let f = WatchFragmenter::new();
        f.push(7, b"AAAA", false).unwrap();
        f.push(8, b"BBBB", false).unwrap();
        assert_eq!(f.pending_bytes(7), 4);
        assert_eq!(f.pending_bytes(8), 4);
    }

    // ── RpcMetrics ────────────────────────────────────────────────────

    #[test]
    fn test_metrics_observe_counts() {
        // cite: client_metrics.go
        let m = RpcMetrics::new();
        m.observe("Range", Duration::from_micros(100), true);
        m.observe("Range", Duration::from_micros(200), true);
        m.observe("Put", Duration::from_micros(50), false);
        let s = m.snapshot();
        assert_eq!(s.total, 3);
        assert_eq!(s.failures, 1);
        assert_eq!(s.per_method["Range"], 2);
        assert_eq!(s.per_method["Put"], 1);
    }

    #[test]
    fn test_metrics_average_latency() {
        let m = RpcMetrics::new();
        m.observe("Range", Duration::from_micros(100), true);
        m.observe("Range", Duration::from_micros(300), true);
        let s = m.snapshot();
        assert_eq!(s.average_micros(), 200);
    }

    #[test]
    fn test_metrics_failure_rate() {
        let m = RpcMetrics::new();
        m.observe("X", Duration::from_micros(1), true);
        m.observe("X", Duration::from_micros(1), false);
        m.observe("X", Duration::from_micros(1), false);
        m.observe("X", Duration::from_micros(1), false);
        let s = m.snapshot();
        assert!((s.failure_rate() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn test_metrics_recent_capped() {
        let m = RpcMetrics::new();
        for _ in 0..50 { m.observe("X", Duration::from_micros(1), true); }
        assert_eq!(m.recent().len(), 32);
    }

    #[test]
    fn test_metrics_reset_clears_state() {
        let m = RpcMetrics::new();
        m.observe("X", Duration::from_micros(1), true);
        m.reset();
        let s = m.snapshot();
        assert_eq!(s.total, 0);
        assert!(m.recent().is_empty());
    }

    #[test]
    fn test_metrics_failure_rate_zero_when_empty() {
        let m = RpcMetrics::new();
        assert_eq!(m.snapshot().failure_rate(), 0.0);
        assert_eq!(m.snapshot().average_micros(), 0);
    }
}
