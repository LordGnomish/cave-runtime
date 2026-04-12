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
    }
}
