// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ResourceQuota tracker — namespace-scoped accounting of pod counts,
//! cpu/memory requests/limits, storage, and arbitrary K8s
//! `count/<group>.<resource>` keys.
//!
//! Mirrors `pkg/quota/v1` of upstream Kubernetes. Admission for
//! resource-creation requests funnels through `check_admit` which
//! returns either Ok or a `QuotaExceeded`-shaped `Error`.

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

/// All quota dimensions in cave-k8s. K8s' full set is open-ended; this
/// enum carries the dimensions that the umbrella tracks itself. Other
/// dimensions (`count/jobs.batch`) are stored as `Custom` strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dimension {
    Pods,
    CpuRequest,
    MemoryRequest,
    CpuLimit,
    MemoryLimit,
    PersistentVolumeClaims,
    StorageRequest,
    Services,
    ConfigMaps,
    Secrets,
    Custom(String),
}

impl Dimension {
    pub fn as_str(&self) -> String {
        match self {
            Dimension::Pods => "pods".into(),
            Dimension::CpuRequest => "requests.cpu".into(),
            Dimension::MemoryRequest => "requests.memory".into(),
            Dimension::CpuLimit => "limits.cpu".into(),
            Dimension::MemoryLimit => "limits.memory".into(),
            Dimension::PersistentVolumeClaims => "persistentvolumeclaims".into(),
            Dimension::StorageRequest => "requests.storage".into(),
            Dimension::Services => "services".into(),
            Dimension::ConfigMaps => "configmaps".into(),
            Dimension::Secrets => "secrets".into(),
            Dimension::Custom(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    pub name: String,
    pub namespace: String,
    pub hard: BTreeMap<String, u64>,
    pub used: BTreeMap<String, u64>,
}

impl Quota {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            hard: BTreeMap::new(),
            used: BTreeMap::new(),
        }
    }
    pub fn with_limit(mut self, dim: Dimension, hard: u64) -> Self {
        self.hard.insert(dim.as_str(), hard);
        self
    }
    pub fn would_exceed(&self, dim: &Dimension, delta: u64) -> Option<u64> {
        let key = dim.as_str();
        let hard = self.hard.get(&key).copied().unwrap_or(u64::MAX);
        let used = self.used.get(&key).copied().unwrap_or(0);
        let new = used.saturating_add(delta);
        if new > hard {
            Some(new - hard)
        } else {
            None
        }
    }
    fn apply_delta(&mut self, dim: &Dimension, delta: i64) {
        let key = dim.as_str();
        let cur = self.used.get(&key).copied().unwrap_or(0) as i64;
        let next = (cur + delta).max(0) as u64;
        self.used.insert(key, next);
    }
}

pub struct QuotaTracker {
    /// (namespace, name) -> Quota
    inner: RwLock<BTreeMap<(String, String), Quota>>,
}

impl Default for QuotaTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaTracker {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn install(&self, q: Quota) {
        let key = (q.namespace.clone(), q.name.clone());
        self.inner.write().expect("quota lock").insert(key, q);
    }

    pub fn get(&self, namespace: &str, name: &str) -> Option<Quota> {
        self.inner
            .read()
            .expect("quota lock")
            .get(&(namespace.to_string(), name.to_string()))
            .cloned()
    }

    pub fn count(&self) -> usize {
        self.inner.read().expect("quota lock").len()
    }

    pub fn list_for_namespace(&self, namespace: &str) -> Vec<Quota> {
        self.inner
            .read()
            .expect("quota lock")
            .iter()
            .filter(|((ns, _), _)| ns == namespace)
            .map(|(_, q)| q.clone())
            .collect()
    }

    /// Admission check.  For each quota in the namespace, verify that
    /// applying `delta` on `dim` would not exceed the hard limit.
    pub fn check_admit(
        &self,
        namespace: &str,
        dim: &Dimension,
        delta: u64,
    ) -> Result<(), Error> {
        let quotas = self.list_for_namespace(namespace);
        for q in &quotas {
            if let Some(over) = q.would_exceed(dim, delta) {
                return Err(Error::QuotaExceeded {
                    quota: format!("{}/{}", q.namespace, q.name),
                    detail: format!("would exceed {} by {}", dim.as_str(), over),
                });
            }
        }
        Ok(())
    }

    /// Commit a successful admission — increment `used` for every quota
    /// that tracks the dimension.  Idempotency keys live in the caller.
    pub fn commit(&self, namespace: &str, dim: &Dimension, delta: i64) {
        let mut g = self.inner.write().expect("quota lock");
        for ((ns, _name), q) in g.iter_mut() {
            if ns == namespace {
                q.apply_delta(dim, delta);
            }
        }
    }

    /// Convenience helper used by tests + the integration smoke tests.
    pub fn admit_and_commit(
        &self,
        namespace: &str,
        dim: &Dimension,
        delta: u64,
    ) -> Result<(), Error> {
        self.check_admit(namespace, dim, delta)?;
        self.commit(namespace, dim, delta as i64);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_namespace_admits_anything() {
        let t = QuotaTracker::new();
        t.check_admit("default", &Dimension::Pods, 1_000_000).unwrap();
    }

    #[test]
    fn admit_under_limit() {
        let t = QuotaTracker::new();
        t.install(
            Quota::new("default", "pods-only").with_limit(Dimension::Pods, 10),
        );
        t.admit_and_commit("default", &Dimension::Pods, 4).unwrap();
        let q = t.get("default", "pods-only").unwrap();
        assert_eq!(q.used.get("pods").copied(), Some(4));
    }

    #[test]
    fn admit_over_limit_rejected() {
        let t = QuotaTracker::new();
        t.install(
            Quota::new("default", "tight").with_limit(Dimension::Pods, 2),
        );
        t.admit_and_commit("default", &Dimension::Pods, 2).unwrap();
        let err = t.check_admit("default", &Dimension::Pods, 1).unwrap_err();
        assert!(matches!(err, Error::QuotaExceeded { .. }));
    }

    #[test]
    fn second_quota_dominates() {
        let t = QuotaTracker::new();
        t.install(
            Quota::new("default", "loose").with_limit(Dimension::Pods, 100),
        );
        t.install(
            Quota::new("default", "strict").with_limit(Dimension::Pods, 1),
        );
        t.admit_and_commit("default", &Dimension::Pods, 1).unwrap();
        let err = t.check_admit("default", &Dimension::Pods, 1).unwrap_err();
        match err {
            Error::QuotaExceeded { quota, .. } => assert!(quota.contains("strict")),
            _ => panic!("expected QuotaExceeded"),
        }
    }

    #[test]
    fn commit_negative_floors_at_zero() {
        let t = QuotaTracker::new();
        t.install(
            Quota::new("default", "pods").with_limit(Dimension::Pods, 5),
        );
        t.commit("default", &Dimension::Pods, -3);
        let q = t.get("default", "pods").unwrap();
        assert_eq!(q.used.get("pods").copied(), Some(0));
    }

    #[test]
    fn custom_dimension_supported() {
        let t = QuotaTracker::new();
        let dim = Dimension::Custom("count/jobs.batch".into());
        t.install(
            Quota::new("default", "jobs").with_limit(dim.clone(), 3),
        );
        t.admit_and_commit("default", &dim, 3).unwrap();
        assert!(matches!(
            t.check_admit("default", &dim, 1).unwrap_err(),
            Error::QuotaExceeded { .. }
        ));
    }

    #[test]
    fn list_for_namespace_partitions() {
        let t = QuotaTracker::new();
        t.install(Quota::new("a", "q1"));
        t.install(Quota::new("a", "q2"));
        t.install(Quota::new("b", "q3"));
        assert_eq!(t.list_for_namespace("a").len(), 2);
        assert_eq!(t.list_for_namespace("b").len(), 1);
        assert_eq!(t.list_for_namespace("c").len(), 0);
    }

    #[test]
    fn would_exceed_returns_overshoot() {
        let q = Quota::new("default", "x").with_limit(Dimension::Pods, 10);
        // 0 used + 11 -> 1 over
        assert_eq!(q.would_exceed(&Dimension::Pods, 11), Some(1));
        assert_eq!(q.would_exceed(&Dimension::Pods, 10), None);
        assert_eq!(q.would_exceed(&Dimension::MemoryRequest, 1_000_000), None);
    }
}
