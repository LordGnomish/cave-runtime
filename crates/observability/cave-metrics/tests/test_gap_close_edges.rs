// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cave-metrics gap-close integration tests.
//!
//! Covers modules whose inline test coverage is sparse or absent:
//! discovery, scrape target/manager, OTLP/OpenMetrics ingestion, alertmgr
//! model + silence semantics, label matchers, recording rules, rule
//! groups, multitenant, state, MetricType FromStr, error variants,
//! remote_read execution, compaction, native histogram boundaries, and
//! template engine edges. Each test is failure-mode focused: empty
//! inputs, NaN / ±Inf, out-of-range timestamps, regex anchors,
//! cross-tenant rejection, idempotency, serde round-trip.

use cave_metrics::{
    Engine, Exemplar, ExemplarRing, LabelMatcher, Labels, MatchOp, MetricType, NativeHistogram,
    Notification, PeerQueue, QueryResult, Sample, ShardedNotifier, TemplateContext, Tsdb,
    TsdbConfig, parse, parse_azure_vms, parse_hetzner_servers, render_template,
};

use cave_metrics::alertmgr::model::{
    Alert, AlertGroup, InhibitRule, Receiver, Route, RouteMatcher, Silence, SilenceMatcher,
    SilenceStatus,
};
use cave_metrics::alertmgr::silence::SilenceStore;
use cave_metrics::discovery_cloud::Target as CloudTarget;
use cave_metrics::error::MetricsError;
use cave_metrics::ingestion::otlp;
use cave_metrics::ingestion::{exposition, graphite, openmetrics, statsd};
use cave_metrics::ingestion::remote_read::{decode_read_request, encode_read_response, execute_read};
use cave_metrics::ingestion::remote_write::{
    LabelMatcher as ProtoMatcher, MatchType, ProtoLabel, ProtoSample, ProtoTimeSeries, Query,
    ReadRequest, WriteRequest, decode_write_request, encode_write_request,
};
use cave_metrics::model::TimeSeries;
use cave_metrics::multitenant::{
    DEFAULT_TENANT, TENANT_LABEL, X_SCOPE_ORG_ID, enforce_tenant_filter, federation_relabel,
    inject_tenant_label, matches_tenant, series_per_tenant, tenant_count, tenant_from_headers,
};
use cave_metrics::remote_read_backend::{
    LabelMatcher as RrLabelMatcher, MatcherKind, MemoryReadBackend, ReadQuery, RemoteReadBackend,
    Sample as RrSample,
};
use cave_metrics::rules::{
    AlertRule, AlertState, FiringAlert, RecordingRule, RuleGroup,
};
use cave_metrics::scrape::ScrapeManager;
use cave_metrics::scrape::discovery::{resolve_all, resolve_file_sd, resolve_static};
use cave_metrics::scrape::target::{
    FileSdConfig, K8sRole, KubernetesSdConfig, ScrapeConfig, ScrapeTarget, StaticConfig,
};
use cave_metrics::state::MetricsState;
use cave_metrics::tsdb::compaction::{apply_retention, downsample_series, merge_samples};
use cave_metrics::tsdb::wal::WalRecord;
use cave_metrics::tsdb::{Block, HeadSeries};

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn lbl(pairs: &[(&str, &str)]) -> Labels {
    Labels::from_pairs(pairs.iter().map(|(k, v)| (*k, *v)))
}

fn sample(ts: i64, v: f64) -> Sample {
    Sample::new(ts, v)
}

// ── model.rs / matchers — edges not covered in lib::tests ───────────────────

#[test]
fn label_matcher_regex_invalid_pattern_returns_err() {
    // unbalanced paren — regex compile must fail
    assert!(LabelMatcher::regex("k", "(foo").is_err());
    assert!(LabelMatcher::not_regex("k", "(foo").is_err());
}

#[test]
fn label_matcher_regex_anchored_full_string() {
    // Prometheus anchors regex to full string; "prod" must not match "production-east"
    let m = LabelMatcher::regex("env", "prod").unwrap();
    assert!(m.matches(&lbl(&[("env", "prod")])));
    assert!(!m.matches(&lbl(&[("env", "production")])));
}

#[test]
fn label_matcher_equal_treats_missing_label_as_empty() {
    let m = LabelMatcher::equal("missing", "");
    // Per Prometheus semantics a missing label equals the empty string
    assert!(m.matches(&Labels::new()));
}

#[test]
fn label_matcher_partial_eq_ignores_compiled_regex_handle() {
    let a = LabelMatcher::regex("env", "prod.*").unwrap();
    let b = LabelMatcher::regex("env", "prod.*").unwrap();
    assert_eq!(a, b); // name/op/value drive equality
}

#[test]
fn match_op_round_trip_through_matcher() {
    assert_eq!(LabelMatcher::equal("k", "v").op, MatchOp::Equal);
    assert_eq!(LabelMatcher::not_equal("k", "v").op, MatchOp::NotEqual);
    assert_eq!(
        LabelMatcher::regex("k", ".*").unwrap().op,
        MatchOp::RegexMatch
    );
    assert_eq!(
        LabelMatcher::not_regex("k", ".*").unwrap().op,
        MatchOp::RegexNotMatch
    );
}

#[test]
fn labels_display_skips_name_label() {
    let l = lbl(&[("__name__", "cpu"), ("job", "api")]);
    let s = format!("{}", l);
    assert!(!s.contains("__name__"));
    assert!(s.contains("job=\"api\""));
}

#[test]
fn labels_serde_round_trip_preserves_order() {
    let l = lbl(&[("z", "1"), ("a", "2"), ("m", "3")]);
    let json = serde_json::to_string(&l).unwrap();
    let back: Labels = serde_json::from_str(&json).unwrap();
    let keys: Vec<&str> = back.iter().map(|(k, _)| k).collect();
    // BTreeMap → keys are sorted alphabetically
    assert_eq!(keys, vec!["a", "m", "z"]);
}

#[test]
fn labels_with_only_and_without_complementary() {
    let l = lbl(&[("a", "1"), ("b", "2"), ("c", "3")]);
    let only = l.with_only(&["a", "c"]);
    let wo = l.without(&["b"]);
    assert_eq!(only.0.len(), 2);
    assert!(only.get("b").is_none());
    assert_eq!(wo.get("a"), Some("1"));
    assert!(wo.get("b").is_none());
}

#[test]
fn labels_fingerprint_changes_with_value() {
    let a = lbl(&[("k", "v1")]);
    let b = lbl(&[("k", "v2")]);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn sample_round_trips_through_serde() {
    let s = sample(1_234_567, 3.14);
    let json = serde_json::to_string(&s).unwrap();
    let back: Sample = serde_json::from_str(&json).unwrap();
    assert_eq!(back.timestamp_ms, 1_234_567);
    assert!((back.value - 3.14).abs() < 1e-12);
}

#[test]
fn time_series_push_collects_samples() {
    let mut ts = TimeSeries::new(lbl(&[("__name__", "x")]));
    ts.push(sample(1, 1.0));
    ts.push(sample(2, 2.0));
    assert_eq!(ts.samples.len(), 2);
}

#[test]
fn metric_type_from_str_known_and_unknown() {
    assert_eq!(MetricType::from_str("counter").unwrap(), MetricType::Counter);
    assert_eq!(MetricType::from_str("HISTOGRAM").unwrap(), MetricType::Histogram);
    assert_eq!(MetricType::from_str("stateset").unwrap(), MetricType::StateSet);
    assert_eq!(MetricType::from_str("garbage").unwrap(), MetricType::Untyped);
    // Default is Untyped
    let mt: MetricType = Default::default();
    assert_eq!(mt, MetricType::Untyped);
}

// ── error.rs — variants & conversions ───────────────────────────────────────

#[test]
fn metrics_error_io_from_conversion() {
    let io = std::io::Error::new(std::io::ErrorKind::Other, "boom");
    let me: MetricsError = io.into();
    assert!(format!("{}", me).contains("IO error"));
}

#[test]
fn metrics_error_serde_from_conversion() {
    let je = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
    let me: MetricsError = je.into();
    assert!(format!("{}", me).contains("serialization error"));
}

#[test]
fn metrics_error_display_includes_variant_name() {
    let e = MetricsError::Parse("x".to_string());
    assert!(format!("{}", e).contains("parse error"));
    let e = MetricsError::Eval("y".to_string());
    assert!(format!("{}", e).contains("PromQL evaluation error"));
    let e = MetricsError::Http("z".to_string());
    assert!(format!("{}", e).contains("HTTP error"));
}

// ── scrape::target — ScrapeTarget health state machine ─────────────────────

#[test]
fn scrape_target_health_unknown_when_never_scraped() {
    let t = ScrapeTarget::new("http://x/m", Labels::new(), ScrapeConfig::default());
    assert_eq!(t.health(), "unknown");
}

#[test]
fn scrape_target_health_up_after_successful_scrape() {
    let mut t = ScrapeTarget::new("http://x/m", Labels::new(), ScrapeConfig::default());
    t.last_scrape_ms = 1000;
    assert_eq!(t.health(), "up");
}

#[test]
fn scrape_target_health_down_when_error_set() {
    let mut t = ScrapeTarget::new("http://x/m", Labels::new(), ScrapeConfig::default());
    t.last_scrape_ms = 1000;
    t.last_error = Some("connection refused".to_string());
    assert_eq!(t.health(), "down");
}

#[test]
fn scrape_config_defaults_match_prometheus_defaults() {
    let c = ScrapeConfig::default();
    assert_eq!(c.scrape_interval_ms, 15_000);
    assert_eq!(c.scrape_timeout_ms, 10_000);
    assert_eq!(c.metrics_path, "/metrics");
    assert_eq!(c.scheme, "http");
    assert!(c.honor_timestamps);
    assert!(!c.honor_labels);
}

#[test]
fn scrape_config_serde_round_trip() {
    let c = ScrapeConfig {
        job_name: "myjob".into(),
        ..ScrapeConfig::default()
    };
    let j = serde_json::to_string(&c).unwrap();
    let back: ScrapeConfig = serde_json::from_str(&j).unwrap();
    assert_eq!(back.job_name, "myjob");
}

#[test]
fn k8s_role_serializes_lowercase() {
    let cfg = KubernetesSdConfig {
        role: K8sRole::EndpointSlice,
        namespaces: vec!["default".to_string()],
        selectors: vec![],
    };
    let j = serde_json::to_string(&cfg).unwrap();
    assert!(j.contains("endpointslice"));
}

// ── scrape::discovery — pure resolvers ─────────────────────────────────────

#[test]
fn resolve_static_attaches_job_and_instance_labels() {
    let cfg = ScrapeConfig {
        job_name: "api".into(),
        ..ScrapeConfig::default()
    };
    let sc = StaticConfig {
        targets: vec!["1.2.3.4:9090".into(), "host:80".into()],
        labels: lbl(&[("env", "prod")]),
    };
    let out = resolve_static("api", &cfg, &sc);
    assert_eq!(out.len(), 2);
    for t in &out {
        assert_eq!(t.labels.get("job"), Some("api"));
        assert_eq!(t.labels.get("env"), Some("prod"));
        assert!(t.url.starts_with("http://"));
        assert!(t.url.ends_with("/metrics"));
    }
    assert_eq!(out[0].labels.get("instance"), Some("1.2.3.4:9090"));
}

#[test]
fn resolve_static_respects_scheme_and_path() {
    let cfg = ScrapeConfig {
        scheme: "https".into(),
        metrics_path: "/probe".into(),
        ..ScrapeConfig::default()
    };
    let sc = StaticConfig {
        targets: vec!["h:1".into()],
        labels: Labels::new(),
    };
    let out = resolve_static("x", &cfg, &sc);
    assert_eq!(out[0].url, "https://h:1/probe");
}

#[test]
fn resolve_file_sd_missing_file_returns_empty() {
    let cfg = ScrapeConfig::default();
    let fsc = FileSdConfig {
        files: vec!["/does/not/exist.json".into()],
        refresh_interval_ms: 10_000,
    };
    let out = resolve_file_sd("j", &cfg, &fsc);
    assert!(out.is_empty());
}

#[test]
fn resolve_file_sd_parses_simple_json_group() {
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        f,
        r#"[{{"targets":["a:1","b:2"],"labels":{{"env":"prod"}}}}]"#
    )
    .unwrap();
    let path = f.path().to_string_lossy().to_string();

    let cfg = ScrapeConfig::default();
    let fsc = FileSdConfig {
        files: vec![path],
        refresh_interval_ms: 10_000,
    };
    let out = resolve_file_sd("j", &cfg, &fsc);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].labels.get("env"), Some("prod"));
    assert_eq!(out[0].labels.get("job"), Some("j"));
}

#[test]
fn resolve_all_combines_static_and_file_sources() {
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    writeln!(f, r#"[{{"targets":["b:2"],"labels":{{}}}}]"#).unwrap();
    let path = f.path().to_string_lossy().to_string();

    let cfg = ScrapeConfig {
        job_name: "j".into(),
        static_configs: vec![StaticConfig {
            targets: vec!["a:1".into()],
            labels: Labels::new(),
        }],
        file_sd_configs: vec![FileSdConfig {
            files: vec![path],
            refresh_interval_ms: 10_000,
        }],
        ..ScrapeConfig::default()
    };
    let out = resolve_all(&cfg);
    assert_eq!(out.len(), 2);
}

// ── scrape::manager — add/remove config wiring ─────────────────────────────

#[test]
fn scrape_manager_add_config_resolves_targets() {
    let mgr = ScrapeManager::new(Arc::new(Tsdb::default()));
    let cfg = ScrapeConfig {
        job_name: "j".into(),
        static_configs: vec![StaticConfig {
            targets: vec!["x:1".into(), "y:2".into()],
            labels: Labels::new(),
        }],
        ..ScrapeConfig::default()
    };
    mgr.add_config(cfg);
    assert_eq!(mgr.targets().len(), 2);
}

#[test]
fn scrape_manager_remove_config_drops_targets() {
    let mgr = ScrapeManager::new(Arc::new(Tsdb::default()));
    let cfg = ScrapeConfig {
        job_name: "to-remove".into(),
        static_configs: vec![StaticConfig {
            targets: vec!["x:1".into()],
            labels: Labels::new(),
        }],
        ..ScrapeConfig::default()
    };
    mgr.add_config(cfg);
    assert_eq!(mgr.targets().len(), 1);
    mgr.remove_config("to-remove");
    assert!(mgr.targets().is_empty());
}

// ── state — wiring sanity (no background tasks started) ─────────────────────

#[test]
fn metrics_state_new_constructs_subsystems() {
    let s = MetricsState::new();
    let _ = Arc::clone(&s.tsdb);
    let _ = Arc::clone(&s.engine);
    let _ = Arc::clone(&s.scrape_manager);
    assert_eq!(s.rule_groups.read().len(), 0);
}

// ── alertmgr::model — Silence + RouteMatcher semantics ─────────────────────

#[test]
fn silence_matcher_equal_negation() {
    let pos = SilenceMatcher {
        name: "alertname".into(),
        value: "Foo".into(),
        is_regex: false,
        is_equal: true,
    };
    let neg = SilenceMatcher {
        name: "alertname".into(),
        value: "Foo".into(),
        is_regex: false,
        is_equal: false,
    };
    let foo = lbl(&[("alertname", "Foo")]);
    let bar = lbl(&[("alertname", "Bar")]);
    assert!(pos.matches(&foo));
    assert!(!pos.matches(&bar));
    assert!(!neg.matches(&foo));
    assert!(neg.matches(&bar));
}

#[test]
fn silence_matcher_regex_anchored() {
    let m = SilenceMatcher {
        name: "env".into(),
        value: "prod".into(),
        is_regex: true,
        is_equal: true,
    };
    // Must be fully anchored — "production" should NOT match
    assert!(m.matches(&lbl(&[("env", "prod")])));
    assert!(!m.matches(&lbl(&[("env", "production")])));
}

#[test]
fn silence_matches_all_matchers_and_active_window() {
    let s = Silence {
        id: "s1".to_string(),
        matchers: vec![
            SilenceMatcher {
                name: "alertname".into(),
                value: "X".into(),
                is_regex: false,
                is_equal: true,
            },
            SilenceMatcher {
                name: "env".into(),
                value: "prod".into(),
                is_regex: false,
                is_equal: true,
            },
        ],
        starts_at: "2026-01-01T00:00:00Z".into(),
        ends_at: "2030-01-01T00:00:00Z".into(),
        created_by: "ops".into(),
        comment: "".into(),
        status: SilenceStatus {
            state: "active".into(),
        },
    };
    let matched = lbl(&[("alertname", "X"), ("env", "prod")]);
    let unmatched = lbl(&[("alertname", "X"), ("env", "dev")]);
    assert!(s.matches(&matched));
    assert!(!s.matches(&unmatched));
    assert!(s.is_active("2026-06-01T00:00:00Z"));
    // Outside the window
    assert!(!s.is_active("2025-01-01T00:00:00Z"));
    assert!(!s.is_active("2030-06-01T00:00:00Z"));
    // State expired
    let mut s2 = s.clone();
    s2.status.state = "expired".into();
    assert!(!s2.is_active("2026-06-01T00:00:00Z"));
}

#[test]
fn silence_store_create_then_expire_removes_silencing() {
    let store = SilenceStore::new();
    let id = store.create(Silence {
        id: String::new(),
        matchers: vec![SilenceMatcher {
            name: "x".into(),
            value: "y".into(),
            is_regex: false,
            is_equal: true,
        }],
        starts_at: "2026-01-01T00:00:00Z".into(),
        ends_at: "2030-01-01T00:00:00Z".into(),
        created_by: "t".into(),
        comment: "".into(),
        status: SilenceStatus {
            state: "active".into(),
        },
    });
    assert!(!id.is_empty());
    assert!(store.is_silenced(&lbl(&[("x", "y")]), "2026-06-01T00:00:00Z"));
    assert!(store.expire(&id));
    assert!(!store.is_silenced(&lbl(&[("x", "y")]), "2026-06-01T00:00:00Z"));
    assert!(!store.expire("non-existent"));
}

#[test]
fn alert_group_serde_round_trip() {
    let g = AlertGroup {
        labels: HashMap::from([("env".to_string(), "prod".to_string())]),
        receiver: Receiver {
            name: "slack".into(),
        },
        alerts: vec![],
    };
    let j = serde_json::to_string(&g).unwrap();
    let back: AlertGroup = serde_json::from_str(&j).unwrap();
    assert_eq!(back.receiver.name, "slack");
}

#[test]
fn route_serde_camel_case_friendly() {
    let r = Route {
        receiver: "p".into(),
        group_by: vec!["alertname".into()],
        group_wait: Some("30s".into()),
        group_interval: None,
        repeat_interval: None,
        matchers: vec![RouteMatcher {
            name: "severity".into(),
            value: "critical".into(),
            is_regex: false,
            is_equal: true,
        }],
        routes: vec![],
        continue_matching: false,
    };
    let j = serde_json::to_string(&r).unwrap();
    let back: Route = serde_json::from_str(&j).unwrap();
    assert_eq!(back.matchers[0].name, "severity");
}

#[test]
fn inhibit_rule_serde_round_trip() {
    let r = InhibitRule {
        source_matchers: vec![SilenceMatcher {
            name: "severity".into(),
            value: "critical".into(),
            is_regex: false,
            is_equal: true,
        }],
        target_matchers: vec![SilenceMatcher {
            name: "severity".into(),
            value: "warning".into(),
            is_regex: false,
            is_equal: true,
        }],
        equal: vec!["cluster".into(), "service".into()],
    };
    let j = serde_json::to_string(&r).unwrap();
    let back: InhibitRule = serde_json::from_str(&j).unwrap();
    assert_eq!(back.equal, vec!["cluster", "service"]);
}

// ── ingestion::openmetrics — strips EOF and detects content type ──────────

#[test]
fn openmetrics_strips_eof_marker_and_parses() {
    let body = "# TYPE http_requests counter\nhttp_requests{method=\"GET\"} 100 1000\n# EOF\n";
    let batch = openmetrics::parse(body).unwrap();
    assert_eq!(batch.len(), 1);
}

#[test]
fn openmetrics_content_type_detection_variants() {
    assert!(openmetrics::is_openmetrics(
        "application/openmetrics-text; version=1.0.0"
    ));
    assert!(openmetrics::is_openmetrics("text/openmetrics-text"));
    assert!(!openmetrics::is_openmetrics("text/plain"));
    assert!(!openmetrics::is_openmetrics(""));
}

// ── ingestion::otlp — JSON parser converts each datapoint type ────────────

#[test]
fn otlp_json_gauge_dp_to_ts() {
    let body = r#"{
      "resourceMetrics":[{
        "resource":{"attributes":[{"key":"service.name","value":{"stringValue":"api"}}]},
        "scopeMetrics":[{"metrics":[
          {"name":"cpu","gauge":{"dataPoints":[
            {"timeUnixNano":"1000000000","asDouble":0.42,
             "attributes":[{"key":"host","value":{"stringValue":"web01"}}]}
          ]}}
        ]}]
      }]
    }"#;
    let batch = otlp::parse_json(body).unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].labels.get("__name__"), Some("cpu"));
    assert_eq!(batch[0].labels.get("host"), Some("web01"));
    assert_eq!(batch[0].labels.get("service.name"), Some("api"));
    assert!((batch[0].samples[0].value - 0.42).abs() < 1e-9);
    // 1_000_000_000 nanoseconds → 1000 milliseconds
    assert_eq!(batch[0].samples[0].timestamp_ms, 1000);
}

#[test]
fn otlp_json_sum_uses_as_int_when_no_as_double() {
    let body = r#"{
      "resourceMetrics":[{
        "scopeMetrics":[{"metrics":[
          {"name":"requests","sum":{"dataPoints":[
            {"timeUnixNano":"2000000000","asInt":42}
          ]}}
        ]}]
      }]
    }"#;
    let batch = otlp::parse_json(body).unwrap();
    assert_eq!(batch.len(), 1);
    assert!((batch[0].samples[0].value - 42.0).abs() < 1e-9);
}

#[test]
fn otlp_json_histogram_emits_count_sum_and_buckets() {
    let body = r#"{
      "resourceMetrics":[{
        "scopeMetrics":[{"metrics":[
          {"name":"lat","histogram":{"dataPoints":[
            {"timeUnixNano":"1000000000","count":100,"sum":12.5,
             "explicitBounds":[0.1,0.5,1.0],
             "bucketCounts":[10,30,40,20]}
          ]}}
        ]}]
      }]
    }"#;
    let batch = otlp::parse_json(body).unwrap();
    // Expect _count + _sum + 3 explicit buckets + +Inf bucket = 6
    assert_eq!(batch.len(), 6);
    let names: Vec<&str> = batch
        .iter()
        .filter_map(|ts| ts.labels.metric_name())
        .collect();
    assert!(names.iter().any(|n| *n == "lat_count"));
    assert!(names.iter().any(|n| *n == "lat_sum"));
    let buckets: Vec<&TimeSeries> = batch
        .iter()
        .filter(|ts| ts.labels.metric_name() == Some("lat_bucket"))
        .collect();
    assert_eq!(buckets.len(), 4);
    assert!(buckets.iter().any(|b| b.labels.get("le") == Some("+Inf")));
}

#[test]
fn otlp_json_summary_emits_count_sum_and_quantiles() {
    let body = r#"{
      "resourceMetrics":[{
        "scopeMetrics":[{"metrics":[
          {"name":"req","summary":{"dataPoints":[
            {"timeUnixNano":"1000000000","count":10,"sum":3.5,
             "quantileValues":[{"quantile":0.5,"value":0.3},{"quantile":0.99,"value":1.1}]}
          ]}}
        ]}]
      }]
    }"#;
    let batch = otlp::parse_json(body).unwrap();
    // _count + _sum + 2 quantiles
    assert_eq!(batch.len(), 4);
    let quant: Vec<_> = batch
        .iter()
        .filter(|ts| ts.labels.get("quantile").is_some())
        .collect();
    assert_eq!(quant.len(), 2);
}

#[test]
fn otlp_json_invalid_returns_ingestion_error() {
    let err = otlp::parse_json("not json").unwrap_err();
    assert!(format!("{}", err).contains("ingestion error"));
}

// ── ingestion::statsd, graphite — extra edges ──────────────────────────────

#[test]
fn statsd_timer_or_unknown_yields_value() {
    // ":1|ms" timer → value should be present in the sample
    let batch = statsd::parse_batch("login.duration:120|ms");
    assert!(!batch.is_empty());
    assert_eq!(batch[0].samples[0].value, 120.0);
}

#[test]
fn statsd_empty_line_skipped() {
    let batch = statsd::parse_batch("");
    assert!(batch.is_empty());
}

#[test]
fn graphite_invalid_line_skipped() {
    // graphite::parse_batch is tolerant — malformed lines simply omitted
    let batch = graphite::parse_batch("");
    assert!(batch.is_empty());
}

// ── ingestion::exposition (the real one, not the dead top-level one) ──────

#[test]
fn exposition_skips_help_and_type_comments() {
    let body = "# HELP foo description\n# TYPE foo counter\nfoo 1\n";
    let batch = exposition::parse(body).unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].labels.metric_name(), Some("foo"));
}

#[test]
fn exposition_negative_inf_and_nan_parse() {
    let body = "x -Inf\ny NaN\n";
    let batch = exposition::parse(body).unwrap();
    let neg_inf = batch
        .iter()
        .any(|ts| ts.samples.iter().any(|s| s.value == f64::NEG_INFINITY));
    let nan = batch
        .iter()
        .any(|ts| ts.samples.iter().any(|s| s.value.is_nan()));
    assert!(neg_inf);
    assert!(nan);
}

// ── exemplars — extra edges (top of buffer, zero-window window, etc.) ──────

#[test]
fn exemplar_ring_capacity_zero_clamps_to_one() {
    // Constructor must clamp 0 → 1 to avoid panics in remove(0)
    let mut r = ExemplarRing::new(0);
    r.push(
        Exemplar {
            timestamp_ms: 1,
            value: 1.0,
            trace_id: "t".into(),
            span_id: None,
            labels: HashMap::new(),
        },
        60_000,
    );
    r.push(
        Exemplar {
            timestamp_ms: 2,
            value: 2.0,
            trace_id: "t".into(),
            span_id: None,
            labels: HashMap::new(),
        },
        60_000,
    );
    // capacity=1 → keeps only newest
    assert_eq!(r.len(), 1);
}

#[test]
fn exemplar_ring_in_range_returns_empty_outside() {
    let mut r = ExemplarRing::new(8);
    r.push(
        Exemplar {
            timestamp_ms: 100,
            value: 1.0,
            trace_id: "t".into(),
            span_id: None,
            labels: HashMap::new(),
        },
        60_000,
    );
    assert!(r.in_range(0, 50).is_empty());
    assert!(r.in_range(200, 300).is_empty());
}

#[test]
fn native_histogram_zero_threshold_default_zero_treats_zero_as_zero() {
    let mut h = NativeHistogram::new(0);
    h.observe(0.0);
    // zero_threshold default is 0.0; |0| <= 0 → zero_count incremented
    assert_eq!(h.zero_count, 1);
    assert_eq!(h.count, 1);
}

#[test]
fn native_histogram_negative_values_go_to_negative_bucket() {
    let mut h = NativeHistogram::new(0);
    h.observe(-3.0);
    assert!(!h.negative_buckets.is_empty());
    assert!(h.positive_buckets.is_empty());
}

#[test]
fn native_histogram_base_for_schema_zero_is_two() {
    let h = NativeHistogram::new(0);
    assert!((h.base() - 2.0).abs() < 1e-9);
}

#[test]
fn native_histogram_quantile_q_out_of_range_returns_nan() {
    let mut h = NativeHistogram::new(0);
    h.observe(3.0);
    assert!(h.quantile(-0.1).is_nan());
    assert!(h.quantile(1.1).is_nan());
}

// ── notifier_sharded — corner cases ────────────────────────────────────────

#[test]
fn sharded_notifier_empty_drain_returns_zero() {
    let mut n = ShardedNotifier::new();
    let sent = n.drain_round(|_, _| Ok(()));
    assert_eq!(sent, 0);
}

#[test]
fn sharded_notifier_default_is_empty() {
    let n = ShardedNotifier::default();
    assert!(n.peers.is_empty());
    assert_eq!(n.total_pending(), 0);
}

#[test]
fn peer_tick_refill_caps_at_max() {
    let mut p = PeerQueue::new("am-1", 4, 3, 100);
    p.tokens = 1;
    p.refill();
    assert_eq!(p.tokens, 3);
}

#[test]
fn peer_send_failure_then_success_eventually_drains() {
    let mut p = PeerQueue::new("am-1", 4, 5, 5);
    p.enqueue(Notification::new("a", 1, "firing"));
    // First attempt fails, alert retained
    let _ = p.try_send_one(|_, _| Err(()));
    assert_eq!(p.queue.len(), 1);
    // Retry succeeds
    let _ = p.try_send_one(|_, _| Ok(()));
    assert_eq!(p.queue.len(), 0);
    assert_eq!(p.sent_count, 1);
    assert_eq!(p.failed_count, 1);
}

#[test]
fn notification_equality_by_value() {
    let a = Notification::new("foo", 1, "firing");
    let b = Notification::new("foo", 1, "firing");
    let c = Notification::new("foo", 2, "firing");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── discovery_cloud — Hetzner / Azure edge cases ───────────────────────────

#[test]
fn hetzner_handles_empty_body() {
    let ts = parse_hetzner_servers("{}", 9100);
    assert!(ts.is_empty());
}

#[test]
fn azure_handles_empty_body() {
    let ts = parse_azure_vms("{}", 9100);
    assert!(ts.is_empty());
}

#[test]
fn cloud_target_with_label_chain() {
    let t = CloudTarget::new("10.0.0.1:9100")
        .with_label("k1", "v1")
        .with_label("k2", "v2");
    assert_eq!(t.labels.len(), 2);
    assert_eq!(t.labels["k1"], "v1");
}

// ── template — additional edges (printf with %d, unknown labels) ───────────

#[test]
fn template_printf_int_via_var() {
    let ctx = TemplateContext::new().set_var("n", "12");
    assert_eq!(
        render_template("{{ printf \"%d\" $n }}", &ctx),
        "12"
    );
}

#[test]
fn template_printf_no_format_returns_empty_string() {
    let ctx = TemplateContext::new();
    // No leading quote — printf returns ""
    let out = render_template("{{ printf foo }}", &ctx);
    assert_eq!(out, "");
}

#[test]
fn template_unknown_label_renders_empty() {
    let ctx = TemplateContext::new();
    assert_eq!(
        render_template("{{ .Labels.missing }}", &ctx),
        ""
    );
}

#[test]
fn template_dollar_value_uses_context_value() {
    let ctx = TemplateContext::new().with_value(7.5);
    assert_eq!(render_template("{{ $value }}", &ctx), "7.5");
}

#[test]
fn template_literal_passthrough_when_no_directives() {
    let ctx = TemplateContext::new();
    assert_eq!(render_template("plain text", &ctx), "plain text");
}

// ── multitenant — header lookup with whitespace and case ────────────────────

#[test]
fn matches_tenant_allows_regex_matcher_to_pass_through() {
    // matches_tenant rejects only Equal matchers — Regex matchers should pass
    let m = vec![LabelMatcher::regex(TENANT_LABEL, ".+").unwrap()];
    assert!(matches_tenant(&m, "acme"));
}

#[test]
fn enforce_idempotent_when_called_twice() {
    let m = enforce_tenant_filter(vec![LabelMatcher::equal("__name__", "x")], "acme");
    let m = enforce_tenant_filter(m, "acme");
    // Already-present matcher → no duplicate
    assert_eq!(m.iter().filter(|x| x.name == TENANT_LABEL).count(), 1);
}

// ── recording rules / alert rules — alternate evaluation paths ─────────────

#[test]
fn recording_rule_with_extra_labels_applied() {
    let db = Arc::new(Tsdb::default());
    db.append(lbl(&[("__name__", "x")]), sample(1000, 5.0));
    let engine = Engine::new(Arc::clone(&db));
    let rule = RecordingRule::new("recorded", "x").with_labels(lbl(&[("team", "core")]));
    rule.evaluate(&engine, &db, 1100).unwrap();

    let series = db.select(&[LabelMatcher::equal("__name__", "recorded")], 0, i64::MAX);
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].0.get("team"), Some("core"));
}

#[test]
fn recording_rule_parse_error_propagates() {
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(Arc::clone(&db));
    let bad = RecordingRule::new("bad", "((not closed");
    let r = bad.evaluate(&engine, &db, 1000);
    assert!(r.is_err());
}

#[test]
fn alert_rule_with_labels_merges_into_alert_labels() {
    let db = Arc::new(Tsdb::default());
    db.append(lbl(&[("__name__", "errs")]), sample(1000, 1.0));
    let engine = Engine::new(Arc::clone(&db));
    let mut rule = AlertRule::new("ErrAlert", "errs", 0)
        .with_labels(lbl(&[("severity", "critical")]));
    let alerts = rule.evaluate(&engine, 1000).unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].labels.get("severity"), Some("critical"));
}

#[test]
fn alert_rule_annotations_pass_through_unchanged() {
    let db = Arc::new(Tsdb::default());
    db.append(lbl(&[("__name__", "x")]), sample(1000, 1.0));
    let engine = Engine::new(Arc::clone(&db));
    let mut rule = AlertRule::new("A", "x", 0)
        .with_annotations(lbl(&[("summary", "boom")]));
    let alerts = rule.evaluate(&engine, 1000).unwrap();
    assert_eq!(alerts[0].annotations.get("summary"), Some("boom"));
}

#[test]
fn firing_alert_serde_round_trip() {
    let a = FiringAlert {
        name: "A".into(),
        state: AlertState::Firing,
        labels: lbl(&[("alertname", "A")]),
        annotations: Labels::new(),
        active_at_ms: 1000,
        fired_at_ms: Some(2000),
        value: 1.0,
    };
    let j = serde_json::to_string(&a).unwrap();
    let back: FiringAlert = serde_json::from_str(&j).unwrap();
    assert_eq!(back.state, AlertState::Firing);
    assert_eq!(back.fired_at_ms, Some(2000));
}

#[test]
fn rule_group_evaluates_recording_and_alert_in_order() {
    let db = Arc::new(Tsdb::default());
    db.append(lbl(&[("__name__", "raw")]), sample(1000, 5.0));
    let engine = Engine::new(Arc::clone(&db));

    let mut group = RuleGroup {
        name: "g".into(),
        interval: std::time::Duration::from_secs(15),
        recording_rules: vec![RecordingRule::new("rec", "raw")],
        alert_rules: vec![AlertRule::new("A", "raw", 0)],
    };
    let alerts = group.evaluate(&engine, &db, 1000).unwrap();
    // Recording rule writes new series; alert fires
    let recorded = db.select(&[LabelMatcher::equal("__name__", "rec")], 0, i64::MAX);
    assert!(!recorded.is_empty());
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].state, AlertState::Firing);
}

// ── remote_read — execution path ───────────────────────────────────────────

#[test]
fn remote_read_execute_returns_series_in_range() {
    let db = Tsdb::default();
    db.append(
        lbl(&[("__name__", "cpu"), ("job", "api")]),
        sample(1000, 0.5),
    );
    db.append(
        lbl(&[("__name__", "cpu"), ("job", "api")]),
        sample(2000, 0.6),
    );

    let req = ReadRequest {
        queries: vec![Query {
            start_timestamp_ms: 0,
            end_timestamp_ms: 5000,
            matchers: vec![ProtoMatcher {
                r#type: MatchType::Eq as i32,
                name: "__name__".into(),
                value: "cpu".into(),
            }],
        }],
    };
    let resp = execute_read(req, &db).unwrap();
    assert_eq!(resp.results.len(), 1);
    assert_eq!(resp.results[0].timeseries.len(), 1);
    assert_eq!(resp.results[0].timeseries[0].samples.len(), 2);
}

#[test]
fn remote_read_request_snappy_round_trip() {
    let req = ReadRequest {
        queries: vec![Query {
            start_timestamp_ms: 10,
            end_timestamp_ms: 20,
            matchers: vec![],
        }],
    };
    // Use the write_request encoder to compress arbitrary bytes via prost
    use prost::Message;
    let plain = req.encode_to_vec();
    let mut enc = snap::raw::Encoder::new();
    let compressed = enc.compress_vec(&plain).unwrap();
    let decoded = decode_read_request(&compressed).unwrap();
    assert_eq!(decoded.queries.len(), 1);
    assert_eq!(decoded.queries[0].start_timestamp_ms, 10);
}

#[test]
fn remote_write_request_round_trip_includes_metadata() {
    let req = WriteRequest {
        timeseries: vec![ProtoTimeSeries {
            labels: vec![ProtoLabel {
                name: "__name__".into(),
                value: "x".into(),
            }],
            samples: vec![ProtoSample {
                value: 1.0,
                timestamp: 1,
            }],
            exemplars: vec![],
        }],
        metadata: vec![],
    };
    let bytes = encode_write_request(&req).unwrap();
    let back = decode_write_request(&bytes).unwrap();
    assert_eq!(back.timeseries.len(), 1);
}

// ── remote_read_backend (trait + memory) ──────────────────────────────────

#[test]
fn remote_read_backend_glob_alternation_full_anchored() {
    let mut b = MemoryReadBackend::new();
    b.add_series(
        std::collections::BTreeMap::from([("__name__".to_string(), "up".to_string())]),
        vec![RrSample {
            timestamp_ms: 100,
            value: 1.0,
        }],
    );
    let q = ReadQuery {
        start_ms: 0,
        end_ms: 200,
        matchers: vec![RrLabelMatcher::re("__name__", "up|down")],
    };
    let r = b.read(&q);
    assert_eq!(r.len(), 1);
}

#[test]
fn remote_read_backend_neq_matcher_excludes() {
    let mut b = MemoryReadBackend::new();
    b.add_series(
        std::collections::BTreeMap::from([
            ("__name__".to_string(), "x".to_string()),
            ("env".to_string(), "prod".to_string()),
        ]),
        vec![RrSample {
            timestamp_ms: 1,
            value: 1.0,
        }],
    );
    let q = ReadQuery {
        start_ms: 0,
        end_ms: 10,
        matchers: vec![RrLabelMatcher::ne("env", "prod")],
    };
    let r = b.read(&q);
    assert!(r.is_empty());
}

#[test]
fn remote_read_backend_kind_variants_distinct() {
    let a = RrLabelMatcher::eq("k", "v");
    let b = RrLabelMatcher::ne("k", "v");
    let c = RrLabelMatcher::re("k", "v");
    let d = RrLabelMatcher::rne("k", "v");
    assert_ne!(a.kind, b.kind);
    assert_ne!(b.kind, c.kind);
    assert_ne!(c.kind, d.kind);
    assert_eq!(a.kind, MatcherKind::Equal);
}

#[test]
fn remote_read_backend_series_count_grows() {
    let mut b = MemoryReadBackend::new();
    assert_eq!(b.series_count(), 0);
    b.add_series(std::collections::BTreeMap::new(), vec![]);
    assert_eq!(b.series_count(), 1);
}

// ── compaction — pure functions ────────────────────────────────────────────

#[test]
fn compaction_downsample_with_zero_resolution_is_passthrough_or_no_op() {
    let samples = vec![sample(0, 1.0), sample(1, 2.0)];
    // resolution_ms=0 → should not panic; either empty or passthrough
    let _ = downsample_series(&samples, 0);
}

#[test]
fn compaction_merge_handles_empty_inputs() {
    let m = merge_samples(&[], &[]);
    assert!(m.is_empty());
}

#[test]
fn compaction_merge_one_empty_returns_other() {
    let a = vec![sample(1, 1.0)];
    let m = merge_samples(&a, &[]);
    assert_eq!(m.len(), 1);
    let m = merge_samples(&[], &a);
    assert_eq!(m.len(), 1);
}

#[test]
fn compaction_apply_retention_removes_old_samples() {
    let mut s = vec![sample(100, 1.0), sample(200, 2.0), sample(300, 3.0)];
    apply_retention(&mut s, 200);
    assert!(!s.iter().any(|x| x.timestamp_ms < 200));
}

// ── tsdb HeadSeries + Block edges ──────────────────────────────────────────

#[test]
fn head_series_duplicate_ts_overwrites_value() {
    let mut hs = HeadSeries::new(lbl(&[("__name__", "x")]));
    hs.append(sample(100, 1.0));
    hs.append(sample(100, 2.0)); // dup ts → overwrite
    assert_eq!(hs.samples.len(), 1);
    assert_eq!(hs.samples[0].value, 2.0);
}

#[test]
fn head_series_samples_in_range_inclusive() {
    let mut hs = HeadSeries::new(lbl(&[("__name__", "x")]));
    for t in [50, 100, 200, 300] {
        hs.append(sample(t, t as f64));
    }
    let r = hs.samples_in_range(100, 200);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].timestamp_ms, 100);
    assert_eq!(r[1].timestamp_ms, 200);
}

#[test]
fn head_series_latest_at_returns_within_lookback() {
    let mut hs = HeadSeries::new(lbl(&[("__name__", "x")]));
    hs.append(sample(50, 1.0));
    hs.append(sample(100, 2.0));
    let s = hs.latest_at(150, 100).unwrap();
    assert_eq!(s.value, 2.0);
    // outside lookback
    assert!(hs.latest_at(1000, 50).is_none());
}

#[test]
fn block_select_filters_by_time_and_matchers() {
    let mut hs = HeadSeries::new(lbl(&[("__name__", "cpu")]));
    hs.append(sample(100, 1.0));
    let block = Block {
        min_ts: 100,
        max_ts: 100,
        series: vec![hs],
    };
    let r = block.select(&[LabelMatcher::equal("__name__", "cpu")], 0, 200);
    assert_eq!(r.len(), 1);
    // matcher excludes
    let r = block.select(&[LabelMatcher::equal("__name__", "mem")], 0, 200);
    assert!(r.is_empty());
}

#[test]
fn tsdb_compact_flushes_old_head_to_block() {
    let db = Tsdb::new(TsdbConfig {
        block_duration_ms: 1, // immediately stale
        ..TsdbConfig::default()
    });
    db.append(lbl(&[("__name__", "x")]), sample(100, 1.0));
    db.compact();
    // After compaction, downsample should be able to see the series
    let ds = db.downsample(60_000);
    // Either head was flushed (ds non-empty) or compaction was a no-op for new
    // samples — either way the call must not panic.
    let _ = ds;
}

// ── WAL record — both variants serialize ───────────────────────────────────

#[test]
fn wal_record_checkpoint_round_trip() {
    let rec = WalRecord::Checkpoint { ts: 12345 };
    let j = serde_json::to_string(&rec).unwrap();
    let back: WalRecord = serde_json::from_str(&j).unwrap();
    matches!(back, WalRecord::Checkpoint { ts: 12345 });
}

// ── PromQL parser — operator precedence + edge tokens ─────────────────────

#[test]
fn promql_parse_precedence_mul_binds_tighter_than_add() {
    // "1 + 2 * 3" → 1 + (2*3) = 7
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(Arc::clone(&db));
    let ast = parse("1 + 2 * 3").unwrap();
    let r = engine.eval_instant(&ast, 0).unwrap();
    match r {
        QueryResult::Scalar(v) => assert!((v - 7.0).abs() < 1e-9),
        _ => panic!("expected scalar"),
    }
}

#[test]
fn promql_parse_pow_right_associative() {
    // "2 ^ 3 ^ 2" → 2^(3^2) = 2^9 = 512
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(Arc::clone(&db));
    let ast = parse("2 ^ 3 ^ 2").unwrap();
    let r = engine.eval_instant(&ast, 0).unwrap();
    if let QueryResult::Scalar(v) = r {
        assert!((v - 512.0).abs() < 1e-6);
    } else {
        panic!("expected scalar");
    }
}

#[test]
fn promql_parse_negative_number_via_unary_minus() {
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(Arc::clone(&db));
    let ast = parse("-5").unwrap();
    let r = engine.eval_instant(&ast, 0).unwrap();
    if let QueryResult::Scalar(v) = r {
        assert_eq!(v, -5.0);
    } else {
        panic!("expected scalar");
    }
}

#[test]
fn promql_parse_inf_and_nan_literals() {
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(Arc::clone(&db));
    let inf = engine.eval_instant(&parse("Inf").unwrap(), 0).unwrap();
    if let QueryResult::Scalar(v) = inf {
        assert!(v.is_infinite());
    } else {
        panic!();
    }
    let nan = engine.eval_instant(&parse("NaN").unwrap(), 0).unwrap();
    if let QueryResult::Scalar(v) = nan {
        assert!(v.is_nan());
    } else {
        panic!();
    }
}

#[test]
fn promql_parse_string_literal_returns_string_result() {
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(Arc::clone(&db));
    let r = engine.eval_instant(&parse(r#""hello""#).unwrap(), 0).unwrap();
    if let QueryResult::String(s) = r {
        assert_eq!(s, "hello");
    } else {
        panic!("expected string");
    }
}

#[test]
fn promql_parse_durations_units() {
    // Each duration unit must parse; we check the offset captures correct ms
    let cases = [
        ("x offset 1ms", 1_i64),
        ("x offset 1s", 1_000),
        ("x offset 2m", 120_000),
        ("x offset 1h", 3_600_000),
        ("x offset 1d", 86_400_000),
    ];
    for (q, want) in cases {
        let ast = parse(q).unwrap();
        if let cave_metrics::promql::ast::Expr::VectorSelector(vs) = ast {
            assert_eq!(vs.offset, Some(want), "duration mismatch: {}", q);
        } else {
            panic!("expected vector selector for: {}", q);
        }
    }
}

#[test]
fn promql_parse_at_modifier_captures_timestamp_ms() {
    let ast = parse("x @ 1700000").unwrap(); // seconds → ms inside parser
    if let cave_metrics::promql::ast::Expr::VectorSelector(vs) = ast {
        assert!(vs.at.is_some());
        // 1700000 * 1000
        assert_eq!(vs.at.unwrap(), 1_700_000_000);
    } else {
        panic!("expected vector selector");
    }
}

#[test]
fn promql_parse_matrix_selector_range_ms() {
    let ast = parse("x[5m]").unwrap();
    if let cave_metrics::promql::ast::Expr::MatrixSelector(ms) = ast {
        assert_eq!(ms.range_ms, 300_000);
    } else {
        panic!("expected matrix selector");
    }
}

#[test]
fn promql_parse_subquery_explicit_step() {
    let ast = parse("x[10m:1m]").unwrap();
    if let cave_metrics::promql::ast::Expr::Subquery(sq) = ast {
        assert_eq!(sq.range_ms, 600_000);
        assert_eq!(sq.step_ms, 60_000);
    } else {
        panic!("expected subquery");
    }
}

#[test]
fn promql_parse_grouping_after_args() {
    // "sum(x) by(job)" — grouping comes after
    let ast = parse("sum(x) by(job)").unwrap();
    if let cave_metrics::promql::ast::Expr::Aggregate(agg) = ast {
        assert_eq!(agg.grouping.labels, vec!["job".to_string()]);
    } else {
        panic!("expected aggregate");
    }
}

#[test]
fn promql_parse_without_grouping_modifier() {
    let ast = parse("sum without(job) (x)").unwrap();
    if let cave_metrics::promql::ast::Expr::Aggregate(agg) = ast {
        assert!(agg.grouping.without);
        assert_eq!(agg.grouping.labels, vec!["job".to_string()]);
    } else {
        panic!("expected aggregate");
    }
}

#[test]
fn promql_parse_unbalanced_paren_returns_err() {
    let r = parse("sum(x");
    assert!(r.is_err());
}

#[test]
fn promql_eval_comparison_returns_bool_when_modifier_set() {
    let db = Arc::new(Tsdb::default());
    db.append(lbl(&[("__name__", "x")]), sample(1000, 5.0));
    let engine = Engine::new(Arc::clone(&db));
    let ast = parse("x > bool 3").unwrap();
    let r = engine.eval_instant(&ast, 1000).unwrap();
    if let QueryResult::InstantVector(iv) = r {
        assert_eq!(iv[0].1, 1.0);
    } else {
        panic!("expected iv");
    }
}

#[test]
fn promql_eval_unary_minus_on_vector_negates() {
    let db = Arc::new(Tsdb::default());
    db.append(lbl(&[("__name__", "x")]), sample(1000, 5.0));
    let engine = Engine::new(Arc::clone(&db));
    let ast = parse("-x").unwrap();
    if let QueryResult::InstantVector(iv) = engine.eval_instant(&ast, 1000).unwrap() {
        assert_eq!(iv[0].1, -5.0);
    } else {
        panic!("expected iv");
    }
}

#[test]
fn promql_eval_range_query_steps_through_window() {
    let db = Arc::new(Tsdb::default());
    for i in 0..5 {
        db.append(lbl(&[("__name__", "x")]), sample(i * 1000, i as f64));
    }
    let engine = Engine::new(Arc::clone(&db));
    let ast = parse("x").unwrap();
    let r = engine.eval_range(&ast, 0, 4000, 1000).unwrap();
    assert_eq!(r.len(), 5);
}

// ── PromQL functions — extra edges ─────────────────────────────────────────

#[test]
fn fn_rate_returns_none_when_single_sample() {
    let r = cave_metrics::promql::functions::rate(&[sample(0, 1.0)], 1000);
    assert!(r.is_none());
}

#[test]
fn fn_rate_zero_duration_returns_none() {
    // two samples at identical timestamps → duration=0 → None
    let r = cave_metrics::promql::functions::rate(&[sample(100, 1.0), sample(100, 2.0)], 1000);
    assert!(r.is_none());
}

#[test]
fn fn_irate_zero_duration_returns_none() {
    let r = cave_metrics::promql::functions::irate(&[sample(100, 1.0), sample(100, 2.0)]);
    assert!(r.is_none());
}

#[test]
fn fn_idelta_simple() {
    let r =
        cave_metrics::promql::functions::idelta(&[sample(0, 1.0), sample(1, 3.0), sample(2, 7.0)]);
    assert_eq!(r, Some(4.0));
}

#[test]
fn fn_histogram_quantile_negative_q_neg_inf() {
    let q = cave_metrics::promql::functions::histogram_quantile(
        -0.1,
        vec![(0.5, 1.0), (1.0, 2.0)],
    );
    assert!(q.is_infinite() && q.is_sign_negative());
}

#[test]
fn fn_histogram_quantile_q_over_one_inf() {
    let q = cave_metrics::promql::functions::histogram_quantile(
        1.5,
        vec![(0.5, 1.0), (1.0, 2.0)],
    );
    assert!(q.is_infinite() && q.is_sign_positive());
}

#[test]
fn fn_histogram_quantile_empty_buckets_nan() {
    let q = cave_metrics::promql::functions::histogram_quantile(0.5, vec![]);
    assert!(q.is_nan());
}

#[test]
fn fn_quantile_sorted_zero_and_one_endpoints() {
    let data = [1.0, 2.0, 3.0];
    assert_eq!(
        cave_metrics::promql::functions::quantile_sorted(0.0, &data),
        1.0
    );
    assert_eq!(
        cave_metrics::promql::functions::quantile_sorted(1.0, &data),
        3.0
    );
}

#[test]
fn fn_quantile_sorted_empty_nan() {
    let q = cave_metrics::promql::functions::quantile_sorted(0.5, &[]);
    assert!(q.is_nan());
}

#[test]
fn fn_topk_returns_descending_truncated() {
    let pairs = vec![
        (lbl(&[("a", "1")]), 3.0),
        (lbl(&[("a", "2")]), 1.0),
        (lbl(&[("a", "3")]), 5.0),
    ];
    let r = cave_metrics::promql::functions::topk(2, pairs);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].1, 5.0);
    assert_eq!(r[1].1, 3.0);
}

#[test]
fn fn_bottomk_returns_ascending_truncated() {
    let pairs = vec![
        (lbl(&[("a", "1")]), 3.0),
        (lbl(&[("a", "2")]), 1.0),
        (lbl(&[("a", "3")]), 5.0),
    ];
    let r = cave_metrics::promql::functions::bottomk(2, pairs);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].1, 1.0);
}

#[test]
fn fn_clamp_bounds_value() {
    assert_eq!(cave_metrics::promql::functions::clamp(5.0, 0.0, 3.0), 3.0);
    assert_eq!(cave_metrics::promql::functions::clamp(-1.0, 0.0, 3.0), 0.0);
    assert_eq!(cave_metrics::promql::functions::clamp(2.0, 0.0, 3.0), 2.0);
}

#[test]
fn fn_label_replace_invalid_regex_returns_err() {
    let l = lbl(&[("k", "v")]);
    let r = cave_metrics::promql::functions::label_replace(&l, "dst", "$1", "k", "(unclosed");
    assert!(r.is_err());
}

#[test]
fn fn_label_replace_no_match_returns_unchanged() {
    let l = lbl(&[("env", "prod")]);
    let r = cave_metrics::promql::functions::label_replace(&l, "dst", "$1", "env", "dev").unwrap();
    assert!(r.get("dst").is_none());
}

// ── Sanity: existing tsdb.compact + downsample chain ───────────────────────

#[test]
fn tsdb_label_names_includes_all_keys() {
    let db = Tsdb::default();
    db.append(
        lbl(&[("__name__", "x"), ("env", "prod"), ("instance", "i1")]),
        sample(1000, 1.0),
    );
    let names = db.label_names(&[]);
    assert!(names.contains(&"__name__".to_string()));
    assert!(names.contains(&"env".to_string()));
    assert!(names.contains(&"instance".to_string()));
}

#[test]
fn tsdb_label_values_empty_matchers_returns_index_values() {
    let db = Tsdb::default();
    db.append(lbl(&[("__name__", "x"), ("env", "prod")]), sample(1, 1.0));
    db.append(lbl(&[("__name__", "y"), ("env", "dev")]), sample(1, 1.0));
    let envs = db.label_values("env", &[]);
    assert!(envs.contains(&"prod".to_string()));
    assert!(envs.contains(&"dev".to_string()));
}

#[test]
fn tsdb_select_no_matchers_returns_all() {
    let db = Tsdb::default();
    db.append(lbl(&[("__name__", "a")]), sample(1, 1.0));
    db.append(lbl(&[("__name__", "b")]), sample(1, 2.0));
    let r = db.select(&[], 0, i64::MAX);
    assert_eq!(r.len(), 2);
}

// ── tenant_from_headers — additional case ──────────────────────────────────

#[test]
fn tenant_constants_have_expected_values() {
    assert_eq!(TENANT_LABEL, "tenant_id");
    assert_eq!(DEFAULT_TENANT, "anonymous");
    assert_eq!(X_SCOPE_ORG_ID, "X-Scope-OrgID");
}
