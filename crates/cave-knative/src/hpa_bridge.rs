// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HPA-direct integration — `pkg/autoscaler/hpa`.
//!
//! Knative supports two autoscaler classes:
//! 1. KPA (Knative Pod Autoscaler) — concurrency / RPS driven
//! 2. HPA (Horizontal Pod Autoscaler) — CPU / memory driven, native
//!    Kubernetes HPA CRD
//!
//! When a Revision is annotated `autoscaling.knative.dev/class: hpa.autoscaling.knative.dev`,
//! the upstream knative-autoscaler-hpa controller renders an
//! `autoscaling/v2.HorizontalPodAutoscaler` CR pointing at the revision's
//! Deployment.
//!
//! This module ports the HPA-class predicate + the CRD-shaped renderer.
//! cave-controller-manager owns runtime HPA reconciliation; cave-knative
//! emits the desired-state HPA spec.

use crate::revision::Revision;
use serde::{Deserialize, Serialize};

pub const ANNOTATION_CLASS: &str = "autoscaling.knative.dev/class";
pub const ANNOTATION_METRIC: &str = "autoscaling.knative.dev/metric";
pub const ANNOTATION_TARGET: &str = "autoscaling.knative.dev/target";
pub const ANNOTATION_MIN_SCALE: &str = "autoscaling.knative.dev/min-scale";
pub const ANNOTATION_MAX_SCALE: &str = "autoscaling.knative.dev/max-scale";

pub const CLASS_HPA: &str = "hpa.autoscaling.knative.dev";
pub const CLASS_KPA: &str = "kpa.autoscaling.knative.dev";

/// `pkg/autoscaler/hpa.classFromAnnotations` — true when the revision opts into HPA.
pub fn is_hpa_class(annotations: &std::collections::HashMap<String, String>) -> bool {
    annotations
        .get(ANNOTATION_CLASS)
        .map(|s| s == CLASS_HPA)
        .unwrap_or(false)
}

/// Metric kind picked by the `autoscaling.knative.dev/metric` annotation.
/// HPA supports cpu and memory upstream; everything else falls through to KPA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpaMetricKind {
    Cpu,
    Memory,
}

impl HpaMetricKind {
    pub fn from_annotation(s: &str) -> Option<Self> {
        match s {
            "cpu" => Some(Self::Cpu),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }
    pub fn resource_name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
        }
    }
}

/// Cluster-scoped HPA `MetricSpec` shape (subset).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HpaMetric {
    #[serde(rename = "type")]
    pub kind: String, // "Resource" upstream — Knative-HPA only emits Resource metrics.
    pub resource: HpaMetricResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HpaMetricResource {
    pub name: String,
    pub target: HpaMetricTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HpaMetricTarget {
    #[serde(rename = "type")]
    pub kind: String, // "Utilization" — Knative renders averageUtilization.
    pub average_utilization: u32,
}

/// `autoscaling/v2.HorizontalPodAutoscalerSpec` (subset).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HpaSpec {
    pub scale_target_ref: ScaleTargetRef,
    pub min_replicas: u32,
    pub max_replicas: u32,
    pub metrics: Vec<HpaMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScaleTargetRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HorizontalPodAutoscaler {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub spec: HpaSpec,
}

fn parse_u32(s: &str, default: u32) -> u32 {
    s.parse().unwrap_or(default)
}

/// Render the HPA CR for a revision that opted into the HPA class. Returns
/// `None` if the revision is not HPA-class.
pub fn render_hpa(rev: &Revision) -> Option<HorizontalPodAutoscaler> {
    if !is_hpa_class(&rev.metadata.annotations) {
        return None;
    }
    let metric_str = rev
        .metadata
        .annotations
        .get(ANNOTATION_METRIC)
        .map(String::as_str)
        .unwrap_or("cpu");
    let metric_kind = HpaMetricKind::from_annotation(metric_str).unwrap_or(HpaMetricKind::Cpu);
    let target = rev
        .metadata
        .annotations
        .get(ANNOTATION_TARGET)
        .map(String::as_str)
        .map(|s| parse_u32(s, 80))
        .unwrap_or(80);
    let min = rev
        .metadata
        .annotations
        .get(ANNOTATION_MIN_SCALE)
        .map(String::as_str)
        .map(|s| parse_u32(s, 1))
        .unwrap_or(1);
    let max = rev
        .metadata
        .annotations
        .get(ANNOTATION_MAX_SCALE)
        .map(String::as_str)
        .map(|s| parse_u32(s, 10))
        .unwrap_or(10);
    let max = max.max(min);

    Some(HorizontalPodAutoscaler {
        api_version: "autoscaling/v2".into(),
        kind: "HorizontalPodAutoscaler".into(),
        name: rev.metadata.name.clone(),
        namespace: rev.metadata.namespace.clone(),
        spec: HpaSpec {
            scale_target_ref: ScaleTargetRef {
                api_version: "apps/v1".into(),
                kind: "Deployment".into(),
                name: rev.metadata.name.clone(),
            },
            min_replicas: min,
            max_replicas: max,
            metrics: vec![HpaMetric {
                kind: "Resource".into(),
                resource: HpaMetricResource {
                    name: metric_kind.resource_name().into(),
                    target: HpaMetricTarget {
                        kind: "Utilization".into(),
                        average_utilization: target,
                    },
                },
            }],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::ObjectMeta;
    use crate::revision::{Revision, RevisionSpec};
    use std::collections::HashMap;

    fn mk_rev(annots: &[(&str, &str)]) -> Revision {
        let mut a = HashMap::new();
        for (k, v) in annots {
            a.insert((*k).to_string(), (*v).to_string());
        }
        Revision {
            metadata: ObjectMeta {
                name: "rev-1".into(),
                namespace: "ns".into(),
                labels: HashMap::new(),
                annotations: a,
                generation: 0,
            },
            spec: RevisionSpec::default(),
            status: Default::default(),
        }
    }

    #[test]
    fn is_hpa_class_true_when_annotated() {
        let r = mk_rev(&[(ANNOTATION_CLASS, CLASS_HPA)]);
        assert!(is_hpa_class(&r.metadata.annotations));
    }

    #[test]
    fn is_hpa_class_false_when_kpa() {
        let r = mk_rev(&[(ANNOTATION_CLASS, CLASS_KPA)]);
        assert!(!is_hpa_class(&r.metadata.annotations));
    }

    #[test]
    fn is_hpa_class_false_when_missing() {
        let r = mk_rev(&[]);
        assert!(!is_hpa_class(&r.metadata.annotations));
    }

    #[test]
    fn render_hpa_returns_none_for_kpa_class() {
        let r = mk_rev(&[(ANNOTATION_CLASS, CLASS_KPA)]);
        assert!(render_hpa(&r).is_none());
    }

    #[test]
    fn render_hpa_returns_default_target_for_cpu_metric() {
        let r = mk_rev(&[(ANNOTATION_CLASS, CLASS_HPA)]);
        let hpa = render_hpa(&r).unwrap();
        assert_eq!(hpa.spec.min_replicas, 1);
        assert_eq!(hpa.spec.max_replicas, 10);
        assert_eq!(hpa.spec.metrics.len(), 1);
        let m = &hpa.spec.metrics[0];
        assert_eq!(m.resource.name, "cpu");
        assert_eq!(m.resource.target.average_utilization, 80);
    }

    #[test]
    fn render_hpa_uses_memory_target_when_annotated() {
        let r = mk_rev(&[
            (ANNOTATION_CLASS, CLASS_HPA),
            (ANNOTATION_METRIC, "memory"),
            (ANNOTATION_TARGET, "65"),
            (ANNOTATION_MIN_SCALE, "2"),
            (ANNOTATION_MAX_SCALE, "20"),
        ]);
        let hpa = render_hpa(&r).unwrap();
        assert_eq!(hpa.spec.min_replicas, 2);
        assert_eq!(hpa.spec.max_replicas, 20);
        assert_eq!(hpa.spec.metrics[0].resource.name, "memory");
        assert_eq!(hpa.spec.metrics[0].resource.target.average_utilization, 65);
    }

    #[test]
    fn render_hpa_clamps_max_below_min() {
        let r = mk_rev(&[
            (ANNOTATION_CLASS, CLASS_HPA),
            (ANNOTATION_MIN_SCALE, "5"),
            (ANNOTATION_MAX_SCALE, "2"),
        ]);
        let hpa = render_hpa(&r).unwrap();
        assert_eq!(hpa.spec.min_replicas, 5);
        assert_eq!(hpa.spec.max_replicas, 5);
    }

    #[test]
    fn render_hpa_targets_revision_deployment() {
        let r = mk_rev(&[(ANNOTATION_CLASS, CLASS_HPA)]);
        let hpa = render_hpa(&r).unwrap();
        assert_eq!(hpa.spec.scale_target_ref.kind, "Deployment");
        assert_eq!(hpa.spec.scale_target_ref.name, "rev-1");
        assert_eq!(hpa.namespace, "ns");
    }
}
