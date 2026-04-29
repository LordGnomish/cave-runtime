//! SelectorCache — deduplication + reverse-index for EndpointSelectors
//! used by the policy compiler.
//!
//! Mirrors `pkg/policy/selectorcache.go`. The same `EndpointSelector`
//! often appears in many rules; rather than re-evaluate it per-rule
//! the compiler interns each unique selector and tracks which rules
//! reference it. When an identity gets/loses a label, the cache
//! resolves which selectors it newly matches and notifies subscribers
//! so they can patch their `PolicyMap` entries.

use crate::cilium::identity::LabelSet;
use crate::cilium::policy::EndpointSelector;
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Stable id for an interned selector (monotonic).
pub type SelectorId = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedSelector {
    pub id: SelectorId,
    pub selector: EndpointSelector,
    /// Identities currently matched by this selector.
    pub matched: BTreeSet<u32>,
    /// Number of rules referencing this selector.
    pub refcount: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SelectorCacheError {
    #[error("selector id {0} not found")]
    NotFound(SelectorId),
    #[error("tenant {tenant} cannot mutate selector cache owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Notification emitted when a selector's matched-identity set changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectorChange {
    pub selector_id: SelectorId,
    pub added: BTreeSet<u32>,
    pub removed: BTreeSet<u32>,
}

#[derive(Debug)]
pub struct SelectorCache {
    pub tenant: TenantId,
    by_id: HashMap<SelectorId, CachedSelector>,
    /// Selector → id (for dedup). We key by the serialized JSON of the
    /// selector since EndpointSelector contains HashMap<String,String>
    /// which doesn't implement Hash directly.
    by_key: HashMap<String, SelectorId>,
    /// Per-identity label cache (so we can recompute matches when an
    /// identity's labels change).
    identities: BTreeMap<u32, LabelSet>,
    next_id: SelectorId,
    pending_changes: Vec<SelectorChange>,
}

impl SelectorCache {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            by_id: HashMap::new(),
            by_key: HashMap::new(),
            identities: BTreeMap::new(),
            next_id: 1,
            pending_changes: Vec::new(),
        }
    }

    /// Intern a selector. Returns its stable id; equal selectors share an id.
    pub fn intern(&mut self, selector: EndpointSelector) -> SelectorId {
        let key = selector_key(&selector);
        if let Some(&id) = self.by_key.get(&key) {
            if let Some(c) = self.by_id.get_mut(&id) {
                c.refcount += 1;
            }
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut matched: BTreeSet<u32> = BTreeSet::new();
        for (i, labels) in &self.identities {
            if selector.matches(labels) {
                matched.insert(*i);
            }
        }
        self.by_id.insert(id, CachedSelector { id, selector, matched, refcount: 1 });
        self.by_key.insert(key, id);
        id
    }

    /// Release a reference. When refcount hits zero the selector is
    /// removed from the cache.
    pub fn release(&mut self, id: SelectorId) -> Result<(), SelectorCacheError> {
        let entry = self.by_id.get_mut(&id).ok_or(SelectorCacheError::NotFound(id))?;
        if entry.refcount > 0 {
            entry.refcount -= 1;
        }
        if entry.refcount == 0 {
            let removed = self.by_id.remove(&id).unwrap();
            let key = selector_key(&removed.selector);
            self.by_key.remove(&key);
        }
        Ok(())
    }

    /// Insert or update an identity's labels and recompute every
    /// selector's matched set, queueing change notifications.
    pub fn update_identity(&mut self, identity: u32, labels: LabelSet) {
        let prev_labels = self.identities.insert(identity, labels.clone());
        for (id, entry) in self.by_id.iter_mut() {
            let previously_matched = entry.matched.contains(&identity);
            let now_matches = entry.selector.matches(&labels);
            if previously_matched && !now_matches {
                entry.matched.remove(&identity);
                self.pending_changes.push(SelectorChange {
                    selector_id: *id,
                    added: BTreeSet::new(),
                    removed: BTreeSet::from([identity]),
                });
            } else if !previously_matched && now_matches {
                entry.matched.insert(identity);
                self.pending_changes.push(SelectorChange {
                    selector_id: *id,
                    added: BTreeSet::from([identity]),
                    removed: BTreeSet::new(),
                });
            }
        }
        let _ = prev_labels;
    }

    /// Remove an identity entirely (pod deleted).
    pub fn remove_identity(&mut self, identity: u32) {
        if self.identities.remove(&identity).is_none() {
            return;
        }
        for (id, entry) in self.by_id.iter_mut() {
            if entry.matched.remove(&identity) {
                self.pending_changes.push(SelectorChange {
                    selector_id: *id,
                    added: BTreeSet::new(),
                    removed: BTreeSet::from([identity]),
                });
            }
        }
    }

    pub fn matched(&self, id: SelectorId) -> Option<&BTreeSet<u32>> {
        self.by_id.get(&id).map(|c| &c.matched)
    }

    pub fn refcount(&self, id: SelectorId) -> u32 {
        self.by_id.get(&id).map(|c| c.refcount).unwrap_or(0)
    }

    pub fn drain_changes(&mut self) -> Vec<SelectorChange> {
        std::mem::take(&mut self.pending_changes)
    }

    pub fn cached_count(&self) -> usize {
        self.by_id.len()
    }

    pub fn identity_count(&self) -> usize {
        self.identities.len()
    }
}

fn selector_key(s: &EndpointSelector) -> String {
    // Stable serialisation: JSON with sorted match_labels for dedup.
    let mut sorted_labels: Vec<(String, String)> = s.match_labels.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    sorted_labels.sort();
    serde_json::to_string(&(sorted_labels, &s.match_expressions)).unwrap_or_default()
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/policy/selectorcache.go", "SelectorCache");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::identity::LabelSet;
    use crate::cilium_test_ctx;

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    fn endpoint_sel(pairs: &[(&str, &str)]) -> EndpointSelector {
        EndpointSelector {
            match_labels: pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            match_expressions: Vec::new(),
        }
    }

    fn cache(tenant: TenantId) -> SelectorCache {
        SelectorCache::new(tenant)
    }

    // ── Intern / dedup ──────────────────────────────────────────────────────

    #[test]
    fn intern_returns_monotonic_id() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Intern.Monotonic", "tenant-sc-mono");
        let mut c = cache(tenant);
        let a = c.intern(endpoint_sel(&[("app", "a")]));
        let b = c.intern(endpoint_sel(&[("app", "b")]));
        assert_eq!(b, a + 1);
    }

    #[test]
    fn intern_dedupes_equal_selectors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Intern.Dedup", "tenant-sc-d");
        let mut c = cache(tenant);
        let a = c.intern(endpoint_sel(&[("app", "web")]));
        let b = c.intern(endpoint_sel(&[("app", "web")]));
        assert_eq!(a, b);
        assert_eq!(c.refcount(a), 2);
    }

    #[test]
    fn intern_dedupes_regardless_of_label_order() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Intern.Dedup.Order", "tenant-sc-do");
        let mut c = cache(tenant);
        let a = c.intern(endpoint_sel(&[("app", "web"), ("env", "prod")]));
        let b = c.intern(endpoint_sel(&[("env", "prod"), ("app", "web")]));
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_selectors_distinct_ids() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Intern.Distinct", "tenant-sc-di");
        let mut c = cache(tenant);
        let a = c.intern(endpoint_sel(&[("app", "web")]));
        let b = c.intern(endpoint_sel(&[("app", "api")]));
        assert_ne!(a, b);
    }

    // ── Matched set ────────────────────────────────────────────────────────

    #[test]
    fn intern_initialises_matched_with_existing_identities() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Intern.MatchExisting", "tenant-sc-me");
        let mut c = cache(tenant);
        c.update_identity(256, ls(&[("app", "web")]));
        c.update_identity(257, ls(&[("app", "api")]));
        let id = c.intern(endpoint_sel(&[("app", "web")]));
        let m = c.matched(id).unwrap();
        assert_eq!(*m, BTreeSet::from([256]));
    }

    #[test]
    fn matched_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Matched.NotFound", "tenant-sc-mnf");
        let c = cache(tenant);
        assert!(c.matched(99).is_none());
    }

    // ── Identity updates / change notifications ─────────────────────────────

    #[test]
    fn new_identity_triggers_added_change_for_matching_selector() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "UpdateIdentity.Added", "tenant-sc-ua");
        let mut c = cache(tenant);
        let sid = c.intern(endpoint_sel(&[("app", "web")]));
        c.update_identity(256, ls(&[("app", "web")]));
        let changes = c.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].selector_id, sid);
        assert!(changes[0].added.contains(&256));
    }

    #[test]
    fn identity_label_change_can_trigger_removed() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "UpdateIdentity.Removed", "tenant-sc-ur");
        let mut c = cache(tenant);
        let sid = c.intern(endpoint_sel(&[("app", "web")]));
        c.update_identity(256, ls(&[("app", "web")]));
        let _ = c.drain_changes();
        c.update_identity(256, ls(&[("app", "api")]));
        let changes = c.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].selector_id, sid);
        assert!(changes[0].removed.contains(&256));
    }

    #[test]
    fn no_change_when_match_state_unchanged() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "UpdateIdentity.NoChange", "tenant-sc-unc");
        let mut c = cache(tenant);
        c.intern(endpoint_sel(&[("app", "web")]));
        c.update_identity(256, ls(&[("app", "web")]));
        let _ = c.drain_changes();
        c.update_identity(256, ls(&[("app", "web"), ("env", "prod")]));
        let changes = c.drain_changes();
        assert!(changes.is_empty());
    }

    #[test]
    fn remove_identity_emits_removed_changes() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "RemoveIdentity", "tenant-sc-rmi");
        let mut c = cache(tenant);
        let sid = c.intern(endpoint_sel(&[("app", "web")]));
        c.update_identity(256, ls(&[("app", "web")]));
        let _ = c.drain_changes();
        c.remove_identity(256);
        let changes = c.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].selector_id, sid);
        assert!(changes[0].removed.contains(&256));
    }

    #[test]
    fn remove_unknown_identity_emits_nothing() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "RemoveIdentity.NotFound", "tenant-sc-rmnf");
        let mut c = cache(tenant);
        c.intern(endpoint_sel(&[("app", "web")]));
        c.remove_identity(999);
        assert!(c.drain_changes().is_empty());
    }

    // ── Refcount / release ──────────────────────────────────────────────────

    #[test]
    fn release_decrements_refcount() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Release", "tenant-sc-rel");
        let mut c = cache(tenant);
        let id = c.intern(endpoint_sel(&[("app", "web")]));
        c.intern(endpoint_sel(&[("app", "web")]));
        c.release(id).unwrap();
        assert_eq!(c.refcount(id), 1);
    }

    #[test]
    fn release_to_zero_drops_selector() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Release.Drop", "tenant-sc-reld");
        let mut c = cache(tenant);
        let id = c.intern(endpoint_sel(&[("app", "web")]));
        c.release(id).unwrap();
        assert_eq!(c.cached_count(), 0);
    }

    #[test]
    fn release_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Release.NotFound", "tenant-sc-relnf");
        let mut c = cache(tenant);
        let err = c.release(99).unwrap_err();
        assert_eq!(err, SelectorCacheError::NotFound(99));
    }

    // ── Counts ──────────────────────────────────────────────────────────────

    #[test]
    fn cached_count_tracks_intern() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "CachedCount", "tenant-sc-cc");
        let mut c = cache(tenant);
        for i in 0..5 {
            c.intern(endpoint_sel(&[("app", &format!("a{i}"))]));
        }
        assert_eq!(c.cached_count(), 5);
    }

    #[test]
    fn identity_count_tracks_updates() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "IdentityCount", "tenant-sc-ic");
        let mut c = cache(tenant);
        for i in 0..3u32 {
            c.update_identity(256 + i, ls(&[("app", "x")]));
        }
        assert_eq!(c.identity_count(), 3);
    }

    // ── Multi-selector multi-identity ───────────────────────────────────────

    #[test]
    fn multiple_selectors_match_disjoint_identities() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "MultiMatch", "tenant-sc-mm");
        let mut c = cache(tenant);
        let s_web = c.intern(endpoint_sel(&[("app", "web")]));
        let s_api = c.intern(endpoint_sel(&[("app", "api")]));
        c.update_identity(256, ls(&[("app", "web")]));
        c.update_identity(257, ls(&[("app", "api")]));
        c.update_identity(258, ls(&[("app", "metrics")]));
        assert_eq!(c.matched(s_web).unwrap(), &BTreeSet::from([256]));
        assert_eq!(c.matched(s_api).unwrap(), &BTreeSet::from([257]));
    }

    #[test]
    fn empty_selector_matches_all_identities() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "EmptySelector.MatchAll", "tenant-sc-es");
        let mut c = cache(tenant);
        let id = c.intern(EndpointSelector::default());
        c.update_identity(256, ls(&[("app", "web")]));
        c.update_identity(257, ls(&[("app", "api")]));
        let m = c.matched(id).unwrap();
        assert_eq!(m.len(), 2);
    }

    // ── Drain semantics ─────────────────────────────────────────────────────

    #[test]
    fn drain_returns_pending_changes_then_clears() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Drain", "tenant-sc-drn");
        let mut c = cache(tenant);
        c.intern(endpoint_sel(&[("app", "web")]));
        c.update_identity(256, ls(&[("app", "web")]));
        let first = c.drain_changes();
        assert_eq!(first.len(), 1);
        let second = c.drain_changes();
        assert!(second.is_empty());
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn selector_change_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Change.Serde", "tenant-sc-cserde");
        let ch = SelectorChange {
            selector_id: 1,
            added: BTreeSet::from([256, 257]),
            removed: BTreeSet::from([258]),
        };
        let s = serde_json::to_string(&ch).unwrap();
        let back: SelectorChange = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ch);
    }

    #[test]
    fn cached_selector_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/selectorcache.go", "Cached.Serde", "tenant-sc-csserde");
        let cs = CachedSelector {
            id: 1, selector: endpoint_sel(&[("app", "web")]),
            matched: BTreeSet::from([256]),
            refcount: 1,
        };
        let s = serde_json::to_string(&cs).unwrap();
        let back: CachedSelector = serde_json::from_str(&s).unwrap();
        assert_eq!(back, cs);
    }
}
