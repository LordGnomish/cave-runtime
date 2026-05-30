// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runtime observability analysis — self-improvement input.
//!
//! Ingests the three Cave observability streams the runtime agent reads —
//! metrics (`cave-metrics`), logs (`cave-logs`), and traces (`cave-trace`) —
//! into a single [`ObservationWindow`], derives signals (metric means, log
//! error-rate, trace latency percentiles), and flags [`Anomaly`]s against a
//! declarative rule set. Anomalies are the input to [`super::tune`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Log severity level (parsed from `cave-logs` lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Case-insensitive parse; anything unrecognised is treated as `Info`.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" | "err" | "fatal" | "critical" => LogLevel::Error,
            "warn" | "warning" => LogLevel::Warn,
            _ => LogLevel::Info,
        }
    }
}

/// Anomaly severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Warning,
    Critical,
}

/// A breached rule, with the observed value and the limit it crossed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anomaly {
    pub signal: String,
    pub observed: f64,
    pub limit: f64,
    pub severity: Severity,
    pub detail: String,
}

/// A single window of observability data across all three streams.
#[derive(Debug, Default, Clone)]
pub struct ObservationWindow {
    metrics: BTreeMap<String, Vec<f64>>,
    logs: Vec<(LogLevel, String)>,
    spans: BTreeMap<String, Vec<f64>>,
}

impl ObservationWindow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_metric(&mut self, name: &str, value: f64) -> &mut Self {
        self.metrics.entry(name.to_string()).or_default().push(value);
        self
    }

    pub fn record_log(&mut self, level: LogLevel, message: &str) -> &mut Self {
        self.logs.push((level, message.to_string()));
        self
    }

    pub fn record_span(&mut self, name: &str, latency_ms: f64) -> &mut Self {
        self.spans.entry(name.to_string()).or_default().push(latency_ms);
        self
    }

    /// Fraction of log lines at `Error` level (0.0 when there are no logs).
    pub fn error_rate(&self) -> f64 {
        if self.logs.is_empty() {
            return 0.0;
        }
        let errors = self
            .logs
            .iter()
            .filter(|(l, _)| *l == LogLevel::Error)
            .count();
        errors as f64 / self.logs.len() as f64
    }

    /// Mean of a metric's samples, or `None` if the metric is unseen.
    pub fn metric_mean(&self, name: &str) -> Option<f64> {
        let v = self.metrics.get(name)?;
        if v.is_empty() {
            return None;
        }
        Some(v.iter().sum::<f64>() / v.len() as f64)
    }

    /// Nearest-rank percentile of a span's latency samples.
    pub fn latency_percentile(&self, span: &str, p: f64) -> Option<f64> {
        let v = self.spans.get(span)?;
        if v.is_empty() {
            return None;
        }
        let mut sorted = v.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let rank = ((p / 100.0) * n as f64).ceil() as usize;
        let idx = rank.clamp(1, n) - 1;
        Some(sorted[idx])
    }
}

/// What derived signal a [`Rule`] watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Target {
    MetricMean(String),
    P99(String),
    ErrorRate,
}

/// A declarative "signal above limit → anomaly" rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    target: Target,
    limit: f64,
    severity: Severity,
}

impl Rule {
    pub fn metric_above(name: &str, limit: f64, severity: Severity) -> Self {
        Self {
            target: Target::MetricMean(name.to_string()),
            limit,
            severity,
        }
    }

    pub fn p99_above(span: &str, limit: f64, severity: Severity) -> Self {
        Self {
            target: Target::P99(span.to_string()),
            limit,
            severity,
        }
    }

    pub fn error_rate_above(limit: f64, severity: Severity) -> Self {
        Self {
            target: Target::ErrorRate,
            limit,
            severity,
        }
    }

    fn evaluate(&self, w: &ObservationWindow) -> Option<Anomaly> {
        let observed = match &self.target {
            Target::MetricMean(name) => w.metric_mean(name)?,
            Target::P99(span) => w.latency_percentile(span, 99.0)?,
            Target::ErrorRate => w.error_rate(),
        };
        if observed > self.limit {
            let signal = match &self.target {
                Target::MetricMean(name) => name.clone(),
                Target::P99(span) => format!("p99:{span}"),
                Target::ErrorRate => "error_rate".to_string(),
            };
            Some(Anomaly {
                signal,
                observed,
                limit: self.limit,
                severity: self.severity,
                detail: format!("observed {observed:.4} > limit {:.4}", self.limit),
            })
        } else {
            None
        }
    }
}

/// Evaluates a set of [`Rule`]s against an [`ObservationWindow`].
#[derive(Debug, Default, Clone)]
pub struct ObservationAnalyzer {
    rules: Vec<Rule>,
}

impl ObservationAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// All anomalies the rules detect, in rule order.
    pub fn analyze(&self, window: &ObservationWindow) -> Vec<Anomaly> {
        self.rules.iter().filter_map(|r| r.evaluate(window)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> ObservationWindow {
        let mut w = ObservationWindow::new();
        w.record_metric("api_latency_ms", 120.0);
        w.record_metric("api_latency_ms", 180.0);
        w.record_log(LogLevel::Info, "ok");
        w.record_log(LogLevel::Info, "ok");
        w.record_log(LogLevel::Error, "boom");
        w.record_log(LogLevel::Error, "boom");
        for v in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0] {
            w.record_span("http", v);
        }
        w
    }

    #[test]
    fn error_rate_is_errors_over_total() {
        assert!((window().error_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn error_rate_zero_when_no_logs() {
        assert_eq!(ObservationWindow::new().error_rate(), 0.0);
    }

    #[test]
    fn metric_mean_averages_samples() {
        assert!((window().metric_mean("api_latency_ms").unwrap() - 150.0).abs() < 1e-9);
        assert!(window().metric_mean("absent").is_none());
    }

    #[test]
    fn latency_percentile_nearest_rank() {
        // p99 of 10..=100 (10 samples) → nearest-rank index ceil(0.99*10)-1 = 9 → 100.
        assert_eq!(window().latency_percentile("http", 99.0), Some(100.0));
        // p50 → ceil(0.50*10)-1 = 4 → 50.
        assert_eq!(window().latency_percentile("http", 50.0), Some(50.0));
    }

    #[test]
    fn metric_above_rule_fires() {
        let analyzer = ObservationAnalyzer::new()
            .rule(Rule::metric_above("api_latency_ms", 100.0, Severity::Warning));
        let anomalies = analyzer.analyze(&window());
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].signal, "api_latency_ms");
        assert_eq!(anomalies[0].severity, Severity::Warning);
        assert!((anomalies[0].observed - 150.0).abs() < 1e-9);
    }

    #[test]
    fn p99_above_rule_fires_critical() {
        let analyzer = ObservationAnalyzer::new()
            .rule(Rule::p99_above("http", 95.0, Severity::Critical));
        let anomalies = analyzer.analyze(&window());
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].severity, Severity::Critical);
    }

    #[test]
    fn error_rate_rule_fires_and_clears() {
        let firing = ObservationAnalyzer::new()
            .rule(Rule::error_rate_above(0.25, Severity::Critical))
            .analyze(&window());
        assert_eq!(firing.len(), 1);
        let clear = ObservationAnalyzer::new()
            .rule(Rule::error_rate_above(0.75, Severity::Critical))
            .analyze(&window());
        assert!(clear.is_empty(), "0.5 error-rate does not breach 0.75");
    }

    #[test]
    fn no_rules_means_no_anomalies() {
        assert!(ObservationAnalyzer::new().analyze(&window()).is_empty());
    }

    #[test]
    fn log_level_parses_case_insensitively() {
        assert_eq!(LogLevel::parse("ERROR"), LogLevel::Error);
        assert_eq!(LogLevel::parse("warn"), LogLevel::Warn);
        assert_eq!(LogLevel::parse("whatever"), LogLevel::Info);
    }
}
