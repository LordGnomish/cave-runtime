//! `storageversiongarbagecollector` — removes `StorageVersion`
//! objects whose owning `APIService` is gone.
//!
//! Mirrors `pkg/controller/storageversiongarbagecollector/`
//! from upstream. The apiserver's storage-version controller
//! writes `StorageVersion` rows describing how each
//! `(group, version, kind)` is persisted; when the owning
//! `APIService` is deleted, those rows become orphan garbage.
//! This GC loop reaps them.

use std::collections::{BTreeMap, BTreeSet};

/// One `StorageVersion` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageVersionRef {
    /// `metadata.name` — typically `<resource>.<group>`, e.g.
    /// `deployments.apps`.
    pub name: String,
    /// `spec.storageServers[*].apiServerID` — every apiserver
    /// that has written this row.
    pub owner_apiservers: BTreeSet<String>,
}

/// The collector's view of the world.
#[derive(Debug, Default)]
pub struct GarbageCollector {
    /// StorageVersion rows the controller is tracking.
    storage_versions: BTreeMap<String, StorageVersionRef>,
    /// APIServer IDs currently registered with the cluster.
    live_apiservers: BTreeSet<String>,
}

impl GarbageCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observer notification: a new StorageVersion row landed.
    pub fn observe_storage_version(&mut self, sv: StorageVersionRef) {
        self.storage_versions.insert(sv.name.clone(), sv);
    }

    /// Observer notification: a StorageVersion was deleted.
    pub fn forget_storage_version(&mut self, name: &str) {
        self.storage_versions.remove(name);
    }

    /// Observer notification: an APIServer registered with the
    /// cluster.
    pub fn observe_apiserver(&mut self, id: impl Into<String>) {
        self.live_apiservers.insert(id.into());
    }

    /// Observer notification: an APIServer was removed.
    pub fn forget_apiserver(&mut self, id: &str) {
        self.live_apiservers.remove(id);
    }

    /// Compute the GC plan — which `StorageVersion` rows can be
    /// dropped because none of their owning apiservers are
    /// alive. Returns the list of names to delete in stable
    /// alphabetical order.
    pub fn gc_plan(&self) -> Vec<String> {
        let mut targets = Vec::new();
        for (name, sv) in &self.storage_versions {
            // If a StorageVersion has zero owners listed,
            // it's already an orphan.
            if sv.owner_apiservers.is_empty() {
                targets.push(name.clone());
                continue;
            }
            let any_live = sv
                .owner_apiservers
                .iter()
                .any(|id| self.live_apiservers.contains(id));
            if !any_live {
                targets.push(name.clone());
            }
        }
        targets
    }

    /// Number of tracked storage-version rows.
    pub fn tracked_count(&self) -> usize {
        self.storage_versions.len()
    }

    /// Number of live apiservers.
    pub fn live_apiserver_count(&self) -> usize {
        self.live_apiservers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(name: &str, owners: &[&str]) -> StorageVersionRef {
        StorageVersionRef {
            name: name.into(),
            owner_apiservers: owners.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn no_orphans_when_all_owners_live() {
        let mut g = GarbageCollector::new();
        g.observe_apiserver("api-1");
        g.observe_storage_version(sv("deployments.apps", &["api-1"]));
        assert!(g.gc_plan().is_empty());
    }

    #[test]
    fn orphan_when_no_owners_live() {
        let mut g = GarbageCollector::new();
        g.observe_storage_version(sv("deployments.apps", &["api-1"]));
        // No apiserver registered → owner not live.
        assert_eq!(g.gc_plan(), vec!["deployments.apps".to_string()]);
    }

    #[test]
    fn orphan_when_zero_owners_listed() {
        let mut g = GarbageCollector::new();
        g.observe_apiserver("api-1");
        g.observe_storage_version(sv("deployments.apps", &[]));
        assert_eq!(g.gc_plan(), vec!["deployments.apps".to_string()]);
    }

    #[test]
    fn alive_when_any_one_owner_live() {
        let mut g = GarbageCollector::new();
        g.observe_apiserver("api-2");
        g.observe_storage_version(sv("d.apps", &["api-1", "api-2", "api-3"]));
        assert!(g.gc_plan().is_empty());
    }

    #[test]
    fn gc_plan_alphabetical() {
        let mut g = GarbageCollector::new();
        for name in ["z", "a", "m"] {
            g.observe_storage_version(sv(name, &[]));
        }
        assert_eq!(g.gc_plan(), vec!["a", "m", "z"]);
    }

    #[test]
    fn forget_apiserver_makes_remaining_owners_evaluated() {
        let mut g = GarbageCollector::new();
        g.observe_apiserver("api-1");
        g.observe_storage_version(sv("d.apps", &["api-1"]));
        g.forget_apiserver("api-1");
        assert_eq!(g.gc_plan(), vec!["d.apps".to_string()]);
    }

    #[test]
    fn forget_storage_version_drops_it() {
        let mut g = GarbageCollector::new();
        g.observe_storage_version(sv("d", &[]));
        g.forget_storage_version("d");
        assert!(g.gc_plan().is_empty());
        assert_eq!(g.tracked_count(), 0);
    }

    #[test]
    fn tracked_counts_reflect_state() {
        let mut g = GarbageCollector::new();
        g.observe_apiserver("a");
        g.observe_apiserver("b");
        g.observe_storage_version(sv("x", &["a"]));
        assert_eq!(g.tracked_count(), 1);
        assert_eq!(g.live_apiserver_count(), 2);
    }

    #[test]
    fn observe_storage_version_overwrites_previous() {
        let mut g = GarbageCollector::new();
        g.observe_storage_version(sv("d", &["a"]));
        g.observe_storage_version(sv("d", &["b"]));
        // owners replaced — apiserver "a" gone means we still
        // depend on b being live for d to stay.
        g.observe_apiserver("b");
        assert!(g.gc_plan().is_empty());
    }
}
