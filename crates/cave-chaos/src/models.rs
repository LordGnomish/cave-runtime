// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentType {
    NetworkLatency,
    NetworkPacketLoss,
    CpuStress,
    MemoryStress,
    PodKill,
    DiskFill,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChaosTarget {
    pub namespace: String,
    pub selector: HashMap<String, String>,
    pub pod_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperimentParams {
    pub latency_ms: Option<u32>,
    pub packet_loss_percent: Option<f32>,
    pub cpu_load_percent: Option<u8>,
    pub memory_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Draft,
    Running,
    Completed,
    Aborted,
    Failed,
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
}
