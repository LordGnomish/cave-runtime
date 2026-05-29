// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A chaos experiment definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChaosExperiment {
    pub id: Uuid,
    pub name: String,
    pub experiment_type: ExperimentType,
    pub target: ChaosTarget,
    pub parameters: ExperimentParams,
    pub status: ExperimentStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_secs: u32,
    /// Blast radius limits how many pods can be affected.
    pub blast_radius: BlastRadius,
    /// Safety guard prevents experiments from running in protected namespaces.
    pub safety_guard: SafetyGuard,
    /// Result is populated when the experiment completes or fails.
    pub result: Option<ExperimentResult>,
    /// Free-form annotations (key-value metadata).
    pub annotations: HashMap<String, String>,
}

/// Blast radius control — limits how many pods are affected by an experiment.
/// Maps to Chaos Mesh's `spec.selector.mode` + pod count limiting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlastRadius {
    /// Maximum fraction of matched pods to affect (0.0 – 1.0). Default: 0.5.
    pub max_pod_fraction: f32,
    /// Hard cap on absolute pod count (overrides fraction if smaller).
    pub max_pods: Option<usize>,
    /// Restrict injection to only these namespaces (empty = all matched namespaces).
    pub namespaces: Vec<String>,
}

impl Default for BlastRadius {
    fn default() -> Self {
        BlastRadius {
            max_pod_fraction: 0.5,
            max_pods: None,
            namespaces: vec![],
        }
    }
}

/// Safety guard — blocks experiments in protected namespaces and halts when
/// pod health drops below a minimum threshold.
/// Maps to Chaos Mesh's `spec.selector.namespaces` exclusion + Grafana liveness gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafetyGuard {
    /// Whether the safety guard is active. Default: true.
    pub enabled: bool,
    /// Namespaces that can never be targeted. Default: kube-system, kube-public, cave-system.
    pub protected_namespaces: Vec<String>,
    /// Minimum fraction of healthy pods required to continue. Default: 0.5.
    pub min_healthy_pod_percentage: f32,
}

impl Default for SafetyGuard {
    fn default() -> Self {
        SafetyGuard {
            enabled: true,
            protected_namespaces: vec![
                "kube-system".to_string(),
                "kube-public".to_string(),
                "cave-system".to_string(),
            ],
            min_healthy_pod_percentage: 0.5,
        }
    }
}

/// All supported fault injection types.
/// Extends the original 6-type enum with the full Chaos Mesh surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentType {
    // Network
    NetworkLatency,
    NetworkPacketLoss,
    NetworkCorruption,
    NetworkBandwidth,
    NetworkPartition,
    // Compute
    CpuStress,
    MemoryStress,
    // Disk / I/O
    DiskFill,
    IoLatency,
    IoChaos,
    // Process / Pod
    PodKill,
    ProcessKill,
    // Node
    NodeDrain,
    // Time
    ClockSkew,
    // Application-layer
    HttpFault,
    GrpcFault,
    JvmException,
}

/// Selector for the pods/nodes to target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChaosTarget {
    pub namespace: String,
    pub selector: HashMap<String, String>,
    pub pod_count: Option<usize>,
}

/// Experiment parameters — union of all fault types' tunable knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentParams {
    /// Added network latency in milliseconds.
    pub latency_ms: Option<u32>,
    /// Packet loss percentage (0 – 100).
    pub packet_loss_percent: Option<f32>,
    /// CPU load percentage (0 – 100).
    pub cpu_load_percent: Option<u8>,
    /// Memory to consume in MB.
    pub memory_mb: Option<u32>,
}

/// Lifecycle status of an experiment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Draft,
    Running,
    Completed,
    Aborted,
    Failed,
}

/// A structured event emitted during experiment execution.
/// Maps to Chaos Mesh's experiment event records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub message: String,
    pub target: Option<String>,
}

/// The outcome of a completed/aborted/failed experiment.
/// Maps to Chaos Mesh's experiment result CRD status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentResult {
    pub experiment_id: Uuid,
    pub status: ExperimentStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Pod/node identifiers that were affected.
    pub affected_targets: Vec<String>,
    /// Snapshot of key metrics before injection.
    pub metrics_before: HashMap<String, f64>,
    /// Snapshot of key metrics after injection.
    pub metrics_after: HashMap<String, f64>,
    /// Whether an automatic rollback was triggered during the experiment.
    pub rollback_triggered: bool,
    /// Non-None only when status == Failed.
    pub error: Option<String>,
    /// Ordered list of events emitted during the experiment.
    pub events: Vec<ExperimentEvent>,
}

/// Cron-based schedule for repeating an experiment automatically.
/// Maps to Chaos Mesh's Schedule CRD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentSchedule {
    pub id: Uuid,
    pub experiment_id: Uuid,
    /// Standard cron expression (5-field).
    pub cron_expression: String,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    /// Maximum number of runs (None = unlimited).
    pub max_runs: Option<u32>,
    pub run_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_experiment() -> ChaosExperiment {
        ChaosExperiment {
            id: Uuid::new_v4(),
            name: "latency-test".to_string(),
            experiment_type: ExperimentType::NetworkLatency,
            target: ChaosTarget {
                namespace: "staging".to_string(),
                selector: {
                    let mut m = HashMap::new();
                    m.insert("app".to_string(), "frontend".to_string());
                    m
                },
                pod_count: Some(2),
            },
            parameters: ExperimentParams {
                latency_ms: Some(100),
                packet_loss_percent: None,
                cpu_load_percent: None,
                memory_mb: None,
            },
            status: ExperimentStatus::Draft,
            created_at: Utc::now(),
            started_at: None,
            ended_at: None,
            duration_secs: 60,
            blast_radius: BlastRadius::default(),
            safety_guard: SafetyGuard::default(),
            result: None,
            annotations: HashMap::new(),
        }
    }

    #[test]
    fn test_experiment_serialization_roundtrip() {
        let exp = make_experiment();
        let json = serde_json::to_string(&exp).unwrap();
        let back: ChaosExperiment = serde_json::from_str(&json).unwrap();
        assert_eq!(exp, back);
    }

    #[test]
    fn test_experiment_type_serialization() {
        let json = serde_json::to_string(&ExperimentType::NetworkLatency).unwrap();
        assert_eq!(json, "\"network_latency\"");
    }

    #[test]
    fn test_experiment_status_serialization() {
        let statuses = vec![
            ExperimentStatus::Draft,
            ExperimentStatus::Running,
            ExperimentStatus::Completed,
            ExperimentStatus::Aborted,
            ExperimentStatus::Failed,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: ExperimentStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_chaos_target_no_pod_count() {
        let target = ChaosTarget {
            namespace: "prod".to_string(),
            selector: HashMap::new(),
            pod_count: None,
        };
        let json = serde_json::to_string(&target).unwrap();
        let back: ChaosTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pod_count, None);
    }

    #[test]
    fn test_experiment_params_all_none() {
        let params = ExperimentParams {
            latency_ms: None,
            packet_loss_percent: None,
            cpu_load_percent: None,
            memory_mb: None,
        };
        let json = serde_json::to_string(&params).unwrap();
        let back: ExperimentParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, params);
    }

    #[test]
    fn test_blast_radius_default() {
        let br = BlastRadius::default();
        assert!((br.max_pod_fraction - 0.5).abs() < 1e-6);
        assert!(br.max_pods.is_none());
        assert!(br.namespaces.is_empty());
    }

    #[test]
    fn test_safety_guard_default_protects_kube_system() {
        let sg = SafetyGuard::default();
        assert!(sg.enabled);
        assert!(sg.protected_namespaces.contains(&"kube-system".to_string()));
    }
}
