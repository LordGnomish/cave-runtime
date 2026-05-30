// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runtime observability analysis — self-improvement input.
//!
//! Ingests the three Cave observability streams the runtime agent reads —
//! metrics (`cave-metrics`), logs (`cave-logs`), and traces (`cave-trace`) —
//! into a single [`ObservationWindow`], derives signals (metric means, log
//! error-rate, trace latency percentiles), and flags [`Anomaly`]s against a
//! declarative rule set. Anomalies are the input to [`super::tune`].

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
