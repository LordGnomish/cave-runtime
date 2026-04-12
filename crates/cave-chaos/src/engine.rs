use crate::models::{ChaosExperiment, ExperimentStatus, ExperimentType};

/// Validate experiment parameters based on type
pub fn validate_experiment(experiment: &ChaosExperiment) -> Vec<String> {
    let mut errors = vec![];
    match &experiment.experiment_type {
        ExperimentType::NetworkLatency => {
            if experiment.parameters.latency_ms.is_none() {
                errors.push("latency_ms required for NetworkLatency".to_string());
            }
        }
        ExperimentType::NetworkPacketLoss => {
            if let Some(loss) = experiment.parameters.packet_loss_percent {
                if loss > 100.0 {
                    errors.push("packet_loss_percent must be 0-100".to_string());
                }
            } else {
                errors.push("packet_loss_percent required for NetworkPacketLoss".to_string());
            }
        }
        ExperimentType::CpuStress => {
            if let Some(cpu) = experiment.parameters.cpu_load_percent {
                if cpu > 100 {
                    errors.push("cpu_load_percent must be 0-100".to_string());
                }
            } else {
                errors.push("cpu_load_percent required for CpuStress".to_string());
            }
        }
        _ => {}
    }
    if experiment.duration_secs == 0 {
        errors.push("duration_secs must be > 0".to_string());
    }
    errors
}

/// Check if experiment is currently active
pub fn is_active(experiment: &ChaosExperiment) -> bool {
    experiment.status == ExperimentStatus::Running
}

/// Calculate duration if completed
pub fn actual_duration_secs(experiment: &ChaosExperiment) -> Option<i64> {
    match (experiment.started_at, experiment.ended_at) {
        (Some(start), Some(end)) => Some((end - start).num_seconds()),
        _ => None,
    }
}

/// Risk assessment: high-risk experiments target production namespace
pub fn is_high_risk(experiment: &ChaosExperiment) -> bool {
    experiment.target.namespace == "production" || experiment.target.namespace == "prod"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChaosTarget, ExperimentParams};
    use std::collections::HashMap;
    use uuid::Uuid;
    use chrono::{Duration, Utc};

    fn make_experiment(
        exp_type: ExperimentType,
        params: ExperimentParams,
        status: ExperimentStatus,
        namespace: &str,
        duration_secs: u32,
    ) -> ChaosExperiment {
        ChaosExperiment {
            id: Uuid::new_v4(),
            name: "test-exp".to_string(),
            experiment_type: exp_type,
            target: ChaosTarget {
                namespace: namespace.to_string(),
                selector: HashMap::new(),
                pod_count: None,
            },
            parameters: params,
            status,
            created_at: Utc::now(),
            started_at: None,
            ended_at: None,
            duration_secs,
        }
    }

    fn empty_params() -> ExperimentParams {
        ExperimentParams {
            latency_ms: None,
            packet_loss_percent: None,
            cpu_load_percent: None,
            memory_mb: None,
        }
    }

    #[test]
    fn test_validate_latency_missing_param() {
        let exp = make_experiment(
            ExperimentType::NetworkLatency,
            empty_params(),
            ExperimentStatus::Draft,
            "staging",
            60,
        );
        let errors = validate_experiment(&exp);
        assert!(errors.iter().any(|e| e.contains("latency_ms")));
    }

    #[test]
    fn test_validate_latency_valid() {
        let params = ExperimentParams { latency_ms: Some(100), ..empty_params() };
        let exp = make_experiment(
            ExperimentType::NetworkLatency,
            params,
            ExperimentStatus::Draft,
            "staging",
            60,
        );
        let errors = validate_experiment(&exp);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_zero_duration() {
        let params = ExperimentParams { latency_ms: Some(100), ..empty_params() };
        let exp = make_experiment(
            ExperimentType::NetworkLatency,
            params,
            ExperimentStatus::Draft,
            "staging",
            0,
        );
        let errors = validate_experiment(&exp);
        assert!(errors.iter().any(|e| e.contains("duration_secs")));
    }

    #[test]
    fn test_is_active_running() {
        let exp = make_experiment(
            ExperimentType::PodKill,
            empty_params(),
            ExperimentStatus::Running,
            "staging",
            30,
        );
        assert!(is_active(&exp));
    }

    #[test]
    fn test_is_active_not_running() {
        let draft = make_experiment(
            ExperimentType::PodKill,
            empty_params(),
            ExperimentStatus::Draft,
            "staging",
            30,
        );
        let completed = make_experiment(
            ExperimentType::PodKill,
            empty_params(),
            ExperimentStatus::Completed,
            "staging",
            30,
        );
        assert!(!is_active(&draft));
        assert!(!is_active(&completed));
    }

    #[test]
    fn test_is_high_risk_production() {
        let exp = make_experiment(
            ExperimentType::PodKill,
            empty_params(),
            ExperimentStatus::Draft,
            "production",
            30,
        );
        assert!(is_high_risk(&exp));
        let prod_exp = make_experiment(
            ExperimentType::PodKill,
            empty_params(),
            ExperimentStatus::Draft,
            "prod",
            30,
        );
        assert!(is_high_risk(&prod_exp));
    }

    #[test]
    fn test_is_high_risk_staging() {
        let exp = make_experiment(
            ExperimentType::PodKill,
            empty_params(),
            ExperimentStatus::Draft,
            "staging",
            30,
        );
        assert!(!is_high_risk(&exp));
    }

    #[test]
    fn test_packet_loss_over_100() {
        let params = ExperimentParams {
            packet_loss_percent: Some(150.0),
            ..empty_params()
        };
        let exp = make_experiment(
            ExperimentType::NetworkPacketLoss,
            params,
            ExperimentStatus::Draft,
            "staging",
            60,
        );
        let errors = validate_experiment(&exp);
        assert!(errors.iter().any(|e| e.contains("packet_loss_percent")));
    }

    #[test]
    fn test_actual_duration_secs() {
        let now = Utc::now();
        let mut exp = make_experiment(
            ExperimentType::CpuStress,
            ExperimentParams { cpu_load_percent: Some(80), ..empty_params() },
            ExperimentStatus::Completed,
            "staging",
            60,
        );
        exp.started_at = Some(now);
        exp.ended_at = Some(now + Duration::seconds(45));
        assert_eq!(actual_duration_secs(&exp), Some(45));
    }
}
