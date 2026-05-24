// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! provider-kubernetes — minimal in-process `Object` CRD impl.
//!
//! Upstream: github.com/crossplane-contrib/provider-kubernetes
//!
//! Stores `Object`s keyed by `{namespace}/{kind}/{name}` and supports
//! list / get / apply / delete locally. Real cluster IO is Phase 2 via
//! cave-apiserver.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedObject {
    pub namespace: String,
    pub kind: String,
    pub name: String,
    pub manifest: Value,
    pub generation: u64,
}

#[derive(Default)]
pub struct KubernetesProvider {
    objects: DashMap<String, ManagedObject>,
}

impl KubernetesProvider {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(ns: &str, kind: &str, name: &str) -> String {
        format!("{}/{}/{}", ns, kind, name)
    }

    pub fn apply(&self, ns: &str, kind: &str, name: &str, manifest: Value) -> ManagedObject {
        let k = Self::key(ns, kind, name);
        let next_gen = self.objects.get(&k).map(|o| o.generation + 1).unwrap_or(1);
        let obj = ManagedObject {
            namespace: ns.to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            manifest,
            generation: next_gen,
        };
        self.objects.insert(k, obj.clone());
        obj
    }

    pub fn get(&self, ns: &str, kind: &str, name: &str) -> Option<ManagedObject> {
        self.objects.get(&Self::key(ns, kind, name)).map(|o| o.clone())
    }

    pub fn list_objects(&self, ns: &str) -> Vec<ManagedObject> {
        self.objects
            .iter()
            .filter(|o| o.value().namespace == ns)
            .map(|o| o.value().clone())
            .collect()
    }

    pub fn delete(&self, ns: &str, kind: &str, name: &str) -> bool {
        self.objects.remove(&Self::key(ns, kind, name)).is_some()
    }

    pub fn count(&self) -> usize {
        self.objects.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_increments_generation() {
        let p = KubernetesProvider::new();
        let o1 = p.apply("default", "ConfigMap", "cm", json!({"data":{"k":"v"}}));
        assert_eq!(o1.generation, 1);
        let o2 = p.apply("default", "ConfigMap", "cm", json!({"data":{"k":"v2"}}));
        assert_eq!(o2.generation, 2);
    }

    #[test]
    fn get_after_apply() {
        let p = KubernetesProvider::new();
        p.apply("default", "Pod", "x", json!({}));
        assert!(p.get("default", "Pod", "x").is_some());
    }

    #[test]
    fn list_in_namespace() {
        let p = KubernetesProvider::new();
        p.apply("a", "Pod", "x", json!({}));
        p.apply("a", "Pod", "y", json!({}));
        p.apply("b", "Pod", "z", json!({}));
        assert_eq!(p.list_objects("a").len(), 2);
        assert_eq!(p.list_objects("b").len(), 1);
    }

    #[test]
    fn delete_removes() {
        let p = KubernetesProvider::new();
        p.apply("a", "Pod", "x", json!({}));
        assert!(p.delete("a", "Pod", "x"));
        assert!(p.get("a", "Pod", "x").is_none());
    }

    #[test]
    fn delete_unknown_false() {
        let p = KubernetesProvider::new();
        assert!(!p.delete("a", "Pod", "missing"));
    }

    #[test]
    fn count_tracks() {
        let p = KubernetesProvider::new();
        p.apply("a", "Pod", "x", json!({}));
        p.apply("a", "Pod", "y", json!({}));
        assert_eq!(p.count(), 2);
    }
}
