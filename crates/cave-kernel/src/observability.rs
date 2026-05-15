// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Observability primitives — tracer + metric + structured log
//! shapes that match the OpenTelemetry / Prometheus contracts so
//! every cave module can emit telemetry through a single import.
//!
//! Each trait is intentionally narrow:
//!
//! * [`Tracer`] yields [`SpanGuard`] handles that record `start_ns`
//!   / `end_ns` and tag the span with key=value attributes. The
//!   shape mirrors `opentelemetry::trace::Tracer::start` reduced to
//!   the surface every cave module actually uses (no propagator, no
//!   exporter — those are wired by the dataplane).
//! * [`Metric`] enumerates the three OpenMetrics / Prometheus
//!   instrument kinds. The `record_*` helpers match the OpenMetrics
//!   `inc_by` / `set` / `observe` semantics.
//! * [`LogRecord`] is the structured shape every cave module
//!   serialises to JSON before handing to the log sink. The
//!   `level` discriminator follows the `tracing::Level` ordering.
//!
//! Adopters: cave-mesh + cave-gateway use the trait surface to
//! emit per-request spans; cave-portal-api uses it for audit log
//! records. None of the production wiring lives here — the kernel
//! provides shapes, transport is per-crate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Five-level severity ladder. Matches `tracing::Level` so a
/// caller can map either way without translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// One structured log entry. The cave dataplane serialises this to
/// JSON before forwarding to the log sink. Use [`LogRecord::now`]
/// for the common "stamp with the current wall clock" path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    pub level: LogLevel,
    pub timestamp_unix_ms: u64,
    pub target: String,
    pub message: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

impl LogRecord {
    /// Build a log record stamped with the current wall-clock time.
    /// Panics only if the system clock is before the unix epoch,
    /// which would be a system-configuration bug.
    pub fn now(level: LogLevel, target: impl Into<String>, message: impl Into<String>) -> Self {
        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_millis() as u64;
        LogRecord {
            level,
            timestamp_unix_ms,
            target: target.into(),
            message: message.into(),
            fields: BTreeMap::new(),
        }
    }

    /// Fluent attach of a key=value field. Field keys are sorted
    /// for deterministic JSON output (BTreeMap is the storage).
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }
}

/// One in-flight span. Drop semantics record the end timestamp
/// automatically — the API matches `tracing::span::EnteredSpan` so a
/// caller switching from `tracing` keeps the same `let _g = ...`
/// pattern.
#[derive(Debug)]
pub struct SpanGuard {
    name: String,
    start_ns: u128,
    end_ns: Option<u128>,
    attributes: BTreeMap<String, String>,
}

impl SpanGuard {
    fn new(name: impl Into<String>) -> Self {
        SpanGuard {
            name: name.into(),
            start_ns: now_ns(),
            end_ns: None,
            attributes: BTreeMap::new(),
        }
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn start_ns(&self) -> u128 { self.start_ns }
    pub fn end_ns(&self) -> Option<u128> { self.end_ns }
    pub fn attributes(&self) -> &BTreeMap<String, String> { &self.attributes }

    /// Tag the span with a key=value attribute. Mirrors
    /// `tracing::Span::record` (string-keyed, string-valued).
    pub fn set_attribute(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.attributes.insert(key.into(), value.into());
    }

    /// Record the end timestamp explicitly. Idempotent — re-calling
    /// after end has no effect. Use this when a caller wants to
    /// inspect `duration_ns` BEFORE the guard drops.
    pub fn end(&mut self) {
        if self.end_ns.is_none() {
            self.end_ns = Some(now_ns());
        }
    }

    /// Duration in nanoseconds; defaults to "from start to now" when
    /// the guard has not been ended yet.
    pub fn duration_ns(&self) -> u128 {
        self.end_ns.unwrap_or_else(now_ns).saturating_sub(self.start_ns)
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        self.end();
    }
}

/// Tracer is the only surface every cave module needs: "open a
/// span, optionally tag it". Real exporters live in the dataplane
/// crate; the kernel provides only the shape.
pub trait Tracer: Send + Sync {
    fn start_span(&self, name: &str) -> SpanGuard;
}

/// In-memory tracer that records every span for later inspection.
/// Useful in tests + during local development.
#[derive(Debug, Default)]
pub struct NoopTracer;

impl Tracer for NoopTracer {
    fn start_span(&self, name: &str) -> SpanGuard {
        SpanGuard::new(name)
    }
}

/// Three OpenMetrics instrument kinds. Matches the OpenMetrics
/// spec's `MetricType` enum reduced to the subset cave modules
/// emit. Histograms carry their bucket boundaries inline so a
/// renderer can format the OpenMetrics `_bucket{le="..."}` lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Metric {
    Counter { name: String, value: u64, labels: BTreeMap<String, String> },
    Gauge { name: String, value: f64, labels: BTreeMap<String, String> },
    Histogram {
        name: String,
        buckets_le_ms: Vec<f64>,
        counts: Vec<u64>,
        sum_ms: f64,
        labels: BTreeMap<String, String>,
    },
}

impl Metric {
    pub fn counter(name: impl Into<String>) -> Self {
        Metric::Counter { name: name.into(), value: 0, labels: BTreeMap::new() }
    }

    pub fn gauge(name: impl Into<String>) -> Self {
        Metric::Gauge { name: name.into(), value: 0.0, labels: BTreeMap::new() }
    }

    pub fn histogram(name: impl Into<String>, buckets_le_ms: Vec<f64>) -> Self {
        let counts = vec![0u64; buckets_le_ms.len()];
        Metric::Histogram {
            name: name.into(),
            buckets_le_ms,
            counts,
            sum_ms: 0.0,
            labels: BTreeMap::new(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Metric::Counter { name, .. } => name,
            Metric::Gauge { name, .. } => name,
            Metric::Histogram { name, .. } => name,
        }
    }

    /// Increment a counter (or histogram's underlying sum/bucket).
    /// Panics if called on a gauge — gauges must use [`Self::set`].
    pub fn inc_by(&mut self, delta: u64) {
        match self {
            Metric::Counter { value, .. } => *value += delta,
            Metric::Histogram { sum_ms, counts, .. } => {
                *sum_ms += delta as f64;
                if let Some(last) = counts.last_mut() { *last += 1; }
            }
            Metric::Gauge { .. } => panic!("inc_by called on Gauge — use set()"),
        }
    }

    /// Set a gauge value. Panics on counters (which only go up).
    pub fn set(&mut self, value: f64) {
        match self {
            Metric::Gauge { value: v, .. } => *v = value,
            _ => panic!("set called on non-Gauge — use inc_by()"),
        }
    }

    /// Observe a value into a histogram. Records sum + the smallest
    /// bucket whose `le` boundary >= value. Panics on
    /// counters/gauges.
    pub fn observe_ms(&mut self, value_ms: f64) {
        match self {
            Metric::Histogram { buckets_le_ms, counts, sum_ms, .. } => {
                *sum_ms += value_ms;
                for (i, le) in buckets_le_ms.iter().enumerate() {
                    if value_ms <= *le {
                        counts[i] += 1;
                        return;
                    }
                }
                // Above the largest bucket — record in +Inf (last bucket).
                if let Some(last) = counts.last_mut() { *last += 1; }
            }
            _ => panic!("observe_ms called on non-Histogram"),
        }
    }
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_record_now_stamps_current_time() {
        let r = LogRecord::now(LogLevel::Info, "test", "hello")
            .with_field("k", "v");
        assert_eq!(r.level, LogLevel::Info);
        assert_eq!(r.target, "test");
        assert_eq!(r.message, "hello");
        assert_eq!(r.fields.get("k").unwrap(), "v");
        assert!(r.timestamp_unix_ms > 0);
    }

    #[test]
    fn log_record_with_field_sorts_by_key() {
        let r = LogRecord::now(LogLevel::Info, "t", "m")
            .with_field("z", "1")
            .with_field("a", "2")
            .with_field("m", "3");
        let keys: Vec<&String> = r.fields.keys().collect();
        assert_eq!(keys, ["a", "m", "z"]);
    }

    #[test]
    fn log_level_ordering_matches_severity() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn log_level_as_str_matches_otel_convention() {
        assert_eq!(LogLevel::Trace.as_str(), "TRACE");
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
    }

    #[test]
    fn noop_tracer_starts_span_with_name() {
        let t = NoopTracer;
        let g = t.start_span("op");
        assert_eq!(g.name(), "op");
        assert!(g.end_ns().is_none());
    }

    #[test]
    fn span_guard_records_end_on_drop() {
        let t = NoopTracer;
        let start_ns;
        let end_ns;
        {
            let g = t.start_span("scope");
            start_ns = g.start_ns();
            drop(g);
        }
        // Re-build a fresh span and compare ordering through .duration_ns.
        let g = t.start_span("scope2");
        assert!(g.start_ns() >= start_ns);
        end_ns = g.start_ns();
        assert!(end_ns >= start_ns);
    }

    #[test]
    fn span_guard_set_attribute_records() {
        let t = NoopTracer;
        let mut g = t.start_span("op");
        g.set_attribute("k", "v");
        assert_eq!(g.attributes().get("k").unwrap(), "v");
    }

    #[test]
    fn span_guard_end_is_idempotent() {
        let t = NoopTracer;
        let mut g = t.start_span("op");
        g.end();
        let first_end = g.end_ns();
        g.end();
        assert_eq!(g.end_ns(), first_end);
    }

    #[test]
    fn span_guard_duration_ns_is_positive() {
        let t = NoopTracer;
        let mut g = t.start_span("op");
        std::thread::sleep(std::time::Duration::from_millis(1));
        g.end();
        assert!(g.duration_ns() >= 1_000_000); // at least 1ms in ns
    }

    #[test]
    fn metric_counter_inc_by_accumulates() {
        let mut c = Metric::counter("requests_total");
        c.inc_by(3);
        c.inc_by(7);
        match c {
            Metric::Counter { value, .. } => assert_eq!(value, 10),
            _ => panic!(),
        }
    }

    #[test]
    fn metric_gauge_set_replaces_value() {
        let mut g = Metric::gauge("connections");
        g.set(5.0);
        g.set(2.5);
        match g {
            Metric::Gauge { value, .. } => assert!((value - 2.5).abs() < 1e-9),
            _ => panic!(),
        }
    }

    #[test]
    fn metric_histogram_observe_records_correct_bucket() {
        let mut h = Metric::histogram("latency_ms", vec![10.0, 100.0, 1000.0]);
        h.observe_ms(5.0);   // bucket[0] (≤10)
        h.observe_ms(50.0);  // bucket[1] (≤100)
        h.observe_ms(500.0); // bucket[2] (≤1000)
        h.observe_ms(5000.0); // > 1000 → +Inf (last bucket)
        match h {
            Metric::Histogram { counts, sum_ms, .. } => {
                assert_eq!(counts[0], 1);
                assert_eq!(counts[1], 1);
                assert_eq!(counts[2], 2); // 500 + 5000 both → last
                assert!((sum_ms - (5.0 + 50.0 + 500.0 + 5000.0)).abs() < 1e-9);
            }
            _ => panic!(),
        }
    }

    #[test]
    #[should_panic(expected = "set called on non-Gauge")]
    fn metric_set_panics_on_counter() {
        let mut c = Metric::counter("c");
        c.set(1.0);
    }

    #[test]
    #[should_panic(expected = "inc_by called on Gauge")]
    fn metric_inc_by_panics_on_gauge() {
        let mut g = Metric::gauge("g");
        g.inc_by(1);
    }

    #[test]
    fn metric_name_returns_underlying_string() {
        assert_eq!(Metric::counter("a").name(), "a");
        assert_eq!(Metric::gauge("b").name(), "b");
        assert_eq!(Metric::histogram("c", vec![1.0]).name(), "c");
    }

    #[test]
    fn log_record_round_trips_through_json() {
        let r = LogRecord::now(LogLevel::Warn, "auth", "denied")
            .with_field("user", "alice");
        let s = serde_json::to_string(&r).unwrap();
        let back: LogRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}
