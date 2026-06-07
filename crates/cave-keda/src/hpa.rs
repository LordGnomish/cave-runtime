// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HPA generation — KEDA extends the Horizontal Pod Autoscaler by emitting an
//! HPA object whose external metrics are fed by the ScaledObject's triggers.
//! upstream: kedacore/keda v2.16.1
//!   controllers/keda/hpa.go              (newHPAForScaledObject, getHPAName)
//!   apis/keda/v1alpha1/scaledobject_types.go (GetHPAMinReplicas/MaxReplicas)
//!   pkg/scalers/scaler.go                (GenerateMetricNameWithIndex, *InMili)
//!
//! This ports the pure spec-generation transform. Applying the resulting HPA
//! to the cluster stays with cave-controller-manager (the HPA reconcile owner).

use crate::scaledobject::ScaledObject;

// scaledobject_types.go default replica bounds.
const DEFAULT_HPA_MIN_REPLICAS: i32 = 1;
const DEFAULT_HPA_MAX_REPLICAS: i32 = 100;

/// External-metric target type. KEDA's GetMetricTargetType yields AverageValue
/// when no explicit metricType is set (Utilization is rejected for external).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpaMetricTargetType {
    AverageValue,
    Value,
}

/// `autoscalingv2.CrossVersionObjectReference` — the scale target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleTargetRef {
    pub name: String,
    pub kind: String,
    pub api_version: String,
}

/// A ScaledObject trigger reduced to what the HPA external-metric spec needs.
#[derive(Debug, Clone)]
pub struct HpaTrigger {
    pub trigger_index: i32,
    pub metric_name: String,
    pub target_value: f64,
}

/// One `autoscalingv2.MetricSpec` of type External.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalMetricSpec {
    pub metric_name: String,
    pub target_type: HpaMetricTargetType,
    /// target.averageValue as a milli-quantity (value * 1000).
    pub target_average_value_milli: i64,
}

/// The generated `autoscalingv2.HorizontalPodAutoscaler` spec (subset).
#[derive(Debug, Clone, PartialEq)]
pub struct HpaSpec {
    pub name: String,
    pub namespace: String,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub metrics: Vec<ExternalMetricSpec>,
    pub scale_target_ref: ScaleTargetRef,
}

/// `getDefaultHpaName` — `keda-hpa-<scaledObjectName>`.
pub fn default_hpa_name(scaled_object_name: &str) -> String {
    format!("keda-hpa-{scaled_object_name}")
}

/// `getHPAName` — the advanced override when non-empty, else the default.
pub fn hpa_name(scaled_object_name: &str, advanced_override: &str) -> String {
    if !advanced_override.is_empty() {
        return advanced_override.to_string();
    }
    default_hpa_name(scaled_object_name)
}

/// `GetHPAMinReplicas` — MinReplicaCount when set and > 0, else default 1.
pub fn hpa_min_replicas(min_replica_count: Option<i32>) -> i32 {
    match min_replica_count {
        Some(m) if m > 0 => m,
        _ => DEFAULT_HPA_MIN_REPLICAS,
    }
}

/// `GetHPAMaxReplicas` — MaxReplicaCount when set, else default 100.
pub fn hpa_max_replicas(max_replica_count: Option<i32>) -> i32 {
    match max_replica_count {
        Some(m) => m,
        None => DEFAULT_HPA_MAX_REPLICAS,
    }
}

/// `GenerateMetricNameWithIndex` — `s%d-%s`.
pub fn generate_metric_name_with_index(trigger_index: i32, metric_name: &str) -> String {
    format!("s{trigger_index}-{metric_name}")
}

/// `GenerateMetricInMili` / `GetMetricTargetMili` — value * 1000 as int64.
pub fn generate_metric_in_mili(value: f64) -> i64 {
    (value * 1000.0) as i64
}

/// Port of `newHPAForScaledObject` (+ `getScaledObjectMetricSpecs`): build the
/// HPA spec from a ScaledObject, its scale target, and its triggers.
///
/// `paused_replica_count` mirrors `executor.GetPausedReplicaCount` — when set,
/// MinReplicas == MaxReplicas == count, and a 0 is lifted to 1 (HPA min can't
/// be 0).
pub fn build_hpa(
    scaled_object_name: &str,
    advanced_hpa_name: &str,
    namespace: &str,
    scaled_object: &ScaledObject,
    scale_target_ref: ScaleTargetRef,
    triggers: &[HpaTrigger],
    paused_replica_count: Option<i32>,
) -> HpaSpec {
    let mut min_replicas = hpa_min_replicas(scaled_object.min_replica_count);
    let mut max_replicas = hpa_max_replicas(scaled_object.max_replica_count);

    if let Some(mut paused) = paused_replica_count {
        // MinReplicas on HPA can't be 0
        if paused == 0 {
            paused = 1;
        }
        min_replicas = paused;
        max_replicas = paused;
    }

    let metrics = triggers
        .iter()
        .map(|t| ExternalMetricSpec {
            metric_name: generate_metric_name_with_index(t.trigger_index, &t.metric_name),
            target_type: HpaMetricTargetType::AverageValue,
            target_average_value_milli: generate_metric_in_mili(t.target_value),
        })
        .collect();

    HpaSpec {
        name: hpa_name(scaled_object_name, advanced_hpa_name),
        namespace: namespace.to_string(),
        min_replicas,
        max_replicas,
        metrics,
        scale_target_ref,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ScaleTargetRef {
        ScaleTargetRef {
            name: "my-deploy".into(),
            kind: "Deployment".into(),
            api_version: "apps/v1".into(),
        }
    }

    #[test]
    fn default_name_prefixes_keda_hpa() {
        assert_eq!(default_hpa_name("orders"), "keda-hpa-orders");
    }

    #[test]
    fn hpa_name_uses_override_when_present() {
        assert_eq!(hpa_name("orders", "custom-hpa"), "custom-hpa");
        assert_eq!(hpa_name("orders", ""), "keda-hpa-orders");
    }

    #[test]
    fn min_replicas_defaults_and_zero_floor() {
        assert_eq!(hpa_min_replicas(None), 1);
        assert_eq!(hpa_min_replicas(Some(0)), 1); // 0 is not > 0 → default 1
        assert_eq!(hpa_min_replicas(Some(3)), 3);
    }

    #[test]
    fn max_replicas_defaults_to_100() {
        assert_eq!(hpa_max_replicas(None), 100);
        assert_eq!(hpa_max_replicas(Some(5)), 5);
        assert_eq!(hpa_max_replicas(Some(0)), 0); // explicit 0 is honored
    }

    #[test]
    fn metric_name_has_trigger_index_prefix() {
        assert_eq!(
            generate_metric_name_with_index(0, "kafka-mytopic"),
            "s0-kafka-mytopic"
        );
        assert_eq!(
            generate_metric_name_with_index(2, "prometheus-http_requests"),
            "s2-prometheus-http_requests"
        );
    }

    #[test]
    fn metric_in_mili_scales_by_1000() {
        assert_eq!(generate_metric_in_mili(1.5), 1500);
        assert_eq!(generate_metric_in_mili(5.0), 5000);
        assert_eq!(generate_metric_in_mili(0.0), 0);
    }

    #[test]
    fn build_hpa_full_spec() {
        let mut so = ScaledObject::new("t");
        so.min_replica_count = Some(2);
        so.max_replica_count = Some(10);
        let triggers = vec![
            HpaTrigger { trigger_index: 0, metric_name: "kafka-topic".into(), target_value: 5.0 },
            HpaTrigger { trigger_index: 1, metric_name: "prometheus".into(), target_value: 100.0 },
        ];
        let hpa = build_hpa("orders", "", "default", &so, target(), &triggers, None);

        assert_eq!(hpa.name, "keda-hpa-orders");
        assert_eq!(hpa.namespace, "default");
        assert_eq!(hpa.min_replicas, 2);
        assert_eq!(hpa.max_replicas, 10);
        assert_eq!(hpa.scale_target_ref.kind, "Deployment");
        assert_eq!(hpa.scale_target_ref.api_version, "apps/v1");
        assert_eq!(hpa.metrics.len(), 2);
        assert_eq!(hpa.metrics[0].metric_name, "s0-kafka-topic");
        assert_eq!(hpa.metrics[0].target_type, HpaMetricTargetType::AverageValue);
        assert_eq!(hpa.metrics[0].target_average_value_milli, 5000);
        assert_eq!(hpa.metrics[1].metric_name, "s1-prometheus");
        assert_eq!(hpa.metrics[1].target_average_value_milli, 100_000);
    }

    #[test]
    fn build_hpa_uses_defaults_when_counts_absent() {
        let so = ScaledObject::new("t");
        let hpa = build_hpa("api", "", "ns", &so, target(), &[], None);
        assert_eq!(hpa.min_replicas, 1);
        assert_eq!(hpa.max_replicas, 100);
        assert!(hpa.metrics.is_empty());
    }

    #[test]
    fn paused_pins_min_and_max_and_lifts_zero() {
        let mut so = ScaledObject::new("t");
        so.min_replica_count = Some(2);
        so.max_replica_count = Some(10);
        // paused at 3 → min == max == 3
        let hpa = build_hpa("api", "", "ns", &so, target(), &[], Some(3));
        assert_eq!(hpa.min_replicas, 3);
        assert_eq!(hpa.max_replicas, 3);
        // paused at 0 → lifted to 1 (HPA min can't be 0)
        let hpa0 = build_hpa("api", "", "ns", &so, target(), &[], Some(0));
        assert_eq!(hpa0.min_replicas, 1);
        assert_eq!(hpa0.max_replicas, 1);
    }
}
