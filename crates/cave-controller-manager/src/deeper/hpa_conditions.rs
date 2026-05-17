// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HPA status conditions — `pkg/controller/podautoscaler/horizontal.go`.
//!
//! The HPA controller writes three conditions onto each HPA's status:
//!
//! * `AbleToScale` — controller can communicate with the scale subresource.
//! * `ScalingActive` — at least one metric source produced a recommendation.
//! * `ScalingLimited` — the recommendation hit min/max replicas, or a
//!   behavior policy capped it.
//!
//! Mirrors `setCondition` and `recordConditionHelper`.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConditionType {
    AbleToScale,
    ScalingActive,
    ScalingLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpaCondition {
    pub kind: ConditionType,
    pub status: ConditionStatus,
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConditionSet {
    conditions: Vec<HpaCondition>,
}

impl ConditionSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, kind: ConditionType) -> Option<&HpaCondition> {
        self.conditions.iter().find(|c| c.kind == kind)
    }

    pub fn set(&mut self, c: HpaCondition) {
        if let Some(slot) = self.conditions.iter_mut().find(|x| x.kind == c.kind) {
            *slot = c;
        } else {
            self.conditions.push(c);
        }
    }

    pub fn len(&self) -> usize {
        self.conditions.len()
    }
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &HpaCondition> {
        self.conditions.iter()
    }

    /// Convenience: derive the conditions from a single reconciliation outcome.
    pub fn from_reconcile(
        able_to_scale: bool,
        scaling_active: bool,
        limited: Option<&'static str>,
    ) -> Self {
        let mut s = Self::new();
        s.set(HpaCondition {
            kind: ConditionType::AbleToScale,
            status: if able_to_scale { ConditionStatus::True } else { ConditionStatus::False },
            reason: if able_to_scale { "SucceededGetScale".into() } else { "FailedGetScale".into() },
            message: String::new(),
        });
        s.set(HpaCondition {
            kind: ConditionType::ScalingActive,
            status: if scaling_active { ConditionStatus::True } else { ConditionStatus::False },
            reason: if scaling_active { "ValidMetricFound".into() } else { "FailedGetMetric".into() },
            message: String::new(),
        });
        s.set(HpaCondition {
            kind: ConditionType::ScalingLimited,
            status: if limited.is_some() { ConditionStatus::True } else { ConditionStatus::False },
            reason: limited.unwrap_or("DesiredWithinRange").into(),
            message: String::new(),
        });
        s
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/horizontal.go",
    "setCondition",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn empty_set_returns_none() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "setCondition",
            "tenant-hpa-cond-empty"
        );
        let s = ConditionSet::new();
        assert!(s.get(ConditionType::AbleToScale).is_none());
    }

    #[test]
    fn set_inserts_new_condition() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "setCondition",
            "tenant-hpa-cond-insert"
        );
        let mut s = ConditionSet::new();
        s.set(HpaCondition {
            kind: ConditionType::AbleToScale,
            status: ConditionStatus::True,
            reason: "ok".into(),
            message: String::new(),
        });
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn set_replaces_existing_condition_of_same_kind() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "setCondition",
            "tenant-hpa-cond-replace"
        );
        let mut s = ConditionSet::new();
        for status in [ConditionStatus::True, ConditionStatus::False] {
            s.set(HpaCondition {
                kind: ConditionType::AbleToScale,
                status,
                reason: "x".into(),
                message: String::new(),
            });
        }
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(ConditionType::AbleToScale).unwrap().status, ConditionStatus::False);
    }

    #[test]
    fn from_reconcile_sets_three_conditions() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "setCondition",
            "tenant-hpa-cond-three"
        );
        let s = ConditionSet::from_reconcile(true, true, None);
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn limited_reason_propagates_into_scaling_limited() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "setCondition",
            "tenant-hpa-cond-limited"
        );
        let s = ConditionSet::from_reconcile(true, true, Some("TooManyReplicas"));
        let c = s.get(ConditionType::ScalingLimited).unwrap();
        assert_eq!(c.status, ConditionStatus::True);
        assert_eq!(c.reason, "TooManyReplicas");
    }

    #[test]
    fn able_to_scale_false_when_scale_fetch_fails() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "reconcileAutoscaler",
            "tenant-hpa-cond-no-scale"
        );
        let s = ConditionSet::from_reconcile(false, false, None);
        assert_eq!(
            s.get(ConditionType::AbleToScale).unwrap().status,
            ConditionStatus::False
        );
        assert_eq!(
            s.get(ConditionType::AbleToScale).unwrap().reason,
            "FailedGetScale"
        );
    }

    #[test]
    fn condition_round_trips_serde() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "HPACondition",
            "tenant-hpa-cond-serde"
        );
        let c = HpaCondition {
            kind: ConditionType::ScalingActive,
            status: ConditionStatus::True,
            reason: "ValidMetricFound".into(),
            message: "all sources reporting".into(),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: HpaCondition = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn iter_yields_in_insertion_order() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "setCondition",
            "tenant-hpa-cond-order"
        );
        let s = ConditionSet::from_reconcile(true, true, None);
        let kinds: Vec<_> = s.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ConditionType::AbleToScale,
                ConditionType::ScalingActive,
                ConditionType::ScalingLimited,
            ]
        );
    }
}
