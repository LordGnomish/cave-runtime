// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Drain controller — PDB-aware pod-eviction loop.
//!
//! Ports the eviction half of `pkg/controllers/node/termination/terminator/terminator.go`
//! and the PodDisruptionBudget arbitration in
//! `pkg/controllers/node/termination/terminator/eviction.go` from upstream
//! Karpenter v1.4.0.
//!
//! Pre-c-tier-uplift the cave-karpenter drain layer only flipped
//! `claim.drained = true`; the actual eviction loop was deferred to
//! cave-kubelet's evict path. This module brings the loop in-crate:
//!
//! * Build a [`DrainPlan`] over the pods scheduled to the node.
//! * Drive [`DrainPlan::step`] in a reconcile cadence; each call drains
//!   one eligible pod respecting:
//!     - DaemonSet pods are *never* evicted (Kubernetes contract).
//!     - Pods governed by a PodDisruptionBudget that would be violated by
//!       the eviction are deferred.
//!     - Pods past their grace deadline are force-evicted.
//! * The plan reports [`DrainStatus::Complete`] when no more pods remain
//!   evictable; the lifecycle controller then proceeds to terminate.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// Lightweight pod descriptor — the controller only needs identifiers and
/// the labels needed to match a PDB.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PodDescriptor {
    pub namespace: String,
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub owner_kind: PodOwnerKind,
    /// Tracking field — when present, the pod is treated as past grace.
    pub grace_deadline: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PodOwnerKind {
    Deployment,
    StatefulSet,
    DaemonSet,
    Job,
    Standalone,
}

/// PodDisruptionBudget — minimum number of pods that must remain available
/// after eviction. `min_available` is matched in absolute terms (the
/// percentage form upstream resolves before this layer).
#[derive(Debug, Clone)]
pub struct PodDisruptionBudget {
    pub namespace: String,
    pub name: String,
    pub selector: Vec<(String, String)>,
    pub min_available: u32,
}

impl PodDisruptionBudget {
    pub fn matches(&self, pod: &PodDescriptor) -> bool {
        if self.namespace != pod.namespace {
            return false;
        }
        // Every label in the selector must be present on the pod.
        self.selector
            .iter()
            .all(|(k, v)| pod.labels.iter().any(|(pk, pv)| pk == k && pv == v))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainStatus {
    InProgress { remaining: usize, blocked_by_pdb: usize },
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvictionDecision {
    Evicted(PodDescriptor),
    /// The pod is exempt (DaemonSet, mirror, etc.).
    Skipped(PodDescriptor),
    /// PDB would be violated; defer to the next reconcile pass.
    BlockedByPdb(PodDescriptor, String),
    /// No more evictable pods.
    Idle,
}

#[derive(Debug, Clone)]
pub struct DrainPlan {
    pods: Vec<PodDescriptor>,
    pdbs: Vec<PodDisruptionBudget>,
    evicted: Vec<PodDescriptor>,
    /// PDB → count of *evicted* pods that matched it.
    pdb_evicted: HashMap<String, u32>,
    /// PDB → original number of matching pods (computed at plan creation).
    pdb_baseline: HashMap<String, u32>,
}

fn pdb_key(pdb: &PodDisruptionBudget) -> String {
    format!("{}/{}", pdb.namespace, pdb.name)
}

impl DrainPlan {
    pub fn new(pods: Vec<PodDescriptor>, pdbs: Vec<PodDisruptionBudget>) -> Self {
        let mut pdb_baseline: HashMap<String, u32> = HashMap::new();
        for pdb in &pdbs {
            let count = pods.iter().filter(|p| pdb.matches(p)).count() as u32;
            pdb_baseline.insert(pdb_key(pdb), count);
        }
        DrainPlan {
            pods,
            pdbs,
            evicted: Vec::new(),
            pdb_evicted: HashMap::new(),
            pdb_baseline,
        }
    }

    pub fn evicted_pods(&self) -> &[PodDescriptor] {
        &self.evicted
    }

    pub fn remaining(&self) -> usize {
        self.pods.len()
    }

    /// Drive one pass of the eviction loop. Returns the decision taken for
    /// the *first* candidate pod considered — so callers can log and
    /// continue calling `step()` until [`DrainStatus::Complete`].
    pub fn step(&mut self, now: SystemTime) -> EvictionDecision {
        // Choose the next candidate. Prefer pods past their grace deadline.
        let idx = self.pods.iter().position(|p| {
            p.grace_deadline.is_some_and(|d| now >= d)
                && !matches!(p.owner_kind, PodOwnerKind::DaemonSet)
        });
        let idx = idx.or_else(|| {
            self.pods
                .iter()
                .position(|p| !matches!(p.owner_kind, PodOwnerKind::DaemonSet))
        });
        let Some(idx) = idx else {
            // Either no pods, or all remaining are DaemonSets.
            if let Some(ds_idx) = self.pods.iter().position(|p| {
                matches!(p.owner_kind, PodOwnerKind::DaemonSet)
            }) {
                let pod = self.pods.remove(ds_idx);
                return EvictionDecision::Skipped(pod);
            }
            return EvictionDecision::Idle;
        };

        let pod = &self.pods[idx];
        let force = pod.grace_deadline.is_some_and(|d| now >= d);

        if !force {
            // Check every PDB this pod participates in.
            for pdb in &self.pdbs {
                if !pdb.matches(pod) {
                    continue;
                }
                let baseline = *self.pdb_baseline.get(&pdb_key(pdb)).unwrap_or(&0);
                let already_evicted = *self.pdb_evicted.get(&pdb_key(pdb)).unwrap_or(&0);
                // After the proposed eviction, remaining_matching = baseline - already - 1
                let remaining_after = baseline.saturating_sub(already_evicted + 1);
                if remaining_after < pdb.min_available {
                    let reason = format!(
                        "PDB {}/{} would drop to {} (< min_available {})",
                        pdb.namespace, pdb.name, remaining_after, pdb.min_available
                    );
                    let pod_clone = pod.clone();
                    return EvictionDecision::BlockedByPdb(pod_clone, reason);
                }
            }
        }

        // Bookkeep matching PDBs.
        for pdb in &self.pdbs {
            if pdb.matches(pod) {
                *self.pdb_evicted.entry(pdb_key(pdb)).or_insert(0) += 1;
            }
        }

        let pod = self.pods.remove(idx);
        self.evicted.push(pod.clone());
        EvictionDecision::Evicted(pod)
    }

    /// Return overall progress.
    pub fn status(&self) -> DrainStatus {
        if self.pods.is_empty() {
            return DrainStatus::Complete;
        }
        // Compute how many of the remaining pods are blocked solely by PDB.
        let mut blocked = 0usize;
        for pod in &self.pods {
            if matches!(pod.owner_kind, PodOwnerKind::DaemonSet) {
                continue;
            }
            for pdb in &self.pdbs {
                if !pdb.matches(pod) {
                    continue;
                }
                let baseline = *self.pdb_baseline.get(&pdb_key(pdb)).unwrap_or(&0);
                let already_evicted = *self.pdb_evicted.get(&pdb_key(pdb)).unwrap_or(&0);
                let remaining_after = baseline.saturating_sub(already_evicted + 1);
                if remaining_after < pdb.min_available {
                    blocked += 1;
                    break;
                }
            }
        }
        DrainStatus::InProgress {
            remaining: self.pods.len(),
            blocked_by_pdb: blocked,
        }
    }

    /// Run the loop until completion or a deadline is hit. Returns the
    /// terminal status. Useful for tests and synchronous callers.
    pub fn drive_to_completion(
        &mut self,
        now: SystemTime,
        max_steps: usize,
    ) -> DrainStatus {
        for _ in 0..max_steps {
            match self.step(now) {
                EvictionDecision::Idle => break,
                EvictionDecision::BlockedByPdb(_, _) => break,
                _ => continue,
            }
        }
        self.status()
    }
}

/// Convenience for the [`crate::nodeclaim_lifecycle::drain`] caller: run
/// the loop once over `pods` (no PDBs) and return whether everything was
/// evicted. Equivalent to upstream's `force=true` evict path.
pub fn drain_all_no_pdb(pods: Vec<PodDescriptor>, now: SystemTime) -> Vec<PodDescriptor> {
    let mut plan = DrainPlan::new(pods, Vec::new());
    let _ = plan.drive_to_completion(now, 4096);
    plan.evicted_pods().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(ns: &str, name: &str, labels: &[(&str, &str)], kind: PodOwnerKind) -> PodDescriptor {
        PodDescriptor {
            namespace: ns.into(),
            name: name.into(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            owner_kind: kind,
            grace_deadline: None,
        }
    }

    #[test]
    fn no_pdb_drains_every_pod() {
        let pods = vec![
            pod("default", "a", &[], PodOwnerKind::Deployment),
            pod("default", "b", &[], PodOwnerKind::Deployment),
            pod("default", "c", &[], PodOwnerKind::Standalone),
        ];
        let evicted = drain_all_no_pdb(pods, SystemTime::now());
        assert_eq!(evicted.len(), 3);
    }

    #[test]
    fn daemonset_pods_are_never_evicted() {
        let pods = vec![
            pod("kube-system", "kube-proxy", &[], PodOwnerKind::DaemonSet),
            pod("kube-system", "cni", &[], PodOwnerKind::DaemonSet),
            pod("default", "app", &[], PodOwnerKind::Deployment),
        ];
        let mut plan = DrainPlan::new(pods, vec![]);
        let s = plan.drive_to_completion(SystemTime::now(), 16);
        assert_eq!(s, DrainStatus::Complete);
        // Only the Deployment pod is evicted.
        assert_eq!(plan.evicted_pods().len(), 1);
        assert_eq!(plan.evicted_pods()[0].name, "app");
    }

    #[test]
    fn pdb_blocks_eviction_below_min_available() {
        // 2 pods, PDB requires min_available=2 → no eviction may happen.
        let pods = vec![
            pod("default", "a", &[("app", "web")], PodOwnerKind::Deployment),
            pod("default", "b", &[("app", "web")], PodOwnerKind::Deployment),
        ];
        let pdb = PodDisruptionBudget {
            namespace: "default".into(),
            name: "web-pdb".into(),
            selector: vec![("app".into(), "web".into())],
            min_available: 2,
        };
        let mut plan = DrainPlan::new(pods, vec![pdb]);
        let decision = plan.step(SystemTime::now());
        assert!(matches!(decision, EvictionDecision::BlockedByPdb(_, _)));
        assert_eq!(plan.evicted_pods().len(), 0);
    }

    #[test]
    fn pdb_allows_partial_eviction_to_floor() {
        // 3 pods, PDB min_available=2 → exactly 1 may be evicted.
        let pods = vec![
            pod("default", "a", &[("app", "web")], PodOwnerKind::Deployment),
            pod("default", "b", &[("app", "web")], PodOwnerKind::Deployment),
            pod("default", "c", &[("app", "web")], PodOwnerKind::Deployment),
        ];
        let pdb = PodDisruptionBudget {
            namespace: "default".into(),
            name: "web-pdb".into(),
            selector: vec![("app".into(), "web".into())],
            min_available: 2,
        };
        let mut plan = DrainPlan::new(pods, vec![pdb]);
        let _ = plan.drive_to_completion(SystemTime::now(), 16);
        assert_eq!(plan.evicted_pods().len(), 1);
        match plan.status() {
            DrainStatus::InProgress {
                remaining,
                blocked_by_pdb,
            } => {
                assert_eq!(remaining, 2);
                assert_eq!(blocked_by_pdb, 2);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn grace_deadline_forces_eviction_through_pdb() {
        let mut a = pod("default", "a", &[("app", "web")], PodOwnerKind::Deployment);
        let b = pod("default", "b", &[("app", "web")], PodOwnerKind::Deployment);
        a.grace_deadline = Some(SystemTime::UNIX_EPOCH);
        let pdb = PodDisruptionBudget {
            namespace: "default".into(),
            name: "web-pdb".into(),
            selector: vec![("app".into(), "web".into())],
            min_available: 2,
        };
        let mut plan = DrainPlan::new(vec![a, b], vec![pdb]);
        let now = SystemTime::now();
        let d = plan.step(now);
        // a is past grace → force-evicted in spite of PDB.
        assert!(matches!(d, EvictionDecision::Evicted(ref p) if p.name == "a"));
    }

    #[test]
    fn drain_status_complete_when_only_daemonsets_remain() {
        let pods = vec![
            pod("kube-system", "cni", &[], PodOwnerKind::DaemonSet),
        ];
        let mut plan = DrainPlan::new(pods, vec![]);
        let _ = plan.drive_to_completion(SystemTime::now(), 4);
        // The DaemonSet pod is *skipped*, leaving the plan empty.
        assert_eq!(plan.status(), DrainStatus::Complete);
    }

    #[test]
    fn pdb_matches_only_same_namespace() {
        let pdb = PodDisruptionBudget {
            namespace: "ns-a".into(),
            name: "p".into(),
            selector: vec![("app".into(), "x".into())],
            min_available: 1,
        };
        let pa = pod("ns-a", "1", &[("app", "x")], PodOwnerKind::Deployment);
        let pb = pod("ns-b", "1", &[("app", "x")], PodOwnerKind::Deployment);
        assert!(pdb.matches(&pa));
        assert!(!pdb.matches(&pb));
    }

    #[test]
    fn multi_pdb_intersection_blocks_when_either_violated() {
        // Pod has app=web AND tier=front. PDBs select each label
        // independently; tier-pdb requires 2 of 1 → would always be
        // violated. Eviction blocked.
        let pods = vec![pod(
            "default",
            "only",
            &[("app", "web"), ("tier", "front")],
            PodOwnerKind::Deployment,
        )];
        let pdb_app = PodDisruptionBudget {
            namespace: "default".into(),
            name: "app".into(),
            selector: vec![("app".into(), "web".into())],
            min_available: 0,
        };
        let pdb_tier = PodDisruptionBudget {
            namespace: "default".into(),
            name: "tier".into(),
            selector: vec![("tier".into(), "front".into())],
            min_available: 1,
        };
        let mut plan = DrainPlan::new(pods, vec![pdb_app, pdb_tier]);
        let d = plan.step(SystemTime::now());
        assert!(matches!(d, EvictionDecision::BlockedByPdb(_, _)));
    }

    #[test]
    fn step_idle_when_no_pods() {
        let mut plan = DrainPlan::new(vec![], vec![]);
        assert!(matches!(plan.step(SystemTime::now()), EvictionDecision::Idle));
        assert_eq!(plan.status(), DrainStatus::Complete);
    }
}
