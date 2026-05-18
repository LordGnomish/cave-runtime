// SPDX-License-Identifier: AGPL-3.0-or-later
//! Watch event filters — `NOPUT` / `NODELETE` / `prev_kv`-strip per-watch
//! filtering layered on top of the existing watch dispatcher in
//! [`crate::store`].
//!
//! Mirrors etcd v3.6.10
//!   `api/etcdserverpb/rpc.proto` (`WatchCreateRequest.filters`,
//!   `WatchCreateRequest.prev_kv`, `WatchCreateRequest.watch_id`)
//!   `server/etcdserver/api/v3rpc/watch.go` (`filterEvent`).

use crate::models::{EventType, WatchConfig, WatchEvent};
use serde::{Deserialize, Serialize};

/// A filter the client sets at watch-create time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchFilter {
    /// Drop `Put` events.  Mirrors `WatchCreateRequest.NOPUT`.
    NoPut,
    /// Drop `Delete` events.  Mirrors `WatchCreateRequest.NODELETE`.
    NoDelete,
}

/// Composite filter: the set of [`WatchFilter`]s plus a `strip_prev_kv`
/// flag that controls whether the event's `prev_kv` should be cleared
/// before dispatch.  In etcd v3.6.10 the filter set lives on
/// `WatchCreateRequest.filters`; cave-etcd flattens it into one record so
/// the dispatcher can consult both decisions in a single pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WatchFilterSet {
    pub filters: Vec<WatchFilter>,
    /// `true` ⇒ the watcher requested `prev_kv=true`; events keep their
    /// `prev_kv`.  `false` ⇒ strip `prev_kv` to `None` before dispatch.
    pub keep_prev_kv: bool,
}

impl WatchFilterSet {
    pub fn from_config(cfg: &WatchConfig) -> Self {
        Self {
            filters: vec![],
            keep_prev_kv: cfg.prev_kv,
        }
    }

    pub fn with_filter(mut self, f: WatchFilter) -> Self {
        if !self.filters.contains(&f) {
            self.filters.push(f);
        }
        self
    }

    /// Decide whether to deliver `event`.  Returns `Some(adjusted)` when
    /// the event survives the filter set; `None` when it must be dropped.
    pub fn apply(&self, event: &WatchEvent) -> Option<WatchEvent> {
        for f in &self.filters {
            match (f, &event.event_type) {
                (WatchFilter::NoPut, EventType::Put) => return None,
                (WatchFilter::NoDelete, EventType::Delete) => return None,
                _ => {}
            }
        }
        let mut out = event.clone();
        if !self.keep_prev_kv {
            out.prev_kv = None;
        }
        Some(out)
    }

    pub fn drops_puts(&self) -> bool {
        self.filters.contains(&WatchFilter::NoPut)
    }

    pub fn drops_deletes(&self) -> bool {
        self.filters.contains(&WatchFilter::NoDelete)
    }
}

/// In-process registry: `watch_id → WatchFilterSet`.  Held alongside the
/// existing watch_configs in [`crate::store::KvStore`] but kept as an
/// independent type so deeper-003 can iterate without touching the core.
#[derive(Default)]
pub struct WatchFilterRegistry {
    by_id: dashmap::DashMap<i64, WatchFilterSet>,
}

impl WatchFilterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, watch_id: i64, set: WatchFilterSet) {
        self.by_id.insert(watch_id, set);
    }

    pub fn get(&self, watch_id: i64) -> Option<WatchFilterSet> {
        self.by_id.get(&watch_id).map(|r| r.clone())
    }

    pub fn deregister(&self, watch_id: i64) -> Option<WatchFilterSet> {
        self.by_id.remove(&watch_id).map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Apply a filter set (looked up by `watch_id`) to an inbound event.
    /// Returns `None` when no filter is registered (caller should pass
    /// the event through unchanged).
    pub fn filter_for(&self, watch_id: i64) -> Option<WatchFilterSet> {
        self.by_id.get(&watch_id).map(|r| r.clone())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Watch-filter tests — feat/cave-etcd-deeper-003
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::KeyValue;

    fn dt(tenant_id: &str, suffix: &str) -> Vec<u8> {
        format!("/tenants/{}/{}", tenant_id, suffix).into_bytes()
    }

    fn put_event(tenant_id: &str, suffix: &str) -> WatchEvent {
        WatchEvent {
            event_type: EventType::Put,
            kv: KeyValue {
                key: dt(tenant_id, suffix),
                value: b"v".to_vec(),
                create_revision: 1,
                mod_revision: 1,
                version: 1,
                lease: None,
            },
            prev_kv: Some(KeyValue {
                key: dt(tenant_id, suffix),
                value: b"prev".to_vec(),
                create_revision: 1,
                mod_revision: 0,
                version: 0,
                lease: None,
            }),
        }
    }

    fn delete_event(tenant_id: &str, suffix: &str) -> WatchEvent {
        WatchEvent {
            event_type: EventType::Delete,
            kv: KeyValue {
                key: dt(tenant_id, suffix),
                value: vec![],
                create_revision: 1,
                mod_revision: 2,
                version: 0,
                lease: None,
            },
            prev_kv: Some(KeyValue {
                key: dt(tenant_id, suffix),
                value: b"old".to_vec(),
                create_revision: 1,
                mod_revision: 1,
                version: 1,
                lease: None,
            }),
        }
    }

    #[test]
    fn test_filter_no_put_drops_put_events() {
        // cite: etcd v3.6.10 WatchCreateRequest.NOPUT
        let tenant_id = "wf-001";
        let set = WatchFilterSet::default().with_filter(WatchFilter::NoPut);
        assert!(set.apply(&put_event(tenant_id, "k")).is_none());
        assert!(set.apply(&delete_event(tenant_id, "k")).is_some());
    }

    #[test]
    fn test_filter_no_delete_drops_delete_events() {
        // cite: etcd v3.6.10 WatchCreateRequest.NODELETE
        let tenant_id = "wf-002";
        let set = WatchFilterSet::default().with_filter(WatchFilter::NoDelete);
        assert!(set.apply(&delete_event(tenant_id, "k")).is_none());
        assert!(set.apply(&put_event(tenant_id, "k")).is_some());
    }

    #[test]
    fn test_filter_strips_prev_kv_when_not_requested() {
        // cite: etcd v3.6.10 WatchCreateRequest.prev_kv (false → strip)
        let tenant_id = "wf-003";
        let set = WatchFilterSet::default(); // keep_prev_kv = false
        let out = set.apply(&put_event(tenant_id, "k")).unwrap();
        assert!(out.prev_kv.is_none());
    }

    #[test]
    fn test_filter_keeps_prev_kv_when_requested() {
        // cite: etcd v3.6.10 WatchCreateRequest.prev_kv (true → keep)
        let tenant_id = "wf-004";
        let mut set = WatchFilterSet::default();
        set.keep_prev_kv = true;
        let out = set.apply(&put_event(tenant_id, "k")).unwrap();
        assert!(out.prev_kv.is_some());
    }

    #[test]
    fn test_filter_combination_no_put_and_no_delete_drops_all() {
        // cite: etcd v3.6.10 (multiple filters AND)
        let tenant_id = "wf-005";
        let set = WatchFilterSet::default()
            .with_filter(WatchFilter::NoPut)
            .with_filter(WatchFilter::NoDelete);
        assert!(set.apply(&put_event(tenant_id, "k")).is_none());
        assert!(set.apply(&delete_event(tenant_id, "k")).is_none());
    }

    #[test]
    fn test_filter_with_filter_dedupes() {
        // cite: etcd v3.6.10 WatchCreateRequest.filters (dedup)
        let _tenant_id = "wf-006";
        let set = WatchFilterSet::default()
            .with_filter(WatchFilter::NoPut)
            .with_filter(WatchFilter::NoPut);
        assert_eq!(set.filters.len(), 1);
    }

    #[test]
    fn test_filter_registry_register_get_deregister() {
        // cite: etcd v3.6.10 watcher_group state machine
        let _tenant_id = "wf-007";
        let reg = WatchFilterRegistry::new();
        let set = WatchFilterSet::default().with_filter(WatchFilter::NoPut);
        reg.register(7, set.clone());
        assert_eq!(reg.len(), 1);
        let got = reg.get(7).unwrap();
        assert!(got.drops_puts());
        let removed = reg.deregister(7).unwrap();
        assert!(removed.drops_puts());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_filter_drops_helpers() {
        // cite: etcd v3.6.10 (helper introspection on filter set)
        let _tenant_id = "wf-008";
        let set_a = WatchFilterSet::default().with_filter(WatchFilter::NoPut);
        let set_b = WatchFilterSet::default().with_filter(WatchFilter::NoDelete);
        assert!(set_a.drops_puts() && !set_a.drops_deletes());
        assert!(set_b.drops_deletes() && !set_b.drops_puts());
    }

    #[test]
    fn test_filter_set_from_config_inherits_prev_kv() {
        // cite: etcd v3.6.10 WatchConfig.prev_kv → filter set
        let tenant_id = "wf-009";
        let cfg = WatchConfig {
            watch_id: 1,
            key: dt(tenant_id, "k"),
            range_end: None,
            start_revision: None,
            prev_kv: true,
        };
        let set = WatchFilterSet::from_config(&cfg);
        assert!(set.keep_prev_kv);
    }
}
