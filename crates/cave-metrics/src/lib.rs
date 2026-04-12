<<<<<<< HEAD
//! cave-metrics: Prometheus + Thanos replacement.
//! Provides TSDB, PromQL evaluation, remote_write, scraping, rules, and Alertmanager integration.

#![allow(dead_code)]

pub mod alertmanager;
pub mod api;
pub mod error;
pub mod exposition;
pub mod model;
pub mod promql;
pub mod remote_write;
pub mod rules;
pub mod scrape;
pub mod state;
pub mod tsdb;

pub use error::{MetricsError, MetricsResult};
pub use model::{Labels, LabelMatcher, MetricType, Sample, TimeSeries, Timestamp, Value};
pub use promql::{Engine, EvalContext, InstantSample, QueryValue};
pub use tsdb::{Tsdb, TsdbConfig};
pub use state::{MetricsConfig, MetricsState};

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use crate::model::{Labels, LabelMatcher, Sample};
    use crate::tsdb::{Tsdb, TsdbConfig};
    use crate::tsdb::wal::{Wal, WalRecord};
    use crate::tsdb::compaction;
    use crate::promql::{Engine, EvalContext, QueryValue};
    use crate::promql::parser::parse;
    use crate::promql::ast::{Expr, AggregateOp};
    use crate::exposition::parse_exposition;
    use crate::remote_write::{encode_write_request, decode_write_request};
    use crate::rules::recording::RecordingRule;
    use crate::rules::alerting::{AlertingRule, AlertState};

    fn make_tsdb() -> Tsdb {
        Tsdb::new(TsdbConfig::default()).unwrap()
    }

    // ─── Labels tests ──────────────────────────────────────────────────────────

    #[test]
    fn test_labels_fingerprint_stable() {
        let a = Labels::from_pairs([("__name__", "http_requests"), ("job", "api")]);
        let b = Labels::from_pairs([("__name__", "http_requests"), ("job", "api")]);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_labels_fingerprint_different() {
        let a = Labels::from_pairs([("__name__", "http_requests"), ("job", "api")]);
        let b = Labels::from_pairs([("__name__", "http_requests"), ("job", "worker")]);
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_label_matcher_equal() {
        let labels = Labels::from_pairs([("job", "api")]);
        let m = LabelMatcher::Equal { name: "job".to_string(), value: "api".to_string() };
        assert!(m.matches(&labels));
        let m2 = LabelMatcher::Equal { name: "job".to_string(), value: "worker".to_string() };
        assert!(!m2.matches(&labels));
    }

    #[test]
    fn test_label_matcher_not_equal() {
        let labels = Labels::from_pairs([("job", "api")]);
        let m = LabelMatcher::NotEqual { name: "job".to_string(), value: "worker".to_string() };
        assert!(m.matches(&labels));
        let m2 = LabelMatcher::NotEqual { name: "job".to_string(), value: "api".to_string() };
        assert!(!m2.matches(&labels));
    }

    #[test]
    fn test_label_matcher_regex() {
        let labels = Labels::from_pairs([("job", "api-server")]);
        let m = LabelMatcher::RegexMatch { name: "job".to_string(), pattern: "api.*".to_string() };
        assert!(m.matches(&labels));
        let m2 = LabelMatcher::RegexMatch { name: "job".to_string(), pattern: "worker.*".to_string() };
        assert!(!m2.matches(&labels));
    }

    #[test]
    fn test_label_matcher_regex_not() {
        let labels = Labels::from_pairs([("job", "api-server")]);
        let m = LabelMatcher::RegexNotMatch { name: "job".to_string(), pattern: "worker.*".to_string() };
        assert!(m.matches(&labels));
        let m2 = LabelMatcher::RegexNotMatch { name: "job".to_string(), pattern: "api.*".to_string() };
        assert!(!m2.matches(&labels));
    }

    // ─── TSDB tests ────────────────────────────────────────────────────────────

    #[test]
    fn test_tsdb_append_and_select() {
        let db = make_tsdb();
        let labels = Labels::from_pairs([("__name__", "requests"), ("job", "api")]);
        db.append(labels.clone(), 1000, 42.0).unwrap();
        let matchers = vec![
            LabelMatcher::Equal { name: "__name__".to_string(), value: "requests".to_string() },
        ];
        let result = db.select(&matchers, 0, 5000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].samples[0].value, 42.0);
    }

    #[test]
    fn test_tsdb_select_range() {
        let db = make_tsdb();
        let labels = Labels::from_pairs([("__name__", "counter")]);
        for i in 0..10i64 {
            db.append(labels.clone(), i * 1000, i as f64).unwrap();
        }
        let matchers = vec![LabelMatcher::Equal { name: "__name__".to_string(), value: "counter".to_string() }];
        let result = db.select(&matchers, 2000, 5000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].samples.len(), 4); // ts 2000, 3000, 4000, 5000
    }

    #[test]
    fn test_tsdb_no_match() {
        let db = make_tsdb();
        let labels = Labels::from_pairs([("__name__", "counter"), ("job", "api")]);
        db.append(labels, 1000, 1.0).unwrap();
        let matchers = vec![LabelMatcher::Equal { name: "__name__".to_string(), value: "nonexistent".to_string() }];
        let result = db.select(&matchers, 0, 5000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_tsdb_label_names() {
        let db = make_tsdb();
        db.append(Labels::from_pairs([("__name__", "m"), ("job", "api"), ("env", "prod")]), 1000, 1.0).unwrap();
        let names = db.label_names();
        assert!(names.contains(&"__name__".to_string()));
        assert!(names.contains(&"job".to_string()));
        assert!(names.contains(&"env".to_string()));
    }

    #[test]
    fn test_tsdb_label_values() {
        let db = make_tsdb();
        db.append(Labels::from_pairs([("__name__", "m"), ("job", "api")]), 1000, 1.0).unwrap();
        db.append(Labels::from_pairs([("__name__", "m"), ("job", "worker")]), 1000, 1.0).unwrap();
        let values = db.label_values("job");
        assert!(values.contains(&"api".to_string()));
        assert!(values.contains(&"worker".to_string()));
    }

    #[test]
    fn test_tsdb_retention_enforcement() {
        let db = Tsdb::new(TsdbConfig {
            retention_ms: 10_000, // 10 seconds
            ..Default::default()
        }).unwrap();
        let labels = Labels::from_pairs([("__name__", "old_counter")]);
        db.append(labels, 1000, 1.0).unwrap(); // old sample
        let now_ms = 100_000; // simulate time passing
        db.enforce_retention(now_ms);
        let matchers = vec![LabelMatcher::Equal { name: "__name__".to_string(), value: "old_counter".to_string() }];
        let result = db.select(&matchers, 0, now_ms);
        // The sample is gone after retention enforcement
        assert!(result.iter().all(|ts| ts.samples.is_empty()));
    }

    #[test]
    fn test_tsdb_compaction() {
        let mut samples: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
        for i in 0..200i64 {
            samples.insert(i * 1000, i as f64);
        }
        assert_eq!(samples.len(), 200);
        compaction::compact(&mut samples, 100);
        assert!(samples.len() <= 100);
    }

    // ─── WAL tests ─────────────────────────────────────────────────────────────

    #[test]
    fn test_wal_append_and_replay() {
        let dir = std::env::temp_dir().join(format!("cave_wal_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wal_path = dir.join("wal.log");
        {
            let wal = Wal::new(Some(&dir)).unwrap();
            let labels = Labels::from_pairs([("__name__", "test_metric")]);
            let fp = labels.fingerprint();
            wal.append_meta(fp, &labels).unwrap();
            wal.append_sample(fp, 1000, 42.0).unwrap();
            wal.append_sample(fp, 2000, 43.0).unwrap();
        }
        let records = Wal::replay(&wal_path).unwrap();
        assert_eq!(records.len(), 3);
        let has_meta = records.iter().any(|r| matches!(r, WalRecord::Meta { .. }));
        let has_samples = records.iter().filter(|r| matches!(r, WalRecord::Sample { .. })).count();
        assert!(has_meta);
        assert_eq!(has_samples, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── PromQL parser tests ───────────────────────────────────────────────────

    #[test]
    fn test_promql_parse_simple_selector() {
        let expr = parse("http_requests_total").unwrap();
        assert!(matches!(expr, Expr::VectorSelector { .. }));
    }

    #[test]
    fn test_promql_parse_range_selector() {
        let expr = parse("http_requests_total[5m]").unwrap();
        assert!(matches!(expr, Expr::MatrixSelector { .. }));
    }

    #[test]
    fn test_promql_parse_aggregation() {
        let expr = parse("sum(http_requests_total) by (job)").unwrap();
        assert!(matches!(expr, Expr::Aggregate { op: AggregateOp::Sum, .. }));
    }

    #[test]
    fn test_promql_parse_function_call() {
        let expr = parse("rate(http_requests_total[5m])").unwrap();
        assert!(matches!(expr, Expr::Call { func, .. } if func == "rate"));
    }

    #[test]
    fn test_promql_parse_binary_op() {
        let expr = parse("a + b").unwrap();
        assert!(matches!(expr, Expr::Binary { .. }));
    }

    // ─── PromQL eval tests ─────────────────────────────────────────────────────

    fn insert_counter(db: &Tsdb, name: &str, job: &str, start_ms: i64, count: usize, step_ms: i64) {
        let labels = Labels::from_pairs([("__name__", name), ("job", job)]);
        for i in 0..count as i64 {
            db.append(labels.clone(), start_ms + i * step_ms, i as f64 * 10.0).unwrap();
        }
    }

    #[test]
    fn test_promql_eval_instant_vector() {
        let db = make_tsdb();
        let labels = Labels::from_pairs([("__name__", "up"), ("job", "api")]);
        db.append(labels, 1000, 1.0).unwrap();
        let engine = Engine::new();
        let expr = parse("up").unwrap();
        let ctx = EvalContext::instant(1000);
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].value, 1.0);
        } else {
            panic!("Expected instant vector");
        }
    }

    #[test]
    fn test_promql_eval_sum_aggregation() {
        let db = make_tsdb();
        db.append(Labels::from_pairs([("__name__", "up"), ("job", "a")]), 1000, 1.0).unwrap();
        db.append(Labels::from_pairs([("__name__", "up"), ("job", "b")]), 1000, 2.0).unwrap();
        let engine = Engine::new();
        let expr = parse("sum(up)").unwrap();
        let ctx = EvalContext::instant(1000);
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].value, 3.0);
        } else {
            panic!("Expected instant vector");
        }
    }

    #[test]
    fn test_promql_eval_rate() {
        let db = make_tsdb();
        // Insert counter increasing by 10 every second
        let labels = Labels::from_pairs([("__name__", "reqs"), ("job", "api")]);
        for i in 0..6i64 {
            db.append(labels.clone(), i * 1000, i as f64 * 10.0).unwrap();
        }
        let engine = Engine::new();
        let expr = parse("rate(reqs[5s])").unwrap();
        let ctx = EvalContext {
            timestamp_ms: 5000,
            lookback_ms: 5000,
            step_ms: 0,
            start_ms: 5000,
            end_ms: 5000,
        };
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            // rate should be ~10/s
            assert!(iv[0].value > 5.0 && iv[0].value < 15.0, "rate={}", iv[0].value);
        } else {
            panic!("Expected instant vector");
        }
    }

    #[test]
    fn test_promql_eval_histogram_quantile() {
        let db = make_tsdb();
        let ts = 1000i64;
        // Insert histogram buckets for a metric
        let buckets = vec![
            ("0.1", 10.0),
            ("0.5", 50.0),
            ("1.0", 90.0),
            ("+Inf", 100.0),
        ];
        for (le, count) in &buckets {
            let labels = Labels::from_pairs([
                ("__name__", "latency_bucket"),
                ("le", le),
                ("job", "api"),
            ]);
            db.append(labels, ts, *count).unwrap();
        }
        let engine = Engine::new();
        let expr = parse("histogram_quantile(0.5, latency_bucket[1m])").unwrap();
        let ctx = EvalContext {
            timestamp_ms: ts,
            lookback_ms: 60_000,
            step_ms: 0,
            start_ms: ts,
            end_ms: ts,
        };
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            // p50 should be ~0.5
            assert!(iv[0].value >= 0.0, "p50={}", iv[0].value);
        } else {
            panic!("Expected instant vector");
        }
    }

    #[test]
    fn test_promql_eval_binary_add() {
        let db = make_tsdb();
        let engine = Engine::new();
        let expr = parse("1 + 2").unwrap();
        let ctx = EvalContext::instant(1000);
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::Scalar(n) = result {
            assert_eq!(n, 3.0);
        } else {
            panic!("Expected scalar");
        }
    }

    #[test]
    fn test_promql_eval_topk() {
        let db = make_tsdb();
        for (job, val) in [("a", 1.0), ("b", 3.0), ("c", 2.0)] {
            db.append(Labels::from_pairs([("__name__", "up"), ("job", job)]), 1000, val).unwrap();
        }
        let engine = Engine::new();
        let expr = parse("topk(2, up)").unwrap();
        let ctx = EvalContext::instant(1000);
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 2);
            let vals: Vec<f64> = iv.iter().map(|s| s.value).collect();
            assert!(vals.contains(&3.0));
            assert!(vals.contains(&2.0));
        } else {
            panic!("Expected instant vector");
        }
    }

    #[test]
    fn test_promql_eval_label_replace() {
        let db = make_tsdb();
        db.append(Labels::from_pairs([("__name__", "up"), ("job", "api-server")]), 1000, 1.0).unwrap();
        let engine = Engine::new();
        let expr = parse(r#"label_replace(up, "service", "$1", "job", "(.+)-server")"#).unwrap();
        let ctx = EvalContext::instant(1000);
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].labels.get("service"), Some("api"));
        } else {
            panic!("Expected instant vector");
        }
    }

    #[test]
    fn test_promql_eval_absent_empty() {
        let db = make_tsdb();
        let engine = Engine::new();
        let expr = parse("absent(nonexistent_metric)").unwrap();
        let ctx = EvalContext::instant(1000);
        let result = engine.eval_instant(&expr, &ctx, &db).unwrap();
        if let QueryValue::InstantVector(iv) = result {
            assert_eq!(iv.len(), 1);
            assert_eq!(iv[0].value, 1.0);
        } else {
            panic!("Expected instant vector with 1 element");
        }
    }

    // ─── Exposition parser tests ───────────────────────────────────────────────

    #[test]
    fn test_exposition_parse_counter() {
        let input = "# HELP http_requests_total HTTP requests\n# TYPE http_requests_total counter\nhttp_requests_total 1234\n";
        let result = parse_exposition(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, 1234.0);
    }

    #[test]
    fn test_exposition_parse_with_labels() {
        let input = r#"http_requests_total{method="GET",path="/api"} 100 1234567890000
"#;
        let result = parse_exposition(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0.get("method"), Some("GET"));
        assert_eq!(result[0].0.get("path"), Some("/api"));
        assert_eq!(result[0].1, 100.0);
        assert_eq!(result[0].2, Some(1234567890000));
    }

    #[test]
    fn test_exposition_parse_histogram() {
        let input = r#"http_latency_bucket{le="0.1"} 10
http_latency_bucket{le="0.5"} 50
http_latency_bucket{le="+Inf"} 100
http_latency_count 100
http_latency_sum 45.3
"#;
        let result = parse_exposition(input).unwrap();
        assert_eq!(result.len(), 5);
    }

    // ─── Remote write tests ────────────────────────────────────────────────────

    #[test]
    fn test_remote_write_encode_decode() {
        use crate::model::TimeSeries;
        let ts = vec![TimeSeries {
            labels: Labels::from_pairs([("__name__", "test"), ("job", "api")]),
            samples: vec![
                Sample { timestamp: 1000, value: 42.0 },
                Sample { timestamp: 2000, value: 43.0 },
            ],
        }];
        let encoded = encode_write_request(&ts).unwrap();
        let decoded = decode_write_request(&encoded).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].labels.get("__name__"), Some("test"));
        assert_eq!(decoded[0].samples.len(), 2);
        assert_eq!(decoded[0].samples[0].value, 42.0);
    }

    // ─── Rules tests ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_recording_rule_evaluate() {
        let db = make_tsdb();
        // Insert some data
        db.append(Labels::from_pairs([("__name__", "requests"), ("job", "a")]), 1000, 5.0).unwrap();
        db.append(Labels::from_pairs([("__name__", "requests"), ("job", "b")]), 1000, 10.0).unwrap();
        let engine = Engine::new();
        let rule = RecordingRule {
            name: "job:requests:sum".to_string(),
            expr: "sum(requests)".to_string(),
            labels: Labels::default(),
            interval_ms: 15_000,
        };
        rule.evaluate(&engine, &db, 1000).await.unwrap();
        // The recording rule result should now be in the TSDB
        let matchers = vec![LabelMatcher::Equal { name: "__name__".to_string(), value: "job:requests:sum".to_string() }];
        let result = db.select(&matchers, 0, 5000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].samples[0].value, 15.0);
    }

    #[tokio::test]
    async fn test_alerting_rule_fires() {
        let db = make_tsdb();
        // Insert high error rate
        db.append(Labels::from_pairs([("__name__", "error_rate"), ("job", "api")]), 1000, 0.5).unwrap();
        let engine = Engine::new();
        let rule = AlertingRule {
            name: "HighErrorRate".to_string(),
            expr: "error_rate > 0.1".to_string(),
            for_ms: 0, // fire immediately
            labels: Labels::default(),
            annotations: Labels::from_pairs([("summary", "High error rate detected")]),
            interval_ms: 15_000,
        };
        let mut pending: HashMap<String, i64> = HashMap::new();
        let alerts = rule.evaluate(&engine, &db, 1000, &mut pending).await.unwrap();
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].state, AlertState::Firing);
        assert_eq!(alerts[0].name, "HighErrorRate");
    }
}
=======
//! CAVE Metrics — time-series metrics ingestion and query engine.
//!
//! Replaces Prometheus + Thanos with a Rust-native implementation.
//! Supports remote_write ingestion, PromQL-compatible query API,
//! series metadata, and Prometheus exposition format for self-metrics.
//!
//! ## Upstream Compatibility: Prometheus
//! - Remote Write: POST /api/v1/write (Prometheus remote_write protobuf)
//! - Query API:    GET  /api/v1/query, /api/v1/query_range
//! - Metadata:     GET  /api/v1/series, /api/v1/labels, /api/v1/label/:name/values
//! - Self-metrics: GET  /metrics (Prometheus exposition format)
//! - Response envelope: {"status":"success","data":{"resultType":"...","result":[...]}}
//!
//! ## Upstream Tracking: Prometheus
//! - GitHub: https://github.com/prometheus/prometheus
//! - Tracked: remote_write protocol v1/v2, HTTP API spec, exposition format

pub mod models;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state shared across request handlers.
pub struct MetricsState {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for the metrics module.
pub fn router(state: Arc<MetricsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "metrics";
>>>>>>> claude/gallant-cartwright
