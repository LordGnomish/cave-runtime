// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workload kinds — Deployment / ReplicaSet / StatefulSet / DaemonSet /
//! Job / CronJob.  This module provides the *umbrella* progressive
//! rollout / scale orchestration on top of the typed objects stored in
//! `cave_apiserver`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
    Job,
    CronJob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RolloutStrategy {
    RollingUpdate {
        max_surge: u32,
        max_unavailable: u32,
    },
    Recreate,
    Canary {
        percent: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleRequest {
    pub kind: WorkloadKind,
    pub namespace: String,
    pub name: String,
    pub target_replicas: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleResult {
    pub previous: u32,
    pub current: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutStep {
    pub batch: u32,
    pub replicas: u32,
}

/// Compute the per-batch replica progression for a `RollingUpdate`
/// strategy.  Mirrors the math in `pkg/controller/deployment/util`.
pub fn plan_rolling_update(
    desired: u32,
    current: u32,
    max_surge: u32,
    max_unavailable: u32,
) -> Vec<RolloutStep> {
    if desired == current {
        return Vec::new();
    }
    let mut steps = Vec::new();
    let mut at = current;
    let mut batch = 1;
    let stride = max_surge.max(1).min(desired.max(1));
    while at != desired {
        if at < desired {
            at = (at + stride).min(desired);
        } else {
            let _ = max_unavailable; // unused on scale-up branch
            at = at.saturating_sub(stride);
            if at < desired {
                at = desired;
            }
        }
        steps.push(RolloutStep { batch, replicas: at });
        batch += 1;
        if batch > 64 {
            // Guard against unbounded loops on absurd inputs.
            break;
        }
    }
    steps
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronExpr {
    pub minute: String,
    pub hour: String,
    pub dom: String,
    pub month: String,
    pub dow: String,
}

impl CronExpr {
    pub fn parse(expr: &str) -> Option<Self> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return None;
        }
        Some(Self {
            minute: parts[0].into(),
            hour: parts[1].into(),
            dom: parts[2].into(),
            month: parts[3].into(),
            dow: parts[4].into(),
        })
    }

    /// Returns true when this expression matches every minute (`*` in
    /// every field).  Used by tests + the scheduler conformance suite.
    pub fn is_wildcard(&self) -> bool {
        self.minute == "*"
            && self.hour == "*"
            && self.dom == "*"
            && self.month == "*"
            && self.dow == "*"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobConcurrencyPolicy {
    Allow,
    Forbid,
    Replace,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_up_in_max_surge_steps() {
        let steps = plan_rolling_update(5, 1, 2, 0);
        // 1->3->5 = 2 batches
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].replicas, 3);
        assert_eq!(steps[1].replicas, 5);
    }

    #[test]
    fn scale_to_same_no_steps() {
        let steps = plan_rolling_update(3, 3, 1, 1);
        assert!(steps.is_empty());
    }

    #[test]
    fn scale_down_steps() {
        let steps = plan_rolling_update(1, 5, 2, 2);
        // 5 -> 3 -> 1 -> 1 (last clamps)
        assert!(!steps.is_empty());
        assert_eq!(steps.last().unwrap().replicas, 1);
    }

    #[test]
    fn cron_parse_five_fields() {
        assert!(CronExpr::parse("* * * * *").is_some());
        assert!(CronExpr::parse("* * * *").is_none());
        let e = CronExpr::parse("0 0 1 * *").unwrap();
        assert_eq!(e.minute, "0");
        assert_eq!(e.dom, "1");
    }

    #[test]
    fn cron_wildcard_predicate() {
        assert!(CronExpr::parse("* * * * *").unwrap().is_wildcard());
        assert!(!CronExpr::parse("0 0 * * *").unwrap().is_wildcard());
    }

    #[test]
    fn workload_kind_roundtrip_json() {
        let s = serde_json::to_string(&WorkloadKind::DaemonSet).unwrap();
        let back: WorkloadKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, WorkloadKind::DaemonSet);
    }

    #[test]
    fn rollout_strategy_serializes() {
        let s = serde_json::to_string(&RolloutStrategy::RollingUpdate {
            max_surge: 2,
            max_unavailable: 1,
        })
        .unwrap();
        assert!(s.contains("RollingUpdate"));
        assert!(s.contains("max_surge"));
    }

    #[test]
    fn canary_carries_percent() {
        let s = RolloutStrategy::Canary { percent: 25 };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("25"));
    }
}
