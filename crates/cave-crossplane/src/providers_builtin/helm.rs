// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! provider-helm — minimal in-process `Release` impl.
//!
//! Upstream: github.com/crossplane-contrib/provider-helm
//!
//! Tracks Helm `Release`s keyed by `{namespace}/{name}` with revision history.
//! Real OCI registry pull + cluster apply is Phase 2 via cave-artifacts.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReleaseStatus {
    Deployed,
    Pending,
    Failed,
    Uninstalled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmRelease {
    pub namespace: String,
    pub name: String,
    pub chart: String,
    pub version: String,
    pub revision: u64,
    pub values: serde_json::Value,
    pub status: ReleaseStatus,
}

#[derive(Default)]
pub struct HelmProvider {
    releases: DashMap<String, HelmRelease>,
}

impl HelmProvider {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(ns: &str, name: &str) -> String {
        format!("{}/{}", ns, name)
    }

    pub fn install(
        &self,
        ns: &str,
        name: &str,
        chart: &str,
        version: &str,
        values: serde_json::Value,
    ) -> HelmRelease {
        let k = Self::key(ns, name);
        let revision = self.releases.get(&k).map(|r| r.revision + 1).unwrap_or(1);
        let release = HelmRelease {
            namespace: ns.to_string(),
            name: name.to_string(),
            chart: chart.to_string(),
            version: version.to_string(),
            revision,
            values,
            status: ReleaseStatus::Deployed,
        };
        self.releases.insert(k, release.clone());
        release
    }

    pub fn upgrade(
        &self,
        ns: &str,
        name: &str,
        chart: &str,
        version: &str,
        values: serde_json::Value,
    ) -> Option<HelmRelease> {
        let k = Self::key(ns, name);
        if !self.releases.contains_key(&k) {
            return None;
        }
        Some(self.install(ns, name, chart, version, values))
    }

    pub fn uninstall(&self, ns: &str, name: &str) -> bool {
        let k = Self::key(ns, name);
        if let Some(mut r) = self.releases.get_mut(&k) {
            r.status = ReleaseStatus::Uninstalled;
            return true;
        }
        false
    }

    pub fn get(&self, ns: &str, name: &str) -> Option<HelmRelease> {
        self.releases.get(&Self::key(ns, name)).map(|r| r.clone())
    }

    pub fn list_releases(&self, ns: &str) -> Vec<HelmRelease> {
        self.releases
            .iter()
            .filter(|r| r.value().namespace == ns)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn count(&self) -> usize {
        self.releases.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn install_revision_one() {
        let h = HelmProvider::new();
        let r = h.install("ns", "n", "nginx", "1.0", json!({}));
        assert_eq!(r.revision, 1);
        assert_eq!(r.status, ReleaseStatus::Deployed);
    }

    #[test]
    fn upgrade_increments() {
        let h = HelmProvider::new();
        h.install("ns", "n", "nginx", "1.0", json!({}));
        let r2 = h.upgrade("ns", "n", "nginx", "1.1", json!({"x":1})).unwrap();
        assert_eq!(r2.revision, 2);
    }

    #[test]
    fn upgrade_unknown_none() {
        let h = HelmProvider::new();
        assert!(h.upgrade("ns", "n", "x", "1.0", json!({})).is_none());
    }

    #[test]
    fn uninstall_sets_status() {
        let h = HelmProvider::new();
        h.install("ns", "n", "x", "1.0", json!({}));
        assert!(h.uninstall("ns", "n"));
        assert_eq!(h.get("ns", "n").unwrap().status, ReleaseStatus::Uninstalled);
    }

    #[test]
    fn list_namespaced() {
        let h = HelmProvider::new();
        h.install("a", "x", "c", "1", json!({}));
        h.install("a", "y", "c", "1", json!({}));
        h.install("b", "z", "c", "1", json!({}));
        assert_eq!(h.list_releases("a").len(), 2);
        assert_eq!(h.list_releases("b").len(), 1);
    }

    #[test]
    fn count_tracks() {
        let h = HelmProvider::new();
        h.install("a", "x", "c", "1", json!({}));
        assert_eq!(h.count(), 1);
    }
}
