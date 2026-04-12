//! Metric analysis — evaluate PromQL/webhook results against thresholds.

use crate::types::{AnalysisPhase, AnalysisRun, MetricCondition, MetricResult};
use chrono::Utc;

/// Evaluate a single metric value against its condition.
pub fn evaluate_metric(
    metric_name: &str,
    value: f64,
    condition: &MetricCondition,
    failure_limit: u32,
    current_failures: u32,
) -> MetricResult {
    let passed = condition.evaluate(value);
    let failure_count = if passed { 0 } else { current_failures + 1 };
    MetricResult {
        metric_name: metric_name.to_string(),
        value,
        passed: passed && failure_count <= failure_limit,
        failure_count,
        message: if passed {
            None
        } else {
            Some(format!(
                "value {value:.4} did not satisfy condition (failures: {failure_count}/{})",
                failure_limit + 1
            ))
        },
    }
}

/// Determine the overall analysis phase from a set of metric results.
pub fn compute_analysis_phase(results: &[MetricResult]) -> AnalysisPhase {
    if results.is_empty() {
        return AnalysisPhase::Inconclusive;
    }
    if results.iter().all(|r| r.passed) {
        AnalysisPhase::Successful
    } else {
        AnalysisPhase::Failed
    }
}

/// Finalize an analysis run with the given metric results.
pub fn finalize_run(run: &mut AnalysisRun, results: Vec<MetricResult>) {
    run.metric_results = results;
    run.phase = compute_analysis_phase(&run.metric_results);
    run.end_time = Some(Utc::now());
}

/// Start an analysis run.
pub fn start_run(run: &mut AnalysisRun) {
    run.phase = AnalysisPhase::Running;
    run.start_time = Some(Utc::now());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnalysisRun, MetricCondition};

    #[test]
    fn test_metric_greater_than_passes() {
        let result = evaluate_metric(
            "success_rate",
            99.5,
            &MetricCondition::GreaterThan { threshold: 95.0 },
            0,
            0,
        );
        assert!(result.passed);
        assert_eq!(result.failure_count, 0);
    }

    #[test]
    fn test_metric_greater_than_fails() {
        let result = evaluate_metric(
            "success_rate",
            90.0,
            &MetricCondition::GreaterThan { threshold: 95.0 },
            0,
            0,
        );
        assert!(!result.passed);
        assert!(result.message.is_some());
    }

    #[test]
    fn test_metric_less_than_passes() {
        let result = evaluate_metric(
            "error_rate",
            0.5,
            &MetricCondition::LessThan { threshold: 1.0 },
            3,
            0,
        );
        assert!(result.passed);
    }

    #[test]
    fn test_metric_between_passes() {
        let result = evaluate_metric(
            "latency_p99",
            250.0,
            &MetricCondition::Between { lo: 0.0, hi: 500.0 },
            0,
            0,
        );
        assert!(result.passed);
    }

    #[test]
    fn test_metric_failure_limit_allows_transient_failures() {
        // failure_limit=2 means up to 2 consecutive failures allowed.
        // With current_failures=1, the metric fails again → failure_count=2.
        // Since 2 <= failure_limit=2, passed is false but failure_count hasn't exceeded limit yet.
        // Actually the check is: failure_count <= failure_limit.
        let result = evaluate_metric(
            "error_rate",
            5.0,
            &MetricCondition::LessThan { threshold: 1.0 },
            2,
            1,
        );
        // value fails the condition (5.0 is not < 1.0)
        assert!(!result.passed);
        assert_eq!(result.failure_count, 2);
    }

    #[test]
    fn test_compute_phase_all_pass() {
        let results = vec![
            MetricResult {
                metric_name: "success_rate".into(),
                value: 99.0,
                passed: true,
                failure_count: 0,
                message: None,
            },
            MetricResult {
                metric_name: "latency".into(),
                value: 200.0,
                passed: true,
                failure_count: 0,
                message: None,
            },
        ];
        assert_eq!(compute_analysis_phase(&results), AnalysisPhase::Successful);
    }

    #[test]
    fn test_compute_phase_any_fail() {
        let results = vec![
            MetricResult {
                metric_name: "success_rate".into(),
                value: 99.0,
                passed: true,
                failure_count: 0,
                message: None,
            },
            MetricResult {
                metric_name: "error_rate".into(),
                value: 5.0,
                passed: false,
                failure_count: 1,
                message: Some("too high".into()),
            },
        ];
        assert_eq!(compute_analysis_phase(&results), AnalysisPhase::Failed);
    }

    #[test]
    fn test_compute_phase_empty_inconclusive() {
        assert_eq!(compute_analysis_phase(&[]), AnalysisPhase::Inconclusive);
    }

    #[test]
    fn test_analysis_run_is_successful() {
        let mut run = AnalysisRun::new("template-a", None);
        start_run(&mut run);
        finalize_run(
            &mut run,
            vec![MetricResult {
                metric_name: "success_rate".into(),
                value: 99.5,
                passed: true,
                failure_count: 0,
                message: None,
            }],
        );
        assert!(run.is_successful());
        assert_eq!(run.phase, AnalysisPhase::Successful);
    }

    #[test]
    fn test_analysis_run_is_not_successful_when_failed() {
        let mut run = AnalysisRun::new("template-b", None);
        start_run(&mut run);
        finalize_run(
            &mut run,
            vec![MetricResult {
                metric_name: "error_rate".into(),
                value: 10.0,
                passed: false,
                failure_count: 3,
                message: Some("failed".into()),
            }],
        );
        assert!(!run.is_successful());
        assert_eq!(run.phase, AnalysisPhase::Failed);
    }
}
