//! gRPC stats sink — Envoy `metrics_service.v3.StreamMetricsMessage` parity.
//!
//! Buffers counters / gauges / histograms in-process and produces a wire-shape
//! `StreamMetricsMessage` ready to be streamed to a downstream metrics
//! service. The transport itself plugs in later; this module owns:
//!
//! - the metric registry (counters, gauges, histograms with bucket bounds),
//! - delta and snapshot semantics,
//! - tag/label support with bounded cardinality,
//! - flush cadence + buffer size config.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsSinkConfig {
    /// gRPC service URI (e.g. `dns:///metrics-service:9000`).
    pub uri: String,
    pub flush_interval: Duration,
    pub max_buffered_messages: u32,
    /// If true, metrics are sent as cumulative values; otherwise deltas.
    pub report_counters_as_deltas: bool,
    /// Maximum number of distinct tag value combinations per metric.
    pub max_tag_cardinality: u32,
}

impl Default for StatsSinkConfig {
    fn default() -> Self {
        Self {
            uri: "".into(),
            flush_interval: Duration::from_secs(5),
            max_buffered_messages: 1000,
            report_counters_as_deltas: false,
            max_tag_cardinality: 1024,
        }
    }
}

impl StatsSinkConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.uri.is_empty() {
            return Err("uri is empty".into());
        }
        if self.flush_interval.is_zero() {
            return Err("flush_interval must be > 0".into());
        }
        if self.max_buffered_messages == 0 {
            return Err("max_buffered_messages must be > 0".into());
        }
        if self.max_tag_cardinality == 0 {
            return Err("max_tag_cardinality must be > 0".into());
        }
        Ok(())
    }
}

// ── Metric kinds ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetricKind {
    Counter { value: u64 },
    Gauge { value: f64 },
    Histogram { buckets: Vec<HistogramBucket>, sum: f64, count: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistogramBucket {
    pub upper_bound: f64,
    pub count: u64,
}

impl HistogramBucket {
    pub fn standard_latency_bounds() -> Vec<f64> {
        // Envoy default histogram buckets (ms).
        vec![
            0.5, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10_000.0, 30_000.0, 60_000.0, 300_000.0, 600_000.0, 1_800_000.0, 3_600_000.0,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricKey {
    pub name: String,
    pub tags: BTreeMap<String, String>,
}

impl MetricKey {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tags: BTreeMap::new(),
        }
    }

    pub fn with_tag(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.tags.insert(k.into(), v.into());
        self
    }

    pub fn canonical(&self) -> String {
        let mut s = self.name.clone();
        for (k, v) in &self.tags {
            s.push('|');
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        s
    }
}

// ── Counter / Gauge / Histogram ─────────────────────────────────────────────

#[derive(Debug, Default)]
struct CounterEntry {
    cumulative: u64,
    last_flushed: u64,
}

#[derive(Debug, Default)]
struct GaugeEntry {
    value: f64,
}

#[derive(Debug, Clone)]
struct HistogramEntry {
    bounds: Vec<f64>,
    counts: Vec<u64>,
    sum: f64,
    count: u64,
}

impl HistogramEntry {
    fn new(bounds: Vec<f64>) -> Self {
        let n = bounds.len();
        Self { bounds, counts: vec![0; n], sum: 0.0, count: 0 }
    }

    fn record(&mut self, value: f64) {
        for (i, b) in self.bounds.iter().enumerate() {
            if value <= *b {
                self.counts[i] += 1;
            }
        }
        self.sum += value;
        self.count += 1;
    }

    fn snapshot(&self) -> (Vec<HistogramBucket>, f64, u64) {
        let buckets: Vec<HistogramBucket> = self
            .bounds
            .iter()
            .zip(&self.counts)
            .map(|(b, c)| HistogramBucket { upper_bound: *b, count: *c })
            .collect();
        (buckets, self.sum, self.count)
    }
}

// ── StatsSink — public registry ─────────────────────────────────────────────

#[derive(Default)]
pub struct StatsSink {
    counters: Mutex<HashMap<String, CounterEntry>>,
    gauges: Mutex<HashMap<String, GaugeEntry>>,
    histograms: Mutex<HashMap<String, HistogramEntry>>,
    config: StatsSinkConfig,
}

impl StatsSink {
    pub fn new(config: StatsSinkConfig) -> Arc<Self> {
        Arc::new(Self {
            counters: Mutex::new(HashMap::new()),
            gauges: Mutex::new(HashMap::new()),
            histograms: Mutex::new(HashMap::new()),
            config,
        })
    }

    pub fn config(&self) -> &StatsSinkConfig {
        &self.config
    }

    pub fn inc_counter(&self, key: &MetricKey, by: u64) -> Result<(), String> {
        let canonical = key.canonical();
        let mut g = self.counters.lock().unwrap();
        if (g.len() as u32) >= self.config.max_tag_cardinality && !g.contains_key(&canonical) {
            return Err(format!(
                "tag cardinality > {}",
                self.config.max_tag_cardinality
            ));
        }
        g.entry(canonical).or_default().cumulative += by;
        Ok(())
    }

    pub fn set_gauge(&self, key: &MetricKey, value: f64) -> Result<(), String> {
        let canonical = key.canonical();
        let mut g = self.gauges.lock().unwrap();
        if (g.len() as u32) >= self.config.max_tag_cardinality && !g.contains_key(&canonical) {
            return Err(format!(
                "tag cardinality > {}",
                self.config.max_tag_cardinality
            ));
        }
        g.entry(canonical).or_default().value = value;
        Ok(())
    }

    pub fn observe_histogram(
        &self,
        key: &MetricKey,
        value: f64,
        bounds: &[f64],
    ) -> Result<(), String> {
        let canonical = key.canonical();
        let mut g = self.histograms.lock().unwrap();
        if !g.contains_key(&canonical) {
            if (g.len() as u32) >= self.config.max_tag_cardinality {
                return Err(format!(
                    "tag cardinality > {}",
                    self.config.max_tag_cardinality
                ));
            }
            g.insert(canonical.clone(), HistogramEntry::new(bounds.to_vec()));
        }
        g.get_mut(&canonical).unwrap().record(value);
        Ok(())
    }

    pub fn counter_value(&self, key: &MetricKey) -> u64 {
        let g = self.counters.lock().unwrap();
        g.get(&key.canonical())
            .map(|e| e.cumulative)
            .unwrap_or(0)
    }

    pub fn gauge_value(&self, key: &MetricKey) -> Option<f64> {
        let g = self.gauges.lock().unwrap();
        g.get(&key.canonical()).map(|e| e.value)
    }

    pub fn snapshot(&self) -> StreamMetricsMessage {
        let mut metrics = Vec::new();
        let now = current_unix_nanos();
        let report_deltas = self.config.report_counters_as_deltas;

        {
            let mut g = self.counters.lock().unwrap();
            for (canonical, e) in g.iter_mut() {
                let value = if report_deltas {
                    let delta = e.cumulative - e.last_flushed;
                    e.last_flushed = e.cumulative;
                    delta
                } else {
                    e.cumulative
                };
                let (name, tags) = decanonicalize(canonical);
                metrics.push(Metric {
                    name,
                    tags,
                    timestamp_unix_nanos: now,
                    kind: MetricKind::Counter { value },
                });
            }
        }
        {
            let g = self.gauges.lock().unwrap();
            for (canonical, e) in g.iter() {
                let (name, tags) = decanonicalize(canonical);
                metrics.push(Metric {
                    name,
                    tags,
                    timestamp_unix_nanos: now,
                    kind: MetricKind::Gauge { value: e.value },
                });
            }
        }
        {
            let g = self.histograms.lock().unwrap();
            for (canonical, e) in g.iter() {
                let (name, tags) = decanonicalize(canonical);
                let (buckets, sum, count) = e.snapshot();
                metrics.push(Metric {
                    name,
                    tags,
                    timestamp_unix_nanos: now,
                    kind: MetricKind::Histogram { buckets, sum, count },
                });
            }
        }
        StreamMetricsMessage { metrics }
    }
}

fn decanonicalize(canonical: &str) -> (String, BTreeMap<String, String>) {
    let mut iter = canonical.split('|');
    let name = iter.next().unwrap_or("").to_string();
    let mut tags = BTreeMap::new();
    for kv in iter {
        if let Some((k, v)) = kv.split_once('=') {
            tags.insert(k.to_string(), v.to_string());
        }
    }
    (name, tags)
}

fn current_unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub tags: BTreeMap<String, String>,
    pub timestamp_unix_nanos: u64,
    #[serde(flatten)]
    pub kind: MetricKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamMetricsMessage {
    pub metrics: Vec<Metric>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> StatsSinkConfig {
        StatsSinkConfig {
            uri: "dns:///metrics:9000".into(),
            ..Default::default()
        }
    }

    #[test]
    fn config_default_invalid_no_uri() {
        let c = StatsSinkConfig::default();
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_uri_validates() {
        cfg().validate().unwrap();
    }

    #[test]
    fn config_zero_flush_rejects() {
        let mut c = cfg();
        c.flush_interval = Duration::ZERO;
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_zero_buffer_rejects() {
        let mut c = cfg();
        c.max_buffered_messages = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn metric_key_canonical_sorted_tags() {
        let k = MetricKey::new("http_requests")
            .with_tag("method", "GET")
            .with_tag("status", "200");
        assert_eq!(k.canonical(), "http_requests|method=GET|status=200");
    }

    #[test]
    fn metric_key_canonical_btreemap_order() {
        let k = MetricKey::new("x")
            .with_tag("z", "1")
            .with_tag("a", "2")
            .with_tag("m", "3");
        // BTreeMap orders keys alphabetically.
        assert_eq!(k.canonical(), "x|a=2|m=3|z=1");
    }

    #[test]
    fn counter_increments() {
        let s = StatsSink::new(cfg());
        let k = MetricKey::new("requests").with_tag("svc", "a");
        s.inc_counter(&k, 1).unwrap();
        s.inc_counter(&k, 4).unwrap();
        assert_eq!(s.counter_value(&k), 5);
    }

    #[test]
    fn gauge_replaces() {
        let s = StatsSink::new(cfg());
        let k = MetricKey::new("active_conns").with_tag("listener", "ingress");
        s.set_gauge(&k, 42.0).unwrap();
        s.set_gauge(&k, 7.5).unwrap();
        assert_eq!(s.gauge_value(&k), Some(7.5));
    }

    #[test]
    fn histogram_records_buckets() {
        let s = StatsSink::new(cfg());
        let k = MetricKey::new("latency_ms");
        let bounds = vec![1.0, 10.0, 100.0];
        s.observe_histogram(&k, 0.5, &bounds).unwrap();
        s.observe_histogram(&k, 5.0, &bounds).unwrap();
        s.observe_histogram(&k, 50.0, &bounds).unwrap();
        s.observe_histogram(&k, 500.0, &bounds).unwrap();
        let snap = s.snapshot();
        let h = snap.metrics.iter().find(|m| m.name == "latency_ms").unwrap();
        match &h.kind {
            MetricKind::Histogram { buckets, sum, count } => {
                assert_eq!(buckets[0].count, 1); // 0.5
                assert_eq!(buckets[1].count, 2); // 0.5, 5
                assert_eq!(buckets[2].count, 3); // 0.5, 5, 50
                assert_eq!(*count, 4);
                assert!((*sum - 555.5).abs() < 0.001);
            }
            _ => panic!("expected histogram"),
        }
    }

    #[test]
    fn snapshot_emits_counters_and_gauges() {
        let s = StatsSink::new(cfg());
        s.inc_counter(&MetricKey::new("a"), 10).unwrap();
        s.set_gauge(&MetricKey::new("b"), 3.14).unwrap();
        let snap = s.snapshot();
        assert!(snap.metrics.iter().any(|m| matches!(m.kind, MetricKind::Counter { .. })));
        assert!(snap.metrics.iter().any(|m| matches!(m.kind, MetricKind::Gauge { .. })));
    }

    #[test]
    fn snapshot_delta_mode_zeroes_after_flush() {
        let mut c = cfg();
        c.report_counters_as_deltas = true;
        let s = StatsSink::new(c);
        s.inc_counter(&MetricKey::new("a"), 5).unwrap();
        let snap = s.snapshot();
        assert!(matches!(
            snap.metrics[0].kind,
            MetricKind::Counter { value: 5 }
        ));
        let snap2 = s.snapshot();
        // After flush, delta = 0.
        match &snap2.metrics[0].kind {
            MetricKind::Counter { value } => assert_eq!(*value, 0),
            _ => panic!("expected counter"),
        }
    }

    #[test]
    fn snapshot_cumulative_mode_keeps_value() {
        let s = StatsSink::new(cfg());
        s.inc_counter(&MetricKey::new("a"), 5).unwrap();
        s.snapshot();
        s.inc_counter(&MetricKey::new("a"), 7).unwrap();
        let snap2 = s.snapshot();
        match &snap2.metrics[0].kind {
            MetricKind::Counter { value } => assert_eq!(*value, 12),
            _ => panic!("expected counter"),
        }
    }

    #[test]
    fn cardinality_cap_blocks_new_keys() {
        let mut c = cfg();
        c.max_tag_cardinality = 2;
        let s = StatsSink::new(c);
        s.inc_counter(&MetricKey::new("a").with_tag("x", "1"), 1).unwrap();
        s.inc_counter(&MetricKey::new("a").with_tag("x", "2"), 1).unwrap();
        // Third unique key should fail.
        let r = s.inc_counter(&MetricKey::new("a").with_tag("x", "3"), 1);
        assert!(r.is_err());
    }

    #[test]
    fn cardinality_cap_existing_key_still_writable() {
        let mut c = cfg();
        c.max_tag_cardinality = 1;
        let s = StatsSink::new(c);
        let k = MetricKey::new("a").with_tag("x", "1");
        s.inc_counter(&k, 1).unwrap();
        // Same key, cardinality already at 1, should still write.
        s.inc_counter(&k, 1).unwrap();
        assert_eq!(s.counter_value(&k), 2);
    }

    #[test]
    fn standard_bounds_monotonic() {
        let b = HistogramBucket::standard_latency_bounds();
        for w in b.windows(2) {
            assert!(w[0] < w[1]);
        }
    }

    #[test]
    fn metric_json_roundtrip() {
        let s = StatsSink::new(cfg());
        s.inc_counter(&MetricKey::new("a").with_tag("svc", "b"), 3).unwrap();
        let snap = s.snapshot();
        let s = serde_json::to_string(&snap).unwrap();
        let s2: StreamMetricsMessage = serde_json::from_str(&s).unwrap();
        assert_eq!(snap.metrics.len(), s2.metrics.len());
    }

    #[test]
    fn metric_kind_json_tag() {
        let m = MetricKind::Counter { value: 7 };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains(r#""kind":"counter""#));
    }

    #[test]
    fn decanonicalize_round_trip() {
        let key = MetricKey::new("x").with_tag("a", "1").with_tag("b", "2");
        let canonical = key.canonical();
        let (name, tags) = decanonicalize(&canonical);
        assert_eq!(name, "x");
        assert_eq!(tags.get("a").unwrap(), "1");
        assert_eq!(tags.get("b").unwrap(), "2");
    }

    #[test]
    fn config_getter() {
        let c = cfg();
        let s = StatsSink::new(c.clone());
        assert_eq!(s.config().uri, c.uri);
    }
}
