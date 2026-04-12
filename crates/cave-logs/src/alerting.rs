<<<<<<< HEAD
//! Log-based alerting: rule evaluation, pattern detection, anomaly detection.

use crate::models::{AlertCondition, AlertSeverity, LogAlert, LogLevel};
use crate::query::execute_query;
use crate::LogsState;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

// ── Result Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AlertFiring {
    pub alert_id: Uuid,
    pub alert_name: String,
    pub severity: AlertSeverity,
    pub value: f64,
    pub threshold: f64,
    pub message: String,
    pub fired_at: DateTime<Utc>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Evaluate every enabled alert rule and return those currently firing.
pub fn evaluate_all_alerts(state: &Arc<LogsState>) -> Vec<AlertFiring> {
    let alerts: Vec<LogAlert> = {
        let lock = state.alerts.lock().unwrap();
        lock.values().filter(|a| a.enabled).cloned().collect()
    };
    alerts
        .iter()
        .filter_map(|alert| evaluate_alert(state, alert))
        .collect()
}

/// Evaluate a single alert rule; returns `Some(AlertFiring)` if it is firing.
pub fn evaluate_alert(state: &Arc<LogsState>, alert: &LogAlert) -> Option<AlertFiring> {
    let value = match &alert.condition {
        AlertCondition::PatternMatch => detect_pattern(state, alert),
        AlertCondition::AnomalyDetected => detect_anomaly(state, alert),
        _ => execute_query(state, &alert.query).total as f64,
    };

    let fires = match &alert.condition {
        AlertCondition::GreaterThan => value > alert.threshold,
        AlertCondition::LessThan => value < alert.threshold,
        AlertCondition::EqualTo => (value - alert.threshold).abs() < f64::EPSILON,
        AlertCondition::PatternMatch => value > 0.0,
        AlertCondition::AnomalyDetected => value > alert.threshold,
    };

    if fires {
        Some(AlertFiring {
            alert_id: alert.id,
            alert_name: alert.name.clone(),
            severity: alert.severity.clone(),
            value,
            threshold: alert.threshold,
            message: format!(
                "Alert '{}' fired: value={:.2} threshold={:.2}",
                alert.name, value, alert.threshold
            ),
            fired_at: Utc::now(),
        })
    } else {
        None
    }
}

// ── Detection Algorithms ──────────────────────────────────────────────────────

/// Count how many log lines match the alert's `regex_filter` within the alert's time window.
pub fn detect_pattern(state: &Arc<LogsState>, alert: &LogAlert) -> f64 {
    let pattern = match &alert.query.regex_filter {
        Some(p) => p.clone(),
        None => return 0.0,
    };
    let re = match Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return 0.0,
    };

    let cutoff = Utc::now() - chrono::Duration::seconds(alert.window_seconds as i64);
    let entries = state.entries.lock().unwrap();
    entries
        .iter()
        .filter(|e| e.timestamp >= cutoff && re.is_match(&e.message))
        .count() as f64
}

/// Detect an anomalous error rate by comparing the recent window to a 5× baseline window.
///
/// Returns a multiplier: `recent_error_rate / baseline_error_rate`.
/// A value > 1.0 means the error rate is elevated above the baseline.
pub fn detect_anomaly(state: &Arc<LogsState>, alert: &LogAlert) -> f64 {
    let window = alert.window_seconds as i64;
    let now = Utc::now();
    let recent_cutoff = now - chrono::Duration::seconds(window);
    let baseline_cutoff = now - chrono::Duration::seconds(window * 5);

    let entries = state.entries.lock().unwrap();

    let service_filter = alert.query.service.as_deref();

    let in_service = |svc: &str| -> bool {
        service_filter.map_or(true, |s| s == svc)
    };

    let recent_total = entries
        .iter()
        .filter(|e| e.timestamp >= recent_cutoff && in_service(&e.service))
        .count() as f64;

    let recent_errors = entries
        .iter()
        .filter(|e| {
            e.timestamp >= recent_cutoff
                && in_service(&e.service)
                && matches!(e.level, LogLevel::Error | LogLevel::Fatal)
        })
        .count() as f64;

    let baseline_entries: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.timestamp >= baseline_cutoff
                && e.timestamp < recent_cutoff
                && in_service(&e.service)
        })
        .collect();

    if baseline_entries.is_empty() || recent_total == 0.0 {
        return 0.0;
    }

    let baseline_errors = baseline_entries
        .iter()
        .filter(|e| matches!(e.level, LogLevel::Error | LogLevel::Fatal))
        .count() as f64;

    let baseline_rate = baseline_errors / baseline_entries.len() as f64;
    let recent_rate = recent_errors / recent_total;

    if baseline_rate < 0.001 {
        // No meaningful baseline — report the absolute recent rate scaled up
        recent_rate * 100.0
    } else {
        recent_rate / baseline_rate
=======
//! Log-based alerting rules.
//!
//! Rules are stored in-process (Arc<RwLock<Vec<AlertRule>>>).  The evaluator
//! runs periodically and fires alerts when the LogQL metric expression crosses
//! the configured threshold.

use crate::models::{AlertRule, FiredAlert};
use crate::store::LogStore;
use chrono::Utc;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

pub struct AlertManager {
    rules: Arc<RwLock<Vec<AlertRule>>>,
    store: Arc<LogStore>,
}

impl AlertManager {
    pub fn new(store: Arc<LogStore>) -> Self {
        Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            store,
        }
    }

    pub fn add_rule(&self, rule: AlertRule) {
        self.rules.write().unwrap().push(rule);
    }

    pub fn remove_rule(&self, id: uuid::Uuid) {
        self.rules.write().unwrap().retain(|r| r.id != id);
    }

    pub fn list_rules(&self) -> Vec<AlertRule> {
        self.rules.read().unwrap().clone()
    }

    /// Evaluate all rules against the current store state.
    /// Returns any fired alerts.
    pub fn evaluate(&self) -> Vec<FiredAlert> {
        let rules = self.rules.read().unwrap().clone();
        let now = Utc::now();
        let mut fired = vec![];

        for rule in &rules {
            debug!(rule = %rule.name, "evaluating alert rule");
            match self.store.eval_alert(rule, now) {
                Some(alert) => {
                    info!(
                        rule = %alert.rule_name,
                        value = alert.value,
                        severity = ?alert.severity,
                        "alert fired"
                    );
                    fired.push(alert);
                }
                None => {}
            }
        }

        fired
    }

    /// Background task: evaluate rules every `interval_secs` seconds.
    pub async fn run_loop(self: Arc<Self>, interval_secs: u64) {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            let fired = self.evaluate();
            if !fired.is_empty() {
                warn!(count = fired.len(), "alert rules fired");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertCondition, AlertSeverity, CompareOp, Labels, LogEntry};
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn make_store() -> Arc<LogStore> {
        Arc::new(LogStore::new(Duration::days(1)))
    }

    #[test]
    fn alert_fires_when_count_exceeds_threshold() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "failing".into())].into());
        for _ in 0..10 {
            store.push(
                labels.clone(),
                vec![LogEntry {
                    timestamp: Utc::now() - Duration::seconds(30),
                    line: "ERROR something went wrong".into(),
                    structured_metadata: HashMap::new(),
                }],
                None,
            );
        }

        let manager = AlertManager::new(Arc::clone(&store));
        manager.add_rule(AlertRule {
            id: uuid::Uuid::new_v4(),
            name: "high-error-rate".into(),
            expr: r#"count_over_time({app="failing"}[5m])"#.into(),
            duration_secs: 300,
            condition: AlertCondition { op: CompareOp::Gt, threshold: 5.0 },
            severity: AlertSeverity::Critical,
            annotations: HashMap::new(),
            tenant: None,
        });

        let fired = manager.evaluate();
        assert!(!fired.is_empty(), "alert should have fired");
        assert!(fired[0].value > 5.0);
    }

    #[test]
    fn alert_does_not_fire_below_threshold() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "quiet".into())].into());
        store.push(
            labels,
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(30),
                line: "INFO ok".into(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        let manager = AlertManager::new(Arc::clone(&store));
        manager.add_rule(AlertRule {
            id: uuid::Uuid::new_v4(),
            name: "high-count".into(),
            expr: r#"count_over_time({app="quiet"}[5m])"#.into(),
            duration_secs: 300,
            condition: AlertCondition { op: CompareOp::Gt, threshold: 100.0 },
            severity: AlertSeverity::Warning,
            annotations: HashMap::new(),
            tenant: None,
        });

        let fired = manager.evaluate();
        assert!(fired.is_empty(), "alert should not have fired");
>>>>>>> claude/inspiring-pascal
    }
}
