// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Eviction manager — node-pressure + API-initiated eviction.
//!
//! Mirrors `pkg/kubelet/eviction`.  cave-k8s tracks pressure signals
//! (memory, nodefs.available, imagefs.available) and computes the
//! sorted eviction candidate list when a threshold is breached.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressureSignal {
    MemoryAvailable,
    NodeFsAvailable,
    ImageFsAvailable,
    PidsAvailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PressureThreshold {
    pub signal: PressureSignal,
    /// Absolute threshold in bytes for the *Available* signals; the
    /// kubelet evicts when the value falls *below* this number.
    pub minimum_available_bytes: u64,
    pub grace_period_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PressureObservation {
    pub signal: PressureSignal,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvictionCandidate {
    pub namespace: String,
    pub name: String,
    /// Larger = more pressure consumption.
    pub usage: u64,
    /// Priority class value; lower priority is evicted first.
    pub priority: i32,
}

pub fn rank_candidates(mut candidates: Vec<EvictionCandidate>) -> Vec<EvictionCandidate> {
    candidates.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.usage.cmp(&a.usage)));
    candidates
}

pub fn under_pressure(
    threshold: &PressureThreshold,
    observation: &PressureObservation,
) -> bool {
    threshold.signal == observation.signal
        && observation.available_bytes < threshold.minimum_available_bytes
}

/// Compute the eviction plan: ranked candidates restricted by signal.
pub fn plan_eviction(
    thresholds: &[PressureThreshold],
    observations: &[PressureObservation],
    candidates: Vec<EvictionCandidate>,
) -> Vec<EvictionCandidate> {
    let mut triggered = false;
    for t in thresholds {
        for o in observations {
            if under_pressure(t, o) {
                triggered = true;
            }
        }
    }
    if !triggered {
        return Vec::new();
    }
    rank_candidates(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(ns: &str, name: &str, usage: u64, prio: i32) -> EvictionCandidate {
        EvictionCandidate {
            namespace: ns.into(),
            name: name.into(),
            usage,
            priority: prio,
        }
    }

    #[test]
    fn rank_lowest_priority_first() {
        let r = rank_candidates(vec![
            cand("a", "p1", 100, 1000),
            cand("a", "p2", 50, 0),
            cand("a", "p3", 75, 0),
        ]);
        assert_eq!(r[0].name, "p3"); // priority 0 + larger usage
        assert_eq!(r[1].name, "p2");
        assert_eq!(r[2].name, "p1");
    }

    #[test]
    fn under_pressure_when_below_threshold() {
        let t = PressureThreshold {
            signal: PressureSignal::MemoryAvailable,
            minimum_available_bytes: 500,
            grace_period_seconds: 0,
        };
        let o = PressureObservation {
            signal: PressureSignal::MemoryAvailable,
            available_bytes: 400,
        };
        assert!(under_pressure(&t, &o));
    }

    #[test]
    fn no_pressure_when_above_threshold() {
        let t = PressureThreshold {
            signal: PressureSignal::MemoryAvailable,
            minimum_available_bytes: 500,
            grace_period_seconds: 0,
        };
        let o = PressureObservation {
            signal: PressureSignal::MemoryAvailable,
            available_bytes: 600,
        };
        assert!(!under_pressure(&t, &o));
    }

    #[test]
    fn signal_mismatch_no_pressure() {
        let t = PressureThreshold {
            signal: PressureSignal::MemoryAvailable,
            minimum_available_bytes: 500,
            grace_period_seconds: 0,
        };
        let o = PressureObservation {
            signal: PressureSignal::NodeFsAvailable,
            available_bytes: 0,
        };
        assert!(!under_pressure(&t, &o));
    }

    #[test]
    fn plan_eviction_empty_when_no_pressure() {
        let t = vec![PressureThreshold {
            signal: PressureSignal::MemoryAvailable,
            minimum_available_bytes: 500,
            grace_period_seconds: 0,
        }];
        let o = vec![PressureObservation {
            signal: PressureSignal::MemoryAvailable,
            available_bytes: 1000,
        }];
        let plan = plan_eviction(&t, &o, vec![cand("a", "p1", 1, 0)]);
        assert!(plan.is_empty());
    }

    #[test]
    fn plan_eviction_ranks_when_triggered() {
        let t = vec![PressureThreshold {
            signal: PressureSignal::MemoryAvailable,
            minimum_available_bytes: 500,
            grace_period_seconds: 0,
        }];
        let o = vec![PressureObservation {
            signal: PressureSignal::MemoryAvailable,
            available_bytes: 100,
        }];
        let plan = plan_eviction(
            &t,
            &o,
            vec![
                cand("a", "p1", 100, 1000),
                cand("a", "p2", 50, 0),
            ],
        );
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].name, "p2");
    }
}
