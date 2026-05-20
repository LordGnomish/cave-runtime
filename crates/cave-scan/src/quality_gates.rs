// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Quality gates & profiles — parity with
//! `server/sonar-webserver-qualitygate/src/main/java/org/sonar/server/qualitygate/`
//! (SonarQube v10.4.1).
//!
//! Evaluate a project's scan result against a Quality Gate. Each gate
//! has conditions on metrics (issue counts by severity, coverage,
//! duplications). If any condition fails on a non-warning operator the
//! gate is reported `ERROR`; warnings yield `WARN`.

use crate::models::{Finding, FindingSeverity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GateMetric {
    NewIssues,
    NewCriticalIssues,
    NewMajorIssues,
    NewMinorIssues,
    CoveragePct,
    DuplicatedLinesPct,
    SecurityHotspotsReviewedPct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GateOperator {
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
    Equal,
    NotEqual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum GateLevel {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateCondition {
    pub metric: GateMetric,
    pub op: GateOperator,
    pub threshold: f64,
    /// When true, condition failure yields WARN instead of ERROR.
    #[serde(default)]
    pub warning_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QualityGate {
    pub name: String,
    pub conditions: Vec<GateCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateEvaluation {
    pub gate: String,
    pub status: GateLevel,
    pub failures: Vec<GateConditionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateConditionResult {
    pub metric: GateMetric,
    pub op: GateOperator,
    pub threshold: f64,
    pub actual: f64,
    pub status: GateLevel,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectMetrics {
    pub new_issues_total: u64,
    pub new_critical: u64,
    pub new_major: u64,
    pub new_minor: u64,
    pub coverage_pct: f64,
    pub duplicated_lines_pct: f64,
    pub security_hotspots_reviewed_pct: f64,
}

impl ProjectMetrics {
    /// Build metrics from a slice of findings + coverage stats.
    pub fn from_findings(findings: &[Finding], coverage_pct: f64) -> Self {
        let mut critical = 0;
        let mut major = 0;
        let mut minor = 0;
        for f in findings {
            match f.severity {
                FindingSeverity::Critical => critical += 1,
                FindingSeverity::Major => major += 1,
                FindingSeverity::Minor => minor += 1,
                FindingSeverity::Info => {}
            }
        }
        Self {
            new_issues_total: (critical + major + minor) as u64,
            new_critical: critical,
            new_major: major,
            new_minor: minor,
            coverage_pct,
            duplicated_lines_pct: 0.0,
            security_hotspots_reviewed_pct: 100.0,
        }
    }

    fn value_for(&self, metric: &GateMetric) -> f64 {
        match metric {
            GateMetric::NewIssues => self.new_issues_total as f64,
            GateMetric::NewCriticalIssues => self.new_critical as f64,
            GateMetric::NewMajorIssues => self.new_major as f64,
            GateMetric::NewMinorIssues => self.new_minor as f64,
            GateMetric::CoveragePct => self.coverage_pct,
            GateMetric::DuplicatedLinesPct => self.duplicated_lines_pct,
            GateMetric::SecurityHotspotsReviewedPct => self.security_hotspots_reviewed_pct,
        }
    }
}

fn satisfies(actual: f64, op: &GateOperator, threshold: f64) -> bool {
    match op {
        GateOperator::GreaterThan => actual > threshold,
        GateOperator::LessThan => actual < threshold,
        GateOperator::GreaterOrEqual => actual >= threshold,
        GateOperator::LessOrEqual => actual <= threshold,
        GateOperator::Equal => (actual - threshold).abs() < f64::EPSILON,
        GateOperator::NotEqual => (actual - threshold).abs() >= f64::EPSILON,
    }
}

pub fn evaluate(gate: &QualityGate, metrics: &ProjectMetrics) -> GateEvaluation {
    let mut failures = Vec::new();
    let mut worst = GateLevel::Ok;
    for cond in &gate.conditions {
        let actual = metrics.value_for(&cond.metric);
        // Sonar semantics: the condition operator describes the *failure*
        // threshold — e.g. `coverage < 80` means "fail if coverage is
        // less than 80". So a failure is detected when `satisfies` is true.
        if satisfies(actual, &cond.op, cond.threshold) {
            let status = if cond.warning_only {
                GateLevel::Warn
            } else {
                GateLevel::Error
            };
            if matches!(status, GateLevel::Error)
                || (matches!(status, GateLevel::Warn) && matches!(worst, GateLevel::Ok))
            {
                worst = status.clone();
            }
            failures.push(GateConditionResult {
                metric: cond.metric.clone(),
                op: cond.op.clone(),
                threshold: cond.threshold,
                actual,
                status,
            });
        }
    }
    GateEvaluation {
        gate: gate.name.clone(),
        status: worst,
        failures,
    }
}

/// Sonar's built-in "Sonar way" gate — the default that ships with v10.
/// Conditions describe failure thresholds (the gate trips when the
/// metric value `op`s the threshold, e.g. `coverage < 80`).
pub fn sonar_way() -> QualityGate {
    QualityGate {
        name: "Sonar way".into(),
        conditions: vec![
            GateCondition {
                metric: GateMetric::NewCriticalIssues,
                op: GateOperator::GreaterThan,
                threshold: 0.0,
                warning_only: false,
            },
            GateCondition {
                metric: GateMetric::CoveragePct,
                op: GateOperator::LessThan,
                threshold: 80.0,
                warning_only: false,
            },
            GateCondition {
                metric: GateMetric::DuplicatedLinesPct,
                op: GateOperator::GreaterThan,
                threshold: 3.0,
                warning_only: true,
            },
            GateCondition {
                metric: GateMetric::SecurityHotspotsReviewedPct,
                op: GateOperator::LessThan,
                threshold: 100.0,
                warning_only: false,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn finding(sev: FindingSeverity) -> Finding {
        Finding {
            id: Uuid::nil(),
            rule_id: Uuid::nil(),
            rule_name: "r".into(),
            file_path: "f.rs".into(),
            line_number: 1,
            matched_text: String::new(),
            severity: sev,
            message: String::new(),
        }
    }

    #[test]
    fn metrics_from_findings_counts_each_bucket() {
        let m = ProjectMetrics::from_findings(
            &[
                finding(FindingSeverity::Critical),
                finding(FindingSeverity::Critical),
                finding(FindingSeverity::Major),
                finding(FindingSeverity::Minor),
                finding(FindingSeverity::Info),
            ],
            85.0,
        );
        assert_eq!(m.new_critical, 2);
        assert_eq!(m.new_major, 1);
        assert_eq!(m.new_minor, 1);
        assert_eq!(m.new_issues_total, 4);
        assert_eq!(m.coverage_pct, 85.0);
    }

    #[test]
    fn sonar_way_passes_when_clean() {
        let m = ProjectMetrics::from_findings(&[], 85.0);
        let e = evaluate(&sonar_way(), &m);
        assert_eq!(e.status, GateLevel::Ok);
        assert!(e.failures.is_empty());
    }

    #[test]
    fn sonar_way_errors_on_new_critical() {
        let m = ProjectMetrics::from_findings(&[finding(FindingSeverity::Critical)], 85.0);
        let e = evaluate(&sonar_way(), &m);
        assert_eq!(e.status, GateLevel::Error);
        assert!(e.failures.iter().any(|f| f.metric == GateMetric::NewCriticalIssues));
    }

    #[test]
    fn sonar_way_errors_on_low_coverage() {
        let m = ProjectMetrics::from_findings(&[], 70.0);
        let e = evaluate(&sonar_way(), &m);
        assert_eq!(e.status, GateLevel::Error);
        assert!(e.failures.iter().any(|f| f.metric == GateMetric::CoveragePct));
    }

    #[test]
    fn warning_only_does_not_escalate_to_error() {
        let gate = QualityGate {
            name: "t".into(),
            conditions: vec![GateCondition {
                metric: GateMetric::DuplicatedLinesPct,
                op: GateOperator::GreaterThan,
                threshold: 1.0,
                warning_only: true,
            }],
        };
        let mut m = ProjectMetrics::default();
        m.duplicated_lines_pct = 5.0;
        m.security_hotspots_reviewed_pct = 100.0;
        let e = evaluate(&gate, &m);
        assert_eq!(e.status, GateLevel::Warn);
    }

    #[test]
    fn operators_evaluate_correctly() {
        assert!(satisfies(5.0, &GateOperator::GreaterThan, 4.0));
        assert!(!satisfies(5.0, &GateOperator::LessThan, 4.0));
        assert!(satisfies(5.0, &GateOperator::Equal, 5.0));
        assert!(satisfies(5.0, &GateOperator::NotEqual, 4.0));
        assert!(satisfies(5.0, &GateOperator::GreaterOrEqual, 5.0));
        assert!(satisfies(5.0, &GateOperator::LessOrEqual, 5.0));
    }

    #[test]
    fn empty_gate_passes() {
        let g = QualityGate {
            name: "empty".into(),
            conditions: vec![],
        };
        let m = ProjectMetrics::from_findings(&[finding(FindingSeverity::Critical)], 0.0);
        let e = evaluate(&g, &m);
        assert_eq!(e.status, GateLevel::Ok);
    }

    #[test]
    fn error_dominates_warning_in_aggregation() {
        let g = QualityGate {
            name: "mix".into(),
            conditions: vec![
                GateCondition {
                    metric: GateMetric::DuplicatedLinesPct,
                    op: GateOperator::GreaterThan,
                    threshold: 0.0,
                    warning_only: true,
                },
                GateCondition {
                    metric: GateMetric::NewCriticalIssues,
                    op: GateOperator::GreaterThan,
                    threshold: 0.0,
                    warning_only: false,
                },
            ],
        };
        let mut m = ProjectMetrics::default();
        m.duplicated_lines_pct = 5.0;
        m.new_critical = 1;
        let e = evaluate(&g, &m);
        assert_eq!(e.status, GateLevel::Error);
    }
}
