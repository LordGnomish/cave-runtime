// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-metrics — Prometheus + VictoriaMetrics parity
//!
//! Compatible with: Prometheus, VictoriaMetrics
//! Provides:
//!   - TSDB with Gorilla XOR compression, inverted index, downsampling, retention
//!   - Full PromQL engine (all operators, aggregators, functions)
//!   - Prometheus HTTP API v1 (all /api/v1/* endpoints)
//!   - Remote write (protobuf+snappy), remote read
//!   - OpenMetrics, OTLP (gRPC+HTTP), StatsD, Graphite, InfluxDB ingestion
//!   - Alert rules, recording rules, AlertManager-compatible API
//!   - Service discovery (static, file-based, Kubernetes)
//!   - Federation (honor_labels, match[])

pub mod alertmgr;
pub mod api;
pub mod discovery_cloud;
pub mod error;
pub mod exemplars;
pub mod ingestion;
pub mod model;
pub mod multitenant;
pub mod notifier_sharded;
pub mod promql;
pub mod remote_read_backend;
pub mod rules;
pub mod scrape;
pub mod state;
pub mod template;
pub mod tsdb;

pub use error::{MetricsError, Result};
pub use model::{Labels, LabelMatcher, MatchOp, MetricType, QueryResult, Sample, TimeSeries};
pub use promql::{Engine, parse};
pub use tsdb::{Tsdb, TsdbConfig};
pub use state::MetricsState;
pub use discovery_cloud::{Target as CloudTarget, parse_hetzner_servers, parse_azure_vms};
pub use exemplars::{Exemplar, ExemplarRing, NativeHistogram};
pub use notifier_sharded::{Notification, PeerQueue, ShardedNotifier};
pub use remote_read_backend::{
    LabelMatcher as RemoteLabelMatcher, MatcherKind, MemoryReadBackend, ReadQuery,
    RemoteReadBackend, Sample as RemoteSample, SeriesSamples,
};
pub use template::{render as render_template, TemplateContext};

use axum::Router;
use std::sync::Arc;

/// Create the axum router for this module.
pub fn router(state: Arc<MetricsState>) -> Router {
    api::create_router(state)
}

pub const MODULE_NAME: &str = "metrics";

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Labels, LabelMatcher, Sample};
    use crate::tsdb::{Tsdb, TsdbConfig};
    use crate::promql::{parse, Engine};
    use crate::rules::{AlertRule, RecordingRule, AlertState};
    use crate::ingestion::{exposition, statsd, graphite, influx, otlp};
    use crate::ingestion::remote_write::{
        WriteRequest, ProtoTimeSeries, ProtoLabel, ProtoSample,
        encode_write_request, decode_write_request, write_request_to_batch,
    };
    use std::sync::Arc;

    // ─── Label / fingerprint tests ────────────────────────────────────────

    #[test]
    fn test_label_fingerprint_stable() {
        let l1 = Labels::from_pairs([("__name__", "cpu"), ("job", "api"), ("instance", "web01")]);
        let l2 = Labels::from_pairs([("instance", "web01"), ("job", "api"), ("__name__", "cpu")]);
        assert_eq!(l1.fingerprint(), l2.fingerprint());
    }

    #[test]
    fn test_label_matcher_equal() {
        let labels = Labels::from_pairs([("env", "prod"), ("app", "api")]);
        assert!(LabelMatcher::equal("env", "prod").matches(&labels));
        assert!(!LabelMatcher::equal("env", "staging").matches(&labels));
    }

    #[test]
    fn test_label_matcher_not_equal() {
        let labels = Labels::from_pairs([("env", "prod")]);
        assert!(LabelMatcher::not_equal("env", "staging").matches(&labels));
        assert!(!LabelMatcher::not_equal("env", "prod").matches(&labels));
    }

    #[test]
    fn test_label_matcher_regex() {
        let labels = Labels::from_pairs([("env", "production")]);
        let m = LabelMatcher::regex("env", "prod.*").unwrap();
        assert!(m.matches(&labels));
        let m2 = LabelMatcher::regex("env", "staging.*").unwrap();
        assert!(!m2.matches(&labels));
    }

    #[test]
    fn test_label_matcher_not_regex() {
        let labels = Labels::from_pairs([("env", "production")]);
        let m = LabelMatcher::not_regex("env", "staging.*").unwrap();
        assert!(m.matches(&labels));
    }

    #[test]
    fn test_labels_without_name() {
        let l = Labels::from_pairs([("__name__", "cpu"), ("job", "api")]);
        let without = l.without_name();
        assert_eq!(without.get("__name__"), None);
        assert_eq!(without.get("job"), Some("api"));
    }

    // ─── TSDB tests ───────────────────────────────────────────────────────

    #[test]
    fn test_tsdb_append_and_select() {
        let db = Tsdb::default();
        let labels = Labels::from_pairs([("__name__", "cpu"), ("job", "api")]);
        db.append(labels.clone(), Sample::new(1000, 0.5));
        db.append(labels.clone(), Sample::new(2000, 0.7));
        db.append(labels.clone(), Sample::new(3000, 0.9));

        let result = db.select(&[LabelMatcher::equal("__name__", "cpu")], 0, 5000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.len(), 3);
        assert_eq!(result[0].1[0].value, 0.5);
    }

    #[test]
    fn test_tsdb_select_range() {
        let db = Tsdb::default();
        let labels = Labels::from_pairs([("__name__", "mem")]);
        for i in 0..10 {
            db.append(labels.clone(), Sample::new(i * 1000, i as f64));
        }

        let result = db.select(&[LabelMatcher::equal("__name__", "mem")], 2000, 5000);
        assert_eq!(result[0].1.len(), 4); // ts 2000, 3000, 4000, 5000
    }

    #[test]
    fn test_tsdb_no_match() {
        let db = Tsdb::default();
        db.append(Labels::from_pairs([("__name__", "cpu")]), Sample::new(1000, 1.0));
        let result = db.select(&[LabelMatcher::equal("__name__", "nonexistent")], 0, 9999);
        assert!(result.is_empty());
    }

    #[test]
    fn test_tsdb_label_enumeration() {
        let db = Tsdb::default();
        db.append(Labels::from_pairs([("__name__", "cpu"), ("job", "api")]), Sample::new(1000, 1.0));
        db.append(Labels::from_pairs([("__name__", "mem"), ("job", "web")]), Sample::new(1000, 2.0));

        let names = db.label_names(&[]);
        assert!(names.contains(&"__name__".to_string()));
        assert!(names.contains(&"job".to_string()));

        let values = db.label_values("__name__", &[]);
        assert!(values.contains(&"cpu".to_string()));
        assert!(values.contains(&"mem".to_string()));
    }

    #[test]
    fn test_tsdb_retention() {
        let db = Tsdb::new(TsdbConfig {
            retention_ms: 5_000,
            ..TsdbConfig::default()
        });
        let labels = Labels::from_pairs([("__name__", "x")]);
        // "Far future" is now() in milliseconds — the sample must be inside the
        // retention window. Use an absolute wall-clock timestamp.
        let now_ms = chrono::Utc::now().timestamp_millis();
        db.append(labels.clone(), Sample::new(now_ms - 10, 1.0)); // recent
        db.append(labels.clone(), Sample::new(now_ms + 10_000, 2.0)); // future (always stays)

        db.enforce_retention();
        let result = db.select(&[LabelMatcher::equal("__name__", "x")], 0, i64::MAX);
        assert!(!result.is_empty()); // future sample must still be present
    }

    // ─── PromQL parser tests ──────────────────────────────────────────────

    #[test]
    fn test_parse_simple_selector() {
        let expr = parse("cpu_usage").unwrap();
        matches!(expr, crate::promql::ast::Expr::VectorSelector(_));
    }

    #[test]
    fn test_parse_label_matchers() {
        let expr = parse(r#"http_requests{method="GET", code!="500"}"#).unwrap();
        if let crate::promql::ast::Expr::VectorSelector(vs) = expr {
            assert!(vs.matchers.iter().any(|m| m.name == "__name__" && m.value == "http_requests"));
            assert!(vs.matchers.iter().any(|m| m.name == "method"));
            assert!(vs.matchers.iter().any(|m| m.name == "code"));
        }
    }

    #[test]
    fn test_parse_range_selector() {
        let expr = parse("rate(http_requests[5m])").unwrap();
        matches!(expr, crate::promql::ast::Expr::Call(_));
    }

    #[test]
    fn test_parse_aggregation() {
        let expr = parse("sum by(job) (http_requests)").unwrap();
        if let crate::promql::ast::Expr::Aggregate(agg) = expr {
            assert_eq!(agg.op, crate::promql::ast::AggregateOp::Sum);
            assert_eq!(agg.grouping.labels, vec!["job"]);
        }
    }

    #[test]
    fn test_parse_binary_op() {
        let expr = parse("cpu_total / cpu_count").unwrap();
        matches!(expr, crate::promql::ast::Expr::Binary(_));
    }

    #[test]
    fn test_parse_offset() {
        let expr = parse("cpu_usage offset 5m").unwrap();
        if let crate::promql::ast::Expr::VectorSelector(vs) = expr {
            assert_eq!(vs.offset, Some(5 * 60 * 1000));
        }
    }

    // ─── PromQL eval tests ────────────────────────────────────────────────

    fn seed_db() -> Arc<Tsdb> {
        let db = Arc::new(Tsdb::default());
        let labels = Labels::from_pairs([("__name__", "requests"), ("job", "api")]);
        db.append(labels.clone(), Sample::new(0, 0.0));
        db.append(labels.clone(), Sample::new(10_000, 10.0));
        db.append(labels.clone(), Sample::new(20_000, 30.0));
        db.append(labels.clone(), Sample::new(30_000, 60.0));

        let labels2 = Labels::from_pairs([("__name__", "requests"), ("job", "web")]);
        db.append(labels2.clone(), Sample::new(0, 0.0));
        db.append(labels2.clone(), Sample::new(10_000, 5.0));
        db.append(labels2.clone(), Sample::new(20_000, 15.0));
        db.append(labels2.clone(), Sample::new(30_000, 30.0));
        db
    }

    #[test]
    fn test_eval_instant_vector() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("requests").unwrap();
        let result = engine.eval_instant(&ast, 30_000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert_eq!(iv.len(), 2);
        } else {
            panic!("expected instant vector");
        }
    }

    #[test]
    fn test_eval_sum_aggregation() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("sum(requests)").unwrap();
        let result = engine.eval_instant(&ast, 30_000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].1, 90.0); // 60 + 30
        } else {
            panic!("expected instant vector");
        }
    }

    #[test]
    fn test_eval_rate() {
        let db = Arc::new(Tsdb::default());
        let labels = Labels::from_pairs([("__name__", "counter")]);
        db.append(labels.clone(), Sample::new(0, 0.0));
        db.append(labels.clone(), Sample::new(60_000, 60.0)); // rate should be ~1/s

        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("rate(counter[2m])").unwrap();
        let result = engine.eval_instant(&ast, 60_000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert!(!iv.is_empty());
            let rate = iv[0].1;
            assert!(rate > 0.0);
        }
    }

    #[test]
    fn test_eval_histogram_quantile() {
        let db = Arc::new(Tsdb::default());
        // Seed histogram _bucket series
        for (le, count) in [("0.1", 10.0), ("0.5", 50.0), ("1.0", 80.0), ("+Inf", 100.0)] {
            let labels = Labels::from_pairs([
                ("__name__", "latency_bucket"),
                ("job", "api"),
                ("le", le),
            ]);
            db.append(labels, Sample::new(1000, count));
        }

        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("histogram_quantile(0.5, latency_bucket)").unwrap();
        let result = engine.eval_instant(&ast, 2000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert!(!iv.is_empty());
            let p50 = iv[0].1;
            assert!(p50 >= 0.0 && p50 <= 1.0);
        }
    }

    #[test]
    fn test_eval_binary_op_scalar() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("requests * 2").unwrap();
        let result = engine.eval_instant(&ast, 30_000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            for (_, v) in &iv {
                assert!(*v == 120.0 || *v == 60.0); // 60*2 or 30*2
            }
        }
    }

    #[test]
    fn test_eval_topk() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("topk(1, requests)").unwrap();
        let result = engine.eval_instant(&ast, 30_000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].1, 60.0);
        }
    }

    #[test]
    fn test_eval_label_replace() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ast = parse(r#"label_replace(requests, "env", "production", "job", "api")"#).unwrap();
        let result = engine.eval_instant(&ast, 30_000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert!(!iv.is_empty());
            let api_series: Vec<_> = iv.iter().filter(|(l, _)| l.get("job") == Some("api")).collect();
            if !api_series.is_empty() {
                assert_eq!(api_series[0].0.get("env"), Some("production"));
            }
        }
    }

    #[test]
    fn test_eval_absent_present() {
        let db = Arc::new(Tsdb::default());
        let engine = Engine::new(Arc::clone(&db));

        // absent on missing series → returns 1
        let ast = parse("absent(missing_metric)").unwrap();
        let result = engine.eval_instant(&ast, 1000).unwrap();
        if let QueryResult::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].1, 1.0);
        }
    }

    #[test]
    fn test_eval_all_math_functions() {
        let db = Arc::new(Tsdb::default());
        let labels = Labels::from_pairs([("__name__", "x")]);
        db.append(labels.clone(), Sample::new(1000, 4.0));
        let engine = Engine::new(Arc::clone(&db));

        for (func, expected) in [
            ("abs(-4)",  "abs"),
            ("ceil(1.1)", "ceil"),
            ("floor(1.9)", "floor"),
            ("sqrt(x)", "sqrt"),
            ("ln(x)", "ln"),
            ("exp(x)", "exp"),
        ] {
            let ast = parse(func).unwrap();
            let _ = engine.eval_instant(&ast, 2000).unwrap(); // just check no panic
        }
    }

    #[test]
    fn test_eval_time_functions() {
        let db = Arc::new(Tsdb::default());
        let engine = Engine::new(Arc::clone(&db));
        let ts_ms = 1_700_000_000_000i64;

        for func in ["time()", "hour()", "minute()", "day_of_week()", "month()", "year()"] {
            let ast = parse(func).unwrap();
            let _ = engine.eval_instant(&ast, ts_ms).unwrap();
        }
    }

    #[test]
    fn test_eval_aggregations_all() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ts_ms = 30_000;

        for agg in ["sum", "min", "max", "avg", "count", "stddev", "stdvar", "group"] {
            let query = format!("{}(requests)", agg);
            let ast = parse(&query).unwrap();
            let result = engine.eval_instant(&ast, ts_ms).unwrap();
            assert!(matches!(result, QueryResult::InstantVector(_)), "Failed: {}", agg);
        }
    }

    #[test]
    fn test_eval_subquery() {
        let db = seed_db();
        let engine = Engine::new(Arc::clone(&db));
        let ast = parse("avg_over_time(requests[10m:1m])").unwrap();
        let _ = engine.eval_instant(&ast, 30_000).unwrap();
    }

    // ─── Gorilla compression tests ─────────────────────────────────────────

    #[test]
    fn test_gorilla_compression_roundtrip() {
        use crate::tsdb::block::{ChunkWriter, ChunkReader};
        let samples = vec![
            (1_000_000_000i64, 1.0f64),
            (1_000_015_000,    1.5),
            (1_000_030_000,    2.0),
            (1_000_045_000,    2.0), // same value — XOR = 0
            (1_000_060_000,    -1.0),
        ];
        let mut w = ChunkWriter::new();
        for (ts, v) in &samples {
            w.append(*ts, *v);
        }
        let (count, data) = w.finish();
        assert_eq!(count as usize, samples.len());

        let decoded = ChunkReader::new(count, &data).decode_all();
        for (i, (ts, v)) in decoded.iter().enumerate() {
            assert_eq!(*ts, samples[i].0);
            assert_eq!(*v, samples[i].1);
        }
    }

    // ─── Ingestion: remote_write ──────────────────────────────────────────

    #[test]
    fn test_remote_write_encode_decode() {
        let req = WriteRequest {
            timeseries: vec![ProtoTimeSeries {
                labels: vec![
                    ProtoLabel { name: "__name__".into(), value: "test_metric".into() },
                    ProtoLabel { name: "job".into(), value: "api".into() },
                ],
                samples: vec![ProtoSample { value: 42.0, timestamp: 1_000_000 }],
                exemplars: vec![],
            }],
            metadata: vec![],
        };

        let encoded = encode_write_request(&req).unwrap();
        let decoded = decode_write_request(&encoded).unwrap();
        assert_eq!(decoded.timeseries[0].samples[0].value, 42.0);
    }

    #[test]
    fn test_remote_write_to_batch() {
        let req = WriteRequest {
            timeseries: vec![ProtoTimeSeries {
                labels: vec![ProtoLabel { name: "__name__".into(), value: "cpu".into() }],
                samples: vec![ProtoSample { value: 0.5, timestamp: 1000 }],
                exemplars: vec![],
            }],
            metadata: vec![],
        };
        let batch = write_request_to_batch(req);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].labels.metric_name(), Some("cpu"));
    }

    // ─── Ingestion: Prometheus exposition format ────────────────────────────

    #[test]
    fn test_exposition_parse_counter() {
        let input = "# TYPE http_requests counter\nhttp_requests{method=\"GET\"} 100 1000\n";
        let batch = exposition::parse(input).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].samples[0].value, 100.0);
    }

    #[test]
    fn test_exposition_parse_multi_labels() {
        let input = r#"latency_bucket{le="0.5",job="api"} 42 2000
"#;
        let batch = exposition::parse(input).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].labels.get("le"), Some("0.5"));
    }

    #[test]
    fn test_exposition_parse_histogram() {
        let input = r#"
# HELP req_duration Request duration
# TYPE req_duration histogram
req_duration_bucket{le="0.1"} 10
req_duration_bucket{le="0.5"} 50
req_duration_bucket{le="+Inf"} 100
req_duration_count 100
req_duration_sum 25.3
"#;
        let batch = exposition::parse(input).unwrap();
        assert!(batch.len() >= 3);
    }

    // ─── Ingestion: StatsD ────────────────────────────────────────────────

    #[test]
    fn test_statsd_counter() {
        let batch = statsd::parse_batch("page.views:1|c|@0.5|#env:prod");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].samples[0].value, 2.0); // 1 / 0.5
    }

    #[test]
    fn test_statsd_gauge() {
        let batch = statsd::parse_batch("memory.heap:1024|g");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].samples[0].value, 1024.0);
    }

    // ─── Ingestion: Graphite ──────────────────────────────────────────────

    #[test]
    fn test_graphite_parse() {
        let batch = graphite::parse_batch("servers.web.cpu 0.85 1609459200");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].samples[0].value, 0.85);
        assert_eq!(batch[0].samples[0].timestamp_ms, 1609459200000);
    }

    // ─── Ingestion: InfluxDB ──────────────────────────────────────────────

    #[test]
    fn test_influx_parse_basic() {
        let ts = influx::parse_line("cpu,host=web01 usage=0.85 1609459200000000000");
        assert_eq!(ts.len(), 1);
        assert!((ts[0].samples[0].value - 0.85).abs() < 1e-9);
    }

    #[test]
    fn test_influx_parse_multiple_fields() {
        let ts = influx::parse_line("mem,host=web01 used=1024i,free=4096i 1000000000000");
        assert_eq!(ts.len(), 2);
    }

    #[test]
    fn test_influx_parse_boolean() {
        let ts = influx::parse_line("sensor alive=t 1000000000000");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].samples[0].value, 1.0);
    }

    // ─── Recording rules ──────────────────────────────────────────────────

    #[test]
    fn test_recording_rule() {
        let db = Arc::new(Tsdb::default());
        db.append(
            Labels::from_pairs([("__name__", "http_req"), ("job", "api")]),
            Sample::new(1000, 10.0),
        );
        db.append(
            Labels::from_pairs([("__name__", "http_req"), ("job", "web")]),
            Sample::new(1000, 5.0),
        );

        let engine = Engine::new(Arc::clone(&db));
        let rule = RecordingRule::new("job:http_req:sum", "sum(http_req)");
        rule.evaluate(&engine, &db, 2000).unwrap();

        let recorded = db.select(
            &[LabelMatcher::equal("__name__", "job:http_req:sum")],
            0, 3000,
        );
        assert!(!recorded.is_empty());
        // sum should be 15
        assert!((recorded[0].1[0].value - 15.0).abs() < 1e-9);
    }

    // ─── Alerting rules ───────────────────────────────────────────────────

    #[test]
    fn test_alert_rule_pending_to_firing() {
        let db = Arc::new(Tsdb::default());
        db.append(
            Labels::from_pairs([("__name__", "error_rate")]),
            Sample::new(0, 1.0),
        );
        db.append(
            Labels::from_pairs([("__name__", "error_rate")]),
            Sample::new(30_000, 1.0),
        );
        db.append(
            Labels::from_pairs([("__name__", "error_rate")]),
            Sample::new(60_000, 1.0),
        );

        let engine = Engine::new(Arc::clone(&db));
        let mut rule = AlertRule::new("HighError", "error_rate > 0", 60_000); // for: 1m

        let alerts = rule.evaluate(&engine, 0).unwrap();
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].state, AlertState::Pending);

        let alerts = rule.evaluate(&engine, 60_001).unwrap();
        assert_eq!(alerts[0].state, AlertState::Firing);
    }

    #[test]
    fn test_alert_rule_resolves() {
        let db = Arc::new(Tsdb::default());
        db.append(
            Labels::from_pairs([("__name__", "val")]),
            Sample::new(0, 5.0),
        );

        let engine = Engine::new(Arc::clone(&db));
        let mut rule = AlertRule::new("HighVal", "val > 4", 0);

        let alerts = rule.evaluate(&engine, 5_000).unwrap();
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].state, AlertState::Firing);

        // Now the metric is gone from the lookback window — rule should resolve
        let alerts_after = rule.evaluate(&engine, 1_000_000).unwrap();
        assert!(alerts_after.is_empty()); // no active series
    }

    // ─── Functions ────────────────────────────────────────────────────────

    #[test]
    fn test_fn_rate_basic() {
        let samples = vec![
            Sample::new(0, 0.0),
            Sample::new(60_000, 60.0),
        ];
        let r = crate::promql::functions::rate(&samples, 60_000).unwrap();
        assert!((r - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_fn_irate_basic() {
        let samples = vec![
            Sample::new(0, 0.0),
            Sample::new(10_000, 100.0),
        ];
        let r = crate::promql::functions::irate(&samples).unwrap();
        assert!((r - 10.0).abs() < 0.1); // 100 / 10s
    }

    #[test]
    fn test_fn_delta() {
        let samples = vec![Sample::new(0, 10.0), Sample::new(60_000, 20.0)];
        let d = crate::promql::functions::delta(&samples, 60_000).unwrap();
        assert!((d - 10.0).abs() < 0.1);
    }

    #[test]
    fn test_fn_deriv() {
        // Straight line: y = t  → slope = 1 per second
        let samples: Vec<Sample> = (0..5).map(|i| Sample::new(i * 1000, i as f64)).collect();
        let d = crate::promql::functions::deriv(&samples).unwrap();
        assert!((d - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_fn_predict_linear() {
        let samples = vec![Sample::new(0, 0.0), Sample::new(10_000, 10.0)];
        let pred = crate::promql::functions::predict_linear(&samples, 10.0).unwrap();
        assert!((pred - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_fn_resets() {
        let samples = vec![
            Sample::new(0, 5.0),
            Sample::new(1000, 10.0),
            Sample::new(2000, 2.0),  // reset
            Sample::new(3000, 15.0),
            Sample::new(4000, 3.0),  // reset
        ];
        assert_eq!(crate::promql::functions::resets(&samples), 2.0);
    }

    #[test]
    fn test_fn_changes() {
        let samples = vec![
            Sample::new(0, 1.0),
            Sample::new(1000, 1.0),
            Sample::new(2000, 2.0),
            Sample::new(3000, 2.0),
            Sample::new(4000, 3.0),
        ];
        assert_eq!(crate::promql::functions::changes(&samples), 2.0);
    }

    #[test]
    fn test_fn_histogram_quantile() {
        let buckets = vec![
            (0.1, 10.0),
            (0.5, 50.0),
            (1.0, 80.0),
            (f64::INFINITY, 100.0),
        ];
        let p50 = crate::promql::functions::histogram_quantile(0.5, buckets);
        assert!(p50 > 0.0 && p50 <= 0.5);
    }

    #[test]
    fn test_fn_quantile_sorted() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(crate::promql::functions::quantile_sorted(0.0, &data), 1.0);
        assert_eq!(crate::promql::functions::quantile_sorted(1.0, &data), 5.0);
        let p50 = crate::promql::functions::quantile_sorted(0.5, &data);
        assert!((p50 - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_fn_over_time_aggregations() {
        let samples: Vec<Sample> = vec![
            Sample::new(0, 1.0),
            Sample::new(1000, 2.0),
            Sample::new(2000, 3.0),
            Sample::new(3000, 4.0),
        ];
        use crate::promql::functions::*;
        assert_eq!(avg_over_time(&samples).unwrap(), 2.5);
        assert_eq!(min_over_time(&samples).unwrap(), 1.0);
        assert_eq!(max_over_time(&samples).unwrap(), 4.0);
        assert_eq!(sum_over_time(&samples).unwrap(), 10.0);
        assert_eq!(count_over_time(&samples).unwrap(), 4.0);
        assert_eq!(last_over_time(&samples).unwrap(), 4.0);
        assert_eq!(present_over_time(&samples).unwrap(), 1.0);
    }

    #[test]
    fn test_fn_label_replace() {
        let labels = Labels::from_pairs([("__name__", "cpu"), ("job", "api-v2")]);
        let result = crate::promql::functions::label_replace(
            &labels, "service", "$1", "job", r"(\w+)-.*"
        ).unwrap();
        assert_eq!(result.get("service"), Some("api"));
    }

    #[test]
    fn test_fn_label_join() {
        let labels = Labels::from_pairs([
            ("namespace", "prod"),
            ("service", "api"),
        ]);
        let result = crate::promql::functions::label_join(&labels, "fqdn", ".", &["namespace", "service"]);
        assert_eq!(result.get("fqdn"), Some("prod.api"));
    }

    // ─── Downsampling tests ────────────────────────────────────────────────

    #[test]
    fn test_downsample_averaging() {
        use crate::tsdb::compaction::downsample_series;
        let samples = vec![
            Sample::new(0, 1.0),
            Sample::new(1_000, 3.0),
            Sample::new(5_000, 2.0),
            Sample::new(6_000, 4.0),
        ];
        let ds = downsample_series(&samples, 5_000);
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].value, 2.0); // avg(1, 3)
        assert_eq!(ds[1].value, 3.0); // avg(2, 4)
    }

    #[test]
    fn test_merge_samples() {
        use crate::tsdb::compaction::merge_samples;
        let a = vec![Sample::new(1, 1.0), Sample::new(3, 3.0)];
        let b = vec![Sample::new(2, 2.0), Sample::new(3, 3.5)];
        let m = merge_samples(&a, &b);
        assert_eq!(m.len(), 3);
        assert_eq!(m[1].value, 2.0);
        assert_eq!(m[2].value, 3.5); // b wins
    }

    // ─── Deep parity: PromQL functions (rate / irate / increase / deriv) ──

    #[test]
    fn parity_rate_handles_counter_reset() {
        use crate::promql::functions::rate;
        // Counter resets to 0 between samples — rate should still be positive.
        let samples = vec![
            Sample::new(0,    100.0),
            Sample::new(1000, 110.0),
            Sample::new(2000,   5.0), // reset
            Sample::new(3000,  20.0),
        ];
        let r = rate(&samples, 3000).unwrap();
        // Without the reset, total would be 20-100 = -80 (nonsense).
        // With the reset, we add 110 (the pre-reset peak) so delta = (20-100) + 110 = 30.
        assert!(r > 0.0);
    }

    #[test]
    fn parity_irate_uses_only_last_two_samples() {
        use crate::promql::functions::irate;
        let samples = vec![
            Sample::new(0, 0.0),
            Sample::new(1000, 5.0),
            Sample::new(3000, 25.0), // gap of 2 seconds, +20
        ];
        let r = irate(&samples).unwrap();
        // 20 / 2s = 10/s
        assert!((r - 10.0).abs() < 0.01);
    }

    #[test]
    fn parity_increase_scales_with_range() {
        use crate::promql::functions::{increase, rate};
        let samples = vec![
            Sample::new(0, 0.0),
            Sample::new(2000, 10.0),
        ];
        let r = rate(&samples, 2000).unwrap();
        let i = increase(&samples, 2000).unwrap();
        // increase = rate * range_seconds
        assert!((i - r * 2.0).abs() < 0.01);
    }

    #[test]
    fn parity_deriv_returns_slope_per_second() {
        use crate::promql::functions::deriv;
        // Linear y = 2x (in seconds): values at t=0,1000,2000 → 0,2,4
        let samples = vec![
            Sample::new(0, 0.0),
            Sample::new(1000, 2.0),
            Sample::new(2000, 4.0),
        ];
        let d = deriv(&samples).unwrap();
        assert!((d - 2.0).abs() < 0.01);
    }

    #[test]
    fn parity_predict_linear_extrapolates_future_value() {
        use crate::promql::functions::predict_linear;
        let samples = vec![
            Sample::new(0, 0.0),
            Sample::new(1000, 1.0),
            Sample::new(2000, 2.0),
        ];
        let pred = predict_linear(&samples, 5.0).unwrap();
        // slope=1/s, last_t=2s, last_v=2, predict at t=2+5=7 → value=7
        assert!((pred - 7.0).abs() < 0.1);
    }

    #[test]
    fn parity_resets_counts_counter_resets() {
        use crate::promql::functions::resets;
        let samples = vec![
            Sample::new(0,   10.0),
            Sample::new(100, 20.0),
            Sample::new(200,  5.0), // reset
            Sample::new(300, 30.0),
            Sample::new(400,  1.0), // reset
        ];
        assert_eq!(resets(&samples), 2.0);
    }

    #[test]
    fn parity_changes_counts_value_changes() {
        use crate::promql::functions::changes;
        let samples = vec![
            Sample::new(0, 1.0),
            Sample::new(1, 1.0),
            Sample::new(2, 2.0),
            Sample::new(3, 2.0),
            Sample::new(4, 3.0),
        ];
        assert_eq!(changes(&samples), 2.0);
    }

    #[test]
    fn parity_avg_over_time_correct() {
        use crate::promql::functions::avg_over_time;
        let samples = vec![Sample::new(0, 1.0), Sample::new(1, 2.0), Sample::new(2, 3.0)];
        assert_eq!(avg_over_time(&samples), Some(2.0));
    }

    #[test]
    fn parity_quantile_over_time_p95() {
        use crate::promql::functions::quantile_over_time;
        let samples: Vec<Sample> = (0..=100)
            .map(|i| Sample::new(i as i64, i as f64))
            .collect();
        let q = quantile_over_time(0.95, &samples).unwrap();
        // 95th percentile of 0..=100 should be ~95
        assert!((q - 95.0).abs() < 1.0);
    }

    #[test]
    fn parity_stddev_over_time_correct() {
        use crate::promql::functions::stddev_over_time;
        // [1, 2, 3, 4, 5] → mean=3, variance=2, stddev≈1.414
        let samples: Vec<Sample> = (1..=5).map(|i| Sample::new(i, i as f64)).collect();
        let s = stddev_over_time(&samples).unwrap();
        assert!((s - 2f64.sqrt()).abs() < 0.01);
    }

    // ─── Deep parity: TSDB head series + select ───────────────────────────

    #[test]
    fn parity_tsdb_head_series_samples_in_range_filters_correctly() {
        use crate::tsdb::HeadSeries;
        let labels = Labels::from_pairs([("__name__", "x")]);
        let mut hs = HeadSeries::new(labels);
        hs.append(Sample::new(100, 1.0));
        hs.append(Sample::new(200, 2.0));
        hs.append(Sample::new(300, 3.0));
        let in_range = hs.samples_in_range(150, 250);
        assert_eq!(in_range.len(), 1);
        assert_eq!(in_range[0].timestamp_ms, 200);
    }

    #[test]
    fn parity_tsdb_select_at_returns_latest_in_lookback() {
        let db = Tsdb::default();
        let labels = Labels::from_pairs([("__name__", "x"), ("job", "api")]);
        db.append(labels.clone(), Sample::new(900, 0.9));
        db.append(labels.clone(), Sample::new(950, 0.95));
        db.append(labels, Sample::new(1000, 1.0));
        let result = db.select_at(&[LabelMatcher::equal("__name__", "x")], 1000, 100);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.value, 1.0);
    }

    #[test]
    fn parity_tsdb_label_values_filtered_by_matchers() {
        let db = Tsdb::default();
        db.append(Labels::from_pairs([("__name__", "cpu"), ("env", "prod")]), Sample::new(1000, 1.0));
        db.append(Labels::from_pairs([("__name__", "cpu"), ("env", "dev")]), Sample::new(1000, 1.0));
        db.append(Labels::from_pairs([("__name__", "mem"), ("env", "prod")]), Sample::new(1000, 1.0));
        let cpu_envs = db.label_values("env", &[LabelMatcher::equal("__name__", "cpu")]);
        assert!(cpu_envs.contains(&"prod".to_string()));
        assert!(cpu_envs.contains(&"dev".to_string()));
        assert_eq!(cpu_envs.len(), 2);
    }

    #[test]
    fn parity_tsdb_series_for_returns_distinct_label_sets() {
        let db = Tsdb::default();
        db.append(Labels::from_pairs([("__name__", "cpu"), ("inst", "a")]), Sample::new(1000, 1.0));
        db.append(Labels::from_pairs([("__name__", "cpu"), ("inst", "b")]), Sample::new(1000, 2.0));
        let series = db.series_for(&[LabelMatcher::equal("__name__", "cpu")]);
        assert_eq!(series.len(), 2);
    }

    #[test]
    fn parity_tsdb_append_many_via_timeseries() {
        let db = Tsdb::default();
        let ts = TimeSeries {
            labels: Labels::from_pairs([("__name__", "y")]),
            samples: vec![Sample::new(100, 1.0), Sample::new(200, 2.0), Sample::new(300, 3.0)],
        };
        db.append_many(&ts);
        let r = db.select(&[LabelMatcher::equal("__name__", "y")], 0, i64::MAX);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1.len(), 3);
    }

    // ─── Deep parity: Block + WAL ─────────────────────────────────────────

    #[test]
    fn parity_block_writer_reader_roundtrip() {
        use crate::tsdb::block::{ChunkWriter, ChunkReader};
        let mut enc = ChunkWriter::new();
        let pairs = vec![(1000i64, 1.0f64), (2000, 2.5), (3000, 3.7), (4000, 4.2)];
        for (t, v) in &pairs {
            enc.append(*t, *v);
        }
        let (count, data) = enc.finish();
        assert_eq!(count, 4);
        let dec = ChunkReader::new(count, &data);
        let decoded = dec.decode_all();
        assert_eq!(decoded.len(), 4);
        for (i, (t, v)) in pairs.iter().enumerate() {
            assert_eq!(decoded[i].0, *t);
            assert!((decoded[i].1 - v).abs() < 0.0001);
        }
    }

    #[test]
    fn parity_wal_record_serializable() {
        use crate::tsdb::wal::WalRecord;
        let rec = WalRecord::Sample {
            labels: std::collections::BTreeMap::from([
                ("__name__".to_string(), "cpu".to_string()),
            ]),
            timestamp_ms: 1000,
            value: 0.5,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"cpu\""));
        let parsed: WalRecord = serde_json::from_str(&json).unwrap();
        match parsed {
            WalRecord::Sample { timestamp_ms, value, .. } => {
                assert_eq!(timestamp_ms, 1000);
                assert_eq!(value, 0.5);
            }
            _ => panic!("expected Sample"),
        }
    }

    // ─── Deep parity: Multi-tenant + remote write ─────────────────────────

    #[test]
    fn parity_multitenant_enforce_filter_idempotent() {
        use crate::multitenant::{enforce_tenant_filter, TENANT_LABEL};
        let m1 = enforce_tenant_filter(vec![LabelMatcher::equal("__name__", "x")], "acme");
        let m2 = enforce_tenant_filter(m1.clone(), "acme");
        let count = m2.iter().filter(|m| m.name == TENANT_LABEL).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn parity_multitenant_federation_relabel_external_only_label() {
        use crate::multitenant::federation_relabel;
        let src = Labels::default();
        let ext = Labels::from_pairs([("k8s_cluster", "us-east-1")]);
        let out = federation_relabel(&src, &ext, false);
        assert_eq!(out.get("k8s_cluster"), Some("us-east-1"));
    }

    #[test]
    fn parity_remote_write_protobuf_roundtrip() {
        use crate::ingestion::remote_write::{
            batch_to_write_request, encode_write_request, decode_write_request,
            write_request_to_batch,
        };
        let batch: Vec<TimeSeries> = vec![TimeSeries {
            labels: Labels::from_pairs([("__name__", "cpu"), ("job", "api")]),
            samples: vec![Sample::new(1000, 0.5), Sample::new(2000, 0.6)],
        }];
        let req = batch_to_write_request(batch);
        let bytes = encode_write_request(&req).unwrap();
        let decoded_req = decode_write_request(&bytes).unwrap();
        let decoded_batch = write_request_to_batch(decoded_req);
        assert_eq!(decoded_batch.len(), 1);
        assert_eq!(decoded_batch[0].samples.len(), 2);
        assert_eq!(decoded_batch[0].labels.get("__name__"), Some("cpu"));
    }

    // ─── Deep parity: Recording + alerting rules ──────────────────────────

    #[test]
    fn parity_recording_rule_evaluates_and_writes_to_tsdb() {
        let db = Arc::new(Tsdb::default());
        let labels = Labels::from_pairs([("__name__", "raw"), ("job", "api")]);
        db.append(labels, Sample::new(1000, 5.0));

        let engine = Engine::new(db.clone());
        let rule = RecordingRule::new("raw_aggregated", "raw")
            .with_labels(Labels::from_pairs([("derived", "true")]));
        rule.evaluate(&engine, &db, 1100).unwrap();

        let recorded = db.select(
            &[LabelMatcher::equal("__name__", "raw_aggregated")],
            0, i64::MAX,
        );
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0.get("derived"), Some("true"));
    }

    #[test]
    fn parity_alert_rule_pending_then_firing_after_for_window() {
        let db = Arc::new(Tsdb::default());
        let labels = Labels::from_pairs([("__name__", "errors")]);
        for ts in (1000..=5000).step_by(1000) {
            db.append(labels.clone(), Sample::new(ts, 10.0));
        }
        let engine = Engine::new(db.clone());
        // for=2000ms (2s)
        let mut rule = AlertRule::new("HighErrors", "errors", 2000);

        let firing_at_t1000 = rule.evaluate(&engine, 1000).unwrap();
        // First evaluation reports the alert in Pending state
        assert_eq!(firing_at_t1000.len(), 1);
        assert_eq!(firing_at_t1000[0].state, AlertState::Pending);

        let firing_at_t4000 = rule.evaluate(&engine, 4000).unwrap();
        // 3000ms past pending start → firing
        assert_eq!(firing_at_t4000.len(), 1);
        assert_eq!(firing_at_t4000[0].state, AlertState::Firing);
    }

    #[test]
    fn parity_alert_rule_clears_active_when_expression_no_longer_matches() {
        let db = Arc::new(Tsdb::default());
        let labels = Labels::from_pairs([("__name__", "live")]);
        db.append(labels.clone(), Sample::new(1000, 10.0));
        let engine = Engine::new(db.clone());
        let mut rule = AlertRule::new("Active", "live", 0);
        let alerts = rule.evaluate(&engine, 1000).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].state, AlertState::Firing);
        // Empty TSDB → expression no longer matches → alerts evicted
        let db2 = Arc::new(Tsdb::default());
        let engine2 = Engine::new(db2);
        let after = rule.evaluate(&engine2, 2000).unwrap();
        assert!(after.is_empty());
        assert!(rule.active.is_empty());
    }

    // ─── Deep parity: AlertManager silence store ──────────────────────────

    #[test]
    fn parity_silence_store_create_get_list() {
        use crate::alertmgr::silence::SilenceStore;
        use crate::alertmgr::model::{Silence, SilenceMatcher, SilenceStatus};
        let store = SilenceStore::new();
        let s = Silence {
            id: String::new(), // auto-assigned
            matchers: vec![SilenceMatcher {
                name: "alertname".to_string(), value: "Foo".to_string(),
                is_regex: false, is_equal: true,
            }],
            starts_at: "2026-01-01T00:00:00Z".to_string(),
            ends_at: "2030-01-01T00:00:00Z".to_string(),
            created_by: "test".to_string(),
            comment: "test".to_string(),
            status: SilenceStatus { state: "active".to_string() },
        };
        let id = store.create(s);
        assert!(!id.is_empty());
        assert!(store.get(&id).is_some());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn parity_silence_matches_active_window_and_labels() {
        use crate::alertmgr::silence::SilenceStore;
        use crate::alertmgr::model::{Silence, SilenceMatcher, SilenceStatus};
        let store = SilenceStore::new();
        store.create(Silence {
            id: String::new(),
            matchers: vec![SilenceMatcher {
                name: "alertname".to_string(), value: "Foo".to_string(),
                is_regex: false, is_equal: true,
            }],
            starts_at: "2026-01-01T00:00:00Z".to_string(),
            ends_at: "2030-01-01T00:00:00Z".to_string(),
            created_by: "x".to_string(), comment: "x".to_string(),
            status: SilenceStatus { state: "active".to_string() },
        });
        let foo_labels = Labels::from_pairs([("alertname", "Foo")]);
        let bar_labels = Labels::from_pairs([("alertname", "Bar")]);
        assert!(store.is_silenced(&foo_labels, "2026-06-01T00:00:00Z"));
        assert!(!store.is_silenced(&bar_labels, "2026-06-01T00:00:00Z"));
        // outside the window
        assert!(!store.is_silenced(&foo_labels, "2025-01-01T00:00:00Z"));
        assert!(!store.is_silenced(&foo_labels, "2031-01-01T00:00:00Z"));
    }

    #[test]
    fn parity_silence_expire_drops_to_expired_state() {
        use crate::alertmgr::silence::SilenceStore;
        use crate::alertmgr::model::{Silence, SilenceMatcher, SilenceStatus};
        let store = SilenceStore::new();
        let id = store.create(Silence {
            id: String::new(), matchers: vec![SilenceMatcher {
                name: "x".to_string(), value: "y".to_string(),
                is_regex: false, is_equal: true,
            }],
            starts_at: "2020-01-01T00:00:00Z".to_string(),
            ends_at: "2030-01-01T00:00:00Z".to_string(),
            created_by: "x".to_string(), comment: "x".to_string(),
            status: SilenceStatus { state: "active".to_string() },
        });
        assert!(store.expire(&id));
        let s = store.get(&id).unwrap();
        assert_eq!(s.status.state, "expired");
        // Expired silence no longer silences
        let labels = Labels::from_pairs([("x", "y")]);
        assert!(!store.is_silenced(&labels, "2026-06-01T00:00:00Z"));
    }

    // ─── Deep parity: ingestion formats ───────────────────────────────────

    #[test]
    fn parity_ingestion_statsd_parses_counter_with_sample_rate() {
        // statsd format: "name:value|c|@rate"
        let pkt = statsd::parse_packet("api.requests:42|c|@0.5").unwrap();
        // sample_rate = 0.5 means observed value 42 was at 50% sampling →
        // accumulated value should reflect the rate.
        assert_eq!(pkt.name, "api.requests");
        assert!((pkt.sample_rate - 0.5).abs() < 0.001);
    }

    #[test]
    fn parity_ingestion_graphite_handles_dotted_metric_name() {
        let ts = graphite::parse_line("servers.web01.cpu 0.5 1620000000").unwrap();
        assert_eq!(ts.samples[0].value, 0.5);
        // Graphite dot-paths usually become __name__ joined with underscores
        assert!(ts.labels.get("__name__").is_some());
    }

    #[test]
    fn parity_ingestion_influx_line_protocol_multi_tags() {
        let series = influx::parse_line("cpu,host=web01,region=us value=0.5 1620000000000000000");
        assert!(!series.is_empty());
        assert!(series[0].labels.get("host").is_some());
        assert!(series[0].labels.get("region").is_some());
    }

    #[test]
    fn parity_ingestion_exposition_handles_inf_nan() {
        let body = "# HELP weird metric\n# TYPE weird gauge\nweird +Inf\nweird_nan NaN\n";
        let batch = exposition::parse(body).unwrap();
        // Both lines should parse — Inf and NaN are valid Prometheus values
        let any_inf = batch.iter().any(|ts|
            ts.samples.iter().any(|s| s.value.is_infinite()));
        let any_nan = batch.iter().any(|ts|
            ts.samples.iter().any(|s| s.value.is_nan()));
        assert!(any_inf, "expected an infinity sample");
        assert!(any_nan, "expected a NaN sample");
    }
}
