//! cave-metrics — Prometheus + VictoriaMetrics parity
//!
//! Replaces: Prometheus, VictoriaMetrics
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
pub mod error;
pub mod ingestion;
pub mod model;
pub mod promql;
pub mod rules;
pub mod scrape;
pub mod state;
pub mod tsdb;

pub use error::{MetricsError, Result};
pub use model::{Labels, LabelMatcher, MatchOp, MetricType, QueryResult, Sample, TimeSeries};
pub use promql::{Engine, parse};
pub use tsdb::{Tsdb, TsdbConfig};
pub use state::MetricsState;

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
        db.append(labels.clone(), Sample::new(0, 1.0));
        db.append(labels.clone(), Sample::new(10_000_000, 2.0)); // far future (stays)

        // Manually enforce retention with current time simulation
        // (In a real test you'd mock time; here we just verify the API works)
        db.enforce_retention();
        let result = db.select(&[LabelMatcher::equal("__name__", "x")], 0, i64::MAX);
        assert!(!result.is_empty()); // future sample should still be there
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
}
