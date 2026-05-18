// SPDX-License-Identifier: AGPL-3.0-or-later
//! HPA metric source typing — `pkg/api/autoscaling/v2/types.go`.
//!
//! The autoscaling/v2 API has five metric source kinds. Each carries a
//! different shape of `target` and a different per-pod / per-object
//! denominator. This module models them as Rust enums and provides
//! `desired_for_source` helpers that translate raw readings into a desired
//! replica count using the formulas in `replica_calculator.go`.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetType {
    /// Utilization — average per-pod resource usage as % of `requests`.
    Utilization,
    /// AverageValue — average per-pod metric value (e.g. RPS).
    AverageValue,
    /// Value — single absolute reading (used by Object metrics).
    Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricSource {
    Resource {
        name: String, // "cpu" / "memory"
        target_type: TargetType,
        target: u64,
    },
    ContainerResource {
        name: String,
        container: String,
        target_type: TargetType,
        target: u64,
    },
    Pods {
        metric: String,
        target: u64, // averageValue
    },
    Object {
        metric: String,
        described_object_kind: String,
        described_object_name: String,
        target_type: TargetType,
        target: u64,
    },
    External {
        metric: String,
        match_labels: Vec<(String, String)>,
        target_type: TargetType,
        target: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricReading {
    /// For Resource/ContainerResource/Pods: per-pod readings.
    /// For Object/External: a single value (use first element).
    pub values: Vec<u64>,
    /// Number of replicas the readings correspond to. Required for
    /// utilization-style proportional scaling.
    pub current_replicas: u32,
}

/// Compute desired replicas from a single source. Mirrors the dispatch in
/// `pkg/controller/podautoscaler/replica_calculator.go::GetMetricReplicas`.
pub fn desired_for_source(
    source: &MetricSource,
    reading: &MetricReading,
) -> Result<u32, ControllerError> {
    match source {
        MetricSource::Resource { target_type, target, .. }
        | MetricSource::ContainerResource { target_type, target, .. } => {
            resource_desired(reading, *target_type, *target)
        }
        MetricSource::Pods { target, .. } => pods_desired(reading, *target),
        MetricSource::Object { target_type, target, .. } => object_desired(reading, *target_type, *target),
        MetricSource::External { target_type, target, .. } => object_desired(reading, *target_type, *target),
    }
}

fn resource_desired(reading: &MetricReading, ttype: TargetType, target: u64) -> Result<u32, ControllerError> {
    let count = reading.values.len() as u64;
    if count == 0 || target == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "no readings or target=0".into(),
        });
    }
    let sum: u64 = reading.values.iter().copied().sum();
    let avg = sum / count;
    match ttype {
        TargetType::Utilization | TargetType::AverageValue => {
            // desired = ceil(current * avg / target)
            let cur = reading.current_replicas.max(1) as u64;
            let desired = (cur * avg + target - 1) / target;
            Ok(desired as u32)
        }
        TargetType::Value => Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "Value targetType not allowed for resource metric".into(),
        }),
    }
}

fn pods_desired(reading: &MetricReading, target: u64) -> Result<u32, ControllerError> {
    let count = reading.values.len() as u64;
    if count == 0 || target == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "no readings or target=0".into(),
        });
    }
    let sum: u64 = reading.values.iter().copied().sum();
    let avg = sum / count;
    let cur = reading.current_replicas.max(1) as u64;
    let desired = (cur * avg + target - 1) / target;
    Ok(desired as u32)
}

fn object_desired(reading: &MetricReading, ttype: TargetType, target: u64) -> Result<u32, ControllerError> {
    if reading.values.is_empty() || target == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "no value or target=0".into(),
        });
    }
    let v = reading.values[0];
    let cur = reading.current_replicas.max(1) as u64;
    match ttype {
        // Object/External Value: desired = ceil(current * v / target).
        TargetType::Value => Ok(((cur * v + target - 1) / target) as u32),
        // Object/External AverageValue: target is per-pod; desired = ceil(v / target).
        TargetType::AverageValue => Ok(((v + target - 1) / target) as u32),
        TargetType::Utilization => Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "Utilization targetType not allowed for Object/External metric".into(),
        }),
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/replica_calculator.go",
    "GetMetricReplicas",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn rdg(values: Vec<u64>, cur: u32) -> MetricReading {
        MetricReading { values, current_replicas: cur }
    }

    #[test]
    fn resource_utilization_drives_proportional_scale() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-src-cpu"
        );
        let s = MetricSource::Resource {
            name: "cpu".into(),
            target_type: TargetType::Utilization,
            target: 50,
        };
        // Avg=100, target=50, current=4 → ceil(4*100/50)=8.
        assert_eq!(
            desired_for_source(&s, &rdg(vec![100, 100, 100, 100], 4)).unwrap(),
            8
        );
    }

    #[test]
    fn pods_metric_uses_average_value() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetMetricReplicas",
            "tenant-hpa-src-pods"
        );
        let s = MetricSource::Pods { metric: "rps".into(), target: 200 };
        // Avg=400, target=200, current=2 → ceil(2*400/200)=4.
        assert_eq!(desired_for_source(&s, &rdg(vec![400, 400], 2)).unwrap(), 4);
    }

    #[test]
    fn object_value_scales_against_single_reading() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetObjectMetricReplicas",
            "tenant-hpa-src-obj-value"
        );
        let s = MetricSource::Object {
            metric: "qps".into(),
            described_object_kind: "Service".into(),
            described_object_name: "web".into(),
            target_type: TargetType::Value,
            target: 100,
        };
        // v=400, target=100, current=2 → ceil(2*400/100)=8.
        assert_eq!(desired_for_source(&s, &rdg(vec![400], 2)).unwrap(), 8);
    }

    #[test]
    fn object_average_value_does_not_multiply_by_current() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetObjectMetricReplicas",
            "tenant-hpa-src-obj-avg"
        );
        let s = MetricSource::Object {
            metric: "qps".into(),
            described_object_kind: "Service".into(),
            described_object_name: "web".into(),
            target_type: TargetType::AverageValue,
            target: 200,
        };
        // v=800, target=200 → ceil(800/200)=4.
        assert_eq!(desired_for_source(&s, &rdg(vec![800], 2)).unwrap(), 4);
    }

    #[test]
    fn external_metric_uses_object_formula() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetExternalMetricReplicas",
            "tenant-hpa-src-external"
        );
        let s = MetricSource::External {
            metric: "queue_depth".into(),
            match_labels: vec![("region".into(), "us-east".into())],
            target_type: TargetType::Value,
            target: 50,
        };
        assert_eq!(desired_for_source(&s, &rdg(vec![200], 4)).unwrap(), 16);
    }

    #[test]
    fn container_resource_uses_resource_formula() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetContainerResourceReplicas",
            "tenant-hpa-src-container"
        );
        let s = MetricSource::ContainerResource {
            name: "cpu".into(),
            container: "main".into(),
            target_type: TargetType::Utilization,
            target: 80,
        };
        // Avg=160, target=80, current=2 → 4.
        assert_eq!(desired_for_source(&s, &rdg(vec![160, 160], 2)).unwrap(), 4);
    }

    #[test]
    fn zero_target_is_rejected_in_every_source() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetMetricReplicas",
            "tenant-hpa-src-zero"
        );
        let s = MetricSource::Resource {
            name: "cpu".into(),
            target_type: TargetType::Utilization,
            target: 0,
        };
        assert!(desired_for_source(&s, &rdg(vec![100], 1)).is_err());
    }

    #[test]
    fn empty_readings_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetMetricReplicas",
            "tenant-hpa-src-empty"
        );
        let s = MetricSource::Pods { metric: "rps".into(), target: 100 };
        assert!(desired_for_source(&s, &rdg(vec![], 4)).is_err());
    }

    #[test]
    fn resource_value_targettype_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/autoscaling/v2/validation.go",
            "validateMetricSpec",
            "tenant-hpa-src-bad-type"
        );
        let s = MetricSource::Resource {
            name: "cpu".into(),
            target_type: TargetType::Value,
            target: 100,
        };
        assert!(desired_for_source(&s, &rdg(vec![100], 1)).is_err());
    }

    #[test]
    fn metric_source_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/autoscaling/v2/types.go",
            "MetricSpec",
            "tenant-hpa-src-serde"
        );
        let s = MetricSource::Pods { metric: "rps".into(), target: 100 };
        let bytes = serde_json::to_string(&s).unwrap();
        let back: MetricSource = serde_json::from_str(&bytes).unwrap();
        match (s, back) {
            (MetricSource::Pods { metric: a, target: ta }, MetricSource::Pods { metric: b, target: tb }) => {
                assert_eq!(a, b);
                assert_eq!(ta, tb);
            }
            _ => panic!("variant mismatch"),
        }
    }
}
