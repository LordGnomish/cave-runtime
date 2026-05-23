// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! API Aggregator registry.
//!
//! Tracks `APIService` registrations — pointers from the cave-k8s
//! `/apis/<group>/<version>` discovery surface at an external HTTP
//! endpoint.  Mirrors `staging/src/k8s.io/kube-aggregator`.
//!
//! cave-k8s consumes the registry from `discovery.rs` to build the
//! merged `/apis` doc + from `routes.rs` to forward requests to the
//! extension API server.  An entry remains *pending* until at least one
//! `mark_available` arrives; the discovery layer hides pending entries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiService {
    /// `<version>.<group>` — e.g. `v1alpha1.metrics.k8s.io`.
    pub name: String,
    pub group: String,
    pub version: String,
    /// Service backing this APIService — `<namespace>/<name>:<port>`.
    pub service: String,
    /// Insecure mode bypasses TLS verification.  Defaults to `false`.
    pub insecure_skip_tls_verify: bool,
    pub group_priority_minimum: u32,
    pub version_priority: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AvailabilityCondition {
    Pending,
    Available,
    Unavailable(String),
}

pub struct AggregatorRegistry {
    inner: RwLock<BTreeMap<String, (ApiService, AvailabilityCondition)>>,
}

impl Default for AggregatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AggregatorRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn register(&self, svc: ApiService) {
        let key = svc.name.clone();
        self.inner
            .write()
            .expect("aggr lock")
            .insert(key, (svc, AvailabilityCondition::Pending));
    }

    pub fn mark_available(&self, name: &str) -> bool {
        if let Some(e) = self.inner.write().expect("aggr lock").get_mut(name) {
            e.1 = AvailabilityCondition::Available;
            true
        } else {
            false
        }
    }

    pub fn mark_unavailable(&self, name: &str, reason: impl Into<String>) -> bool {
        if let Some(e) = self.inner.write().expect("aggr lock").get_mut(name) {
            e.1 = AvailabilityCondition::Unavailable(reason.into());
            true
        } else {
            false
        }
    }

    pub fn deregister(&self, name: &str) -> bool {
        self.inner.write().expect("aggr lock").remove(name).is_some()
    }

    pub fn count(&self) -> usize {
        self.inner.read().expect("aggr lock").len()
    }

    pub fn available_groups(&self) -> Vec<String> {
        let g = self.inner.read().expect("aggr lock");
        let mut set = std::collections::BTreeSet::new();
        for (svc, cond) in g.values() {
            if matches!(cond, AvailabilityCondition::Available) {
                set.insert(svc.group.clone());
            }
        }
        set.into_iter().collect()
    }

    /// Resolve a `(group, version)` to its registered backend service
    /// — used by request forwarding.
    pub fn resolve(&self, group: &str, version: &str) -> Option<ApiService> {
        let g = self.inner.read().expect("aggr lock");
        for (svc, cond) in g.values() {
            if svc.group == group
                && svc.version == version
                && matches!(cond, AvailabilityCondition::Available)
            {
                return Some(svc.clone());
            }
        }
        None
    }

    pub fn list(&self) -> Vec<(ApiService, AvailabilityCondition)> {
        self.inner.read().expect("aggr lock").values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, group: &str, version: &str) -> ApiService {
        ApiService {
            name: name.into(),
            group: group.into(),
            version: version.into(),
            service: "kube-system/metrics:443".into(),
            insecure_skip_tls_verify: false,
            group_priority_minimum: 100,
            version_priority: 10,
        }
    }

    #[test]
    fn register_starts_as_pending() {
        let r = AggregatorRegistry::new();
        r.register(svc("v1.metrics.k8s.io", "metrics.k8s.io", "v1"));
        let l = r.list();
        assert_eq!(l.len(), 1);
        assert!(matches!(l[0].1, AvailabilityCondition::Pending));
    }

    #[test]
    fn mark_available_flips_condition() {
        let r = AggregatorRegistry::new();
        r.register(svc("v1.metrics.k8s.io", "metrics.k8s.io", "v1"));
        assert!(r.mark_available("v1.metrics.k8s.io"));
        let l = r.list();
        assert!(matches!(l[0].1, AvailabilityCondition::Available));
    }

    #[test]
    fn mark_unavailable_carries_reason() {
        let r = AggregatorRegistry::new();
        r.register(svc("v1.x.io", "x.io", "v1"));
        r.mark_unavailable("v1.x.io", "endpoint unreachable");
        let cond = &r.list()[0].1;
        match cond {
            AvailabilityCondition::Unavailable(s) => assert_eq!(s, "endpoint unreachable"),
            _ => panic!(),
        }
    }

    #[test]
    fn resolve_only_returns_available() {
        let r = AggregatorRegistry::new();
        r.register(svc("v1.a.io", "a.io", "v1"));
        r.register(svc("v1.b.io", "b.io", "v1"));
        r.mark_available("v1.b.io");
        assert!(r.resolve("a.io", "v1").is_none());
        assert!(r.resolve("b.io", "v1").is_some());
    }

    #[test]
    fn available_groups_dedups() {
        let r = AggregatorRegistry::new();
        r.register(svc("v1.x.io", "x.io", "v1"));
        r.register(svc("v2.x.io", "x.io", "v2"));
        r.mark_available("v1.x.io");
        r.mark_available("v2.x.io");
        let g = r.available_groups();
        assert_eq!(g, vec!["x.io"]);
    }

    #[test]
    fn deregister_removes_entry() {
        let r = AggregatorRegistry::new();
        r.register(svc("v1.y.io", "y.io", "v1"));
        assert!(r.deregister("v1.y.io"));
        assert!(!r.deregister("v1.y.io"));
        assert_eq!(r.count(), 0);
    }

    #[test]
    fn mark_available_unknown_returns_false() {
        let r = AggregatorRegistry::new();
        assert!(!r.mark_available("never-registered"));
        assert!(!r.mark_unavailable("nope", "x"));
    }
}
