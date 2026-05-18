// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use chrono::Utc;

use crate::models::{
    ChaosExperiment, ExperimentEvent, ExperimentResult, ExperimentStatus, ExperimentType,
};

pub struct ChaosExecutor;

impl ChaosExecutor {
    pub fn new() -> Self {
        ChaosExecutor
    }

    /// Run an experiment (simulated — no real injection).
    /// Returns a completed ExperimentResult.
    pub fn execute(&self, experiment: &mut ChaosExperiment) -> ExperimentResult {
        let started_at = Utc::now();
        experiment.status = ExperimentStatus::Running;
        experiment.started_at = Some(started_at);

        if let Err(e) = self.validate(experiment) {
            let ended_at = Utc::now();
            experiment.status = ExperimentStatus::Failed;
            experiment.ended_at = Some(ended_at);
            let result = ExperimentResult {
                experiment_id: experiment.id,
                status: ExperimentStatus::Failed,
                started_at,
                ended_at: Some(ended_at),
                affected_targets: vec![],
                metrics_before: HashMap::new(),
                metrics_after: HashMap::new(),
                rollback_triggered: false,
                error: Some(e),
                events: vec![],
            };
            experiment.result = Some(result.clone());
            return result;
        }

        let (events, metrics_before, metrics_after) = simulate_experiment(experiment);
        let affected_targets: Vec<String> = events
            .iter()
            .filter_map(|e| e.target.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let ended_at = Utc::now();
        experiment.status = ExperimentStatus::Completed;
        experiment.ended_at = Some(ended_at);

        let result = ExperimentResult {
            experiment_id: experiment.id,
            status: ExperimentStatus::Completed,
            started_at,
            ended_at: Some(ended_at),
            affected_targets,
            metrics_before,
            metrics_after,
            rollback_triggered: false,
            error: None,
            events,
        };
        experiment.result = Some(result.clone());
        result
    }

    /// Validate experiment safety before running.
    pub fn validate(&self, experiment: &ChaosExperiment) -> Result<(), String> {
        let guard = &experiment.safety_guard;
        if guard.enabled {
            let ns = &experiment.target.namespace;
            if guard.protected_namespaces.iter().any(|n| n == ns) {
                return Err(format!(
                    "namespace '{}' is protected by safety guard",
                    ns
                ));
            }
        }
        let errs = crate::engine::validate_experiment(experiment);
        if !errs.is_empty() {
            return Err(errs.join("; "));
        }
        Ok(())
    }

    /// Check if experiment should be halted by safety guard.
    /// Returns `true` if the experiment should be halted.
    pub fn check_safety(&self, experiment: &ChaosExperiment, healthy_pct: f32) -> bool {
        let guard = &experiment.safety_guard;
        if !guard.enabled {
            return false;
        }
        healthy_pct < guard.min_healthy_pod_percentage
    }

    /// Simulate rollback of a running experiment.
    pub fn rollback(&self, experiment: &mut ChaosExperiment) -> ExperimentResult {
        let started_at = experiment.started_at.unwrap_or_else(Utc::now);
        let ended_at = Utc::now();
        experiment.status = ExperimentStatus::Aborted;
        experiment.ended_at = Some(ended_at);

        let rollback_event = ExperimentEvent {
            timestamp: ended_at,
            event_type: "rollback".to_string(),
            message: "Experiment rolled back by operator request".to_string(),
            target: None,
        };

        let events = experiment
            .result
            .as_ref()
            .map(|r| r.events.clone())
            .unwrap_or_default()
            .into_iter()
            .chain(std::iter::once(rollback_event))
            .collect();

        let metrics_before = experiment
            .result
            .as_ref()
            .map(|r| r.metrics_before.clone())
            .unwrap_or_default();

        let result = ExperimentResult {
            experiment_id: experiment.id,
            status: ExperimentStatus::Aborted,
            started_at,
            ended_at: Some(ended_at),
            affected_targets: vec![],
            metrics_before,
            metrics_after: HashMap::new(),
            rollback_triggered: true,
            error: None,
            events,
        };
        experiment.result = Some(result.clone());
        result
    }
}

impl Default for ChaosExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Simulate what an experiment does — generate realistic events and metrics.
fn simulate_experiment(
    exp: &ChaosExperiment,
) -> (Vec<ExperimentEvent>, HashMap<String, f64>, HashMap<String, f64>) {
    let now = Utc::now();
    let pod_name = format!(
        "{}-pod-{}",
        exp.target.namespace,
        &exp.id.to_string()[..8]
    );

    // Realistic baseline metrics
    let mut metrics_before = HashMap::new();
    metrics_before.insert("p50_latency_ms".to_string(), 12.0_f64);
    metrics_before.insert("p99_latency_ms".to_string(), 48.0_f64);
    metrics_before.insert("error_rate".to_string(), 0.002_f64);
    metrics_before.insert("cpu_util".to_string(), 0.25_f64);
    metrics_before.insert("memory_util".to_string(), 0.40_f64);

    // Degrade metrics according to experiment type
    let mut metrics_after = metrics_before.clone();
    match &exp.experiment_type {
        ExperimentType::NetworkLatency => {
            *metrics_after.get_mut("p50_latency_ms").unwrap() += 100.0;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 500.0;
        }
        ExperimentType::NetworkPacketLoss => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.15;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 200.0;
        }
        ExperimentType::NetworkCorruption => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.08;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 150.0;
        }
        ExperimentType::NetworkBandwidth => {
            *metrics_after.get_mut("p50_latency_ms").unwrap() += 50.0;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 300.0;
        }
        ExperimentType::CpuStress => {
            *metrics_after.get_mut("cpu_util").unwrap() += 0.70;
            let v = metrics_after["cpu_util"].min(1.0_f64);
            metrics_after.insert("cpu_util".to_string(), v);
        }
        ExperimentType::MemoryStress => {
            *metrics_after.get_mut("memory_util").unwrap() += 0.40;
            let v = metrics_after["memory_util"].min(1.0_f64);
            metrics_after.insert("memory_util".to_string(), v);
        }
        ExperimentType::PodKill => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.05;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 200.0;
        }
        ExperimentType::DiskFill => {
            *metrics_after.get_mut("p50_latency_ms").unwrap() += 30.0;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 120.0;
        }
        ExperimentType::IoLatency | ExperimentType::IoChaos => {
            *metrics_after.get_mut("p50_latency_ms").unwrap() += 80.0;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 400.0;
        }
        ExperimentType::ProcessKill => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.10;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 250.0;
        }
        ExperimentType::NodeDrain => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.03;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 100.0;
            *metrics_after.get_mut("cpu_util").unwrap() += 0.15;
        }
        ExperimentType::ClockSkew => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.02;
        }
        ExperimentType::HttpFault => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.20;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 50.0;
        }
        ExperimentType::GrpcFault => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.18;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 60.0;
        }
        ExperimentType::JvmException => {
            *metrics_after.get_mut("error_rate").unwrap() += 0.25;
            *metrics_after.get_mut("p99_latency_ms").unwrap() += 80.0;
        }
    }

    let experiment_label = format!("{:?}", exp.experiment_type);

    let events = vec![
        ExperimentEvent {
            timestamp: now,
            event_type: "target_selected".to_string(),
            message: format!("Target selected: {}", pod_name),
            target: Some(pod_name.clone()),
        },
        ExperimentEvent {
            timestamp: now,
            event_type: "metrics_collected".to_string(),
            message: "Baseline metrics collected before injection".to_string(),
            target: Some(pod_name.clone()),
        },
        ExperimentEvent {
            timestamp: now,
            event_type: "injection_started".to_string(),
            message: format!("Fault injection started: {}", experiment_label),
            target: Some(pod_name.clone()),
        },
        ExperimentEvent {
            timestamp: now,
            event_type: "metrics_observed".to_string(),
            message: format!(
                "Observed degradation — p99 latency: {:.0}ms, error_rate: {:.3}",
                metrics_after["p99_latency_ms"], metrics_after["error_rate"]
            ),
            target: Some(pod_name.clone()),
        },
        ExperimentEvent {
            timestamp: now,
            event_type: "injection_stopped".to_string(),
            message: "Fault injection stopped, system recovering".to_string(),
            target: Some(pod_name),
        },
    ];

    (events, metrics_before, metrics_after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BlastRadius, ChaosTarget, ExperimentParams, SafetyGuard};
    use std::collections::HashMap;
    use uuid::Uuid;
    use chrono::Utc;

    fn make_experiment(exp_type: ExperimentType, namespace: &str, duration_secs: u32) -> ChaosExperiment {
        ChaosExperiment {
            id: Uuid::new_v4(),
            name: "executor-test".to_string(),
            experiment_type: exp_type,
            target: ChaosTarget {
                namespace: namespace.to_string(),
                selector: HashMap::new(),
                pod_count: Some(1),
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
            duration_secs,
            blast_radius: BlastRadius::default(),
            safety_guard: SafetyGuard::default(),
            result: None,
            annotations: HashMap::new(),
        }
    }

    #[test]
    fn test_execute_completes_experiment() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::NetworkLatency, "staging", 60);
        let result = executor.execute(&mut exp);
        assert_eq!(result.status, ExperimentStatus::Completed);
        assert!(!result.events.is_empty());
        assert!(!result.metrics_before.is_empty());
        assert!(!result.metrics_after.is_empty());
        assert_eq!(exp.status, ExperimentStatus::Completed);
    }

    #[test]
    fn test_execute_fails_on_protected_namespace() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::PodKill, "kube-system", 60);
        let result = executor.execute(&mut exp);
        assert_eq!(result.status, ExperimentStatus::Failed);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_execute_sets_started_at() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::NetworkLatency, "staging", 60);
        executor.execute(&mut exp);
        assert!(exp.started_at.is_some());
        assert!(exp.ended_at.is_some());
    }

    #[test]
    fn test_execute_stores_result_on_experiment() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::CpuStress, "staging", 30);
        exp.parameters.cpu_load_percent = Some(80);
        executor.execute(&mut exp);
        assert!(exp.result.is_some());
    }

    #[test]
    fn test_network_latency_degrades_p99() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::NetworkLatency, "staging", 60);
        let result = executor.execute(&mut exp);
        let before = result.metrics_before["p99_latency_ms"];
        let after = result.metrics_after["p99_latency_ms"];
        assert!(after > before, "p99 latency should increase after network latency injection");
    }

    #[test]
    fn test_cpu_stress_degrades_cpu_util() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::CpuStress, "staging", 60);
        exp.parameters.cpu_load_percent = Some(90);
        let result = executor.execute(&mut exp);
        let before = result.metrics_before["cpu_util"];
        let after = result.metrics_after["cpu_util"];
        assert!(after > before);
    }

    #[test]
    fn test_rollback_sets_aborted() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::PodKill, "staging", 60);
        exp.status = ExperimentStatus::Running;
        exp.started_at = Some(Utc::now());
        let result = executor.rollback(&mut exp);
        assert_eq!(result.status, ExperimentStatus::Aborted);
        assert!(result.rollback_triggered);
        assert_eq!(exp.status, ExperimentStatus::Aborted);
    }

    #[test]
    fn test_validate_rejects_protected_namespace() {
        let executor = ChaosExecutor::new();
        let exp = make_experiment(ExperimentType::PodKill, "kube-system", 60);
        assert!(executor.validate(&exp).is_err());
    }

    #[test]
    fn test_validate_accepts_staging() {
        let executor = ChaosExecutor::new();
        let exp = make_experiment(ExperimentType::NetworkLatency, "staging", 60);
        assert!(executor.validate(&exp).is_ok());
    }

    #[test]
    fn test_check_safety_halts_below_threshold() {
        let executor = ChaosExecutor::new();
        let exp = make_experiment(ExperimentType::PodKill, "staging", 30);
        assert!(executor.check_safety(&exp, 0.3)); // below 50% threshold → halt
    }

    #[test]
    fn test_check_safety_ok_above_threshold() {
        let executor = ChaosExecutor::new();
        let exp = make_experiment(ExperimentType::PodKill, "staging", 30);
        assert!(!executor.check_safety(&exp, 0.9)); // above threshold → do not halt
    }

    #[test]
    fn test_check_safety_disabled_guard() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::PodKill, "staging", 30);
        exp.safety_guard.enabled = false;
        assert!(!executor.check_safety(&exp, 0.0)); // guard disabled → never halt
    }

    #[test]
    fn test_pod_kill_increases_error_rate() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::PodKill, "staging", 60);
        let result = executor.execute(&mut exp);
        assert!(result.metrics_after["error_rate"] > result.metrics_before["error_rate"]);
    }

    #[test]
    fn test_http_fault_increases_error_rate() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::HttpFault, "staging", 60);
        let result = executor.execute(&mut exp);
        assert!(result.metrics_after["error_rate"] > result.metrics_before["error_rate"]);
    }

    #[test]
    fn test_events_have_required_fields() {
        let executor = ChaosExecutor::new();
        let mut exp = make_experiment(ExperimentType::NetworkLatency, "staging", 60);
        let result = executor.execute(&mut exp);
        for event in &result.events {
            assert!(!event.event_type.is_empty());
            assert!(!event.message.is_empty());
        }
    }
}
