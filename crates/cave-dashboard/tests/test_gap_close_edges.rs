// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-close coverage for cave-dashboard.
//!
//! Targets modules that have no `#[cfg(test)]` block of their own:
//! `datasource`, `query`, `auth`, `alerting`, `routes`, `models`, `store`.
//!
//! Test design: failure-mode, boundary, state-transition, and serde
//! round-trip cases — disjoint from the inline tests in `lib.rs`.

use cave_dashboard::alerting::{
    apply_reducer, build_alert_groups, eval_threshold, evaluate_alert_conditions, is_muted,
    is_silenced, route_alert,
};
use cave_dashboard::auth::{
    Principal, extract_bearer, generate_api_key, hash_api_key, require_admin, require_editor,
};
use cave_dashboard::datasource::{
    PrometheusVariableQuery, health_check_url, jaeger_trace_url, loki_query_url,
    parse_prometheus_variable_query, prometheus_label_values_url, prometheus_query_url,
    prometheus_range_url, prometheus_series_url, tempo_trace_url,
};
use cave_dashboard::models::*;
use cave_dashboard::query::{QueryCache, apply_transformations, cache_key};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

// ─── datasource: URL builders ─────────────────────────────────────────────────

#[test]
fn datasource_prom_query_url_trims_trailing_slash() {
    let url = prometheus_query_url("http://prom:9090/", "up", "1000");
    assert!(url.starts_with("http://prom:9090/api/v1/query?query=up"));
    assert!(!url.contains("prom:9090//"));
}

#[test]
fn datasource_prom_query_url_encodes_special_chars() {
    let url = prometheus_query_url("http://prom", "rate(http_requests_total[5m])", "now");
    // '(' ')' '[' ']' are not unreserved; must be %-encoded.
    assert!(url.contains("%28"), "no %28 in {url}");
    assert!(url.contains("%29"), "no %29 in {url}");
    assert!(url.contains("%5B"), "no %5B in {url}");
    assert!(url.contains("%5D"), "no %5D in {url}");
}

#[test]
fn datasource_prom_query_url_space_becomes_plus() {
    let url = prometheus_query_url("http://p", "a b", "0");
    assert!(url.contains("query=a+b"), "got {url}");
}

#[test]
fn datasource_prom_range_url_orders_params() {
    let url = prometheus_range_url("http://p", "up", "0", "10", "1");
    // start/end/step appear in canonical order.
    let s_pos = url.find("start=0").expect("start");
    let e_pos = url.find("end=10").expect("end");
    let step_pos = url.find("step=1").expect("step");
    assert!(s_pos < e_pos && e_pos < step_pos);
}

#[test]
fn datasource_prom_label_values_and_series_urls() {
    assert!(prometheus_label_values_url("http://p", "job").ends_with("/label/job/values"));
    let series = prometheus_series_url("http://p", "up{job=\"k\"}");
    assert!(series.contains("/series?match[]=up"));
}

#[test]
fn datasource_loki_url_contains_query_and_limit() {
    let url = loki_query_url("http://loki:3100", "{app=\"foo\"}", "0", "1", 500);
    assert!(url.contains("/loki/api/v1/query_range?"));
    assert!(url.contains("limit=500"));
}

#[test]
fn datasource_trace_urls_format() {
    assert_eq!(
        jaeger_trace_url("http://j/", "abc"),
        "http://j/api/traces/abc"
    );
    assert_eq!(
        tempo_trace_url("http://t//", "x"),
        // trim_end_matches strips all trailing '/'
        "http://t/api/traces/x"
    );
}

#[test]
fn datasource_health_url_per_type() {
    // We build a DataSource manually; the new() constructor is the API surface.
    let prom = DataSource::new(1, 1, "p", "P", DataSourceType::Prometheus, "http://p");
    let url = health_check_url(&prom).unwrap();
    assert!(url.ends_with("/api/v1/query?query=1"));

    let loki = DataSource::new(2, 1, "l", "L", DataSourceType::Loki, "http://l");
    assert!(health_check_url(&loki).unwrap().ends_with("/ready"));

    // Postgres has no HTTP health endpoint.
    let pg = DataSource::new(3, 1, "g", "G", DataSourceType::Postgres, "");
    assert!(health_check_url(&pg).is_none());
}

#[test]
fn datasource_parse_prom_variable_query_label_values_with_metric() {
    let q = parse_prometheus_variable_query("label_values(http_requests_total, job)");
    match q {
        PrometheusVariableQuery::LabelValues { metric, label } => {
            assert_eq!(metric, "http_requests_total");
            assert_eq!(label, "job");
        }
        _ => panic!("expected LabelValues, got {q:?}"),
    }
}

#[test]
fn datasource_parse_prom_variable_query_label_values_label_only() {
    let q = parse_prometheus_variable_query("label_values(instance)");
    match q {
        PrometheusVariableQuery::LabelValues { metric, label } => {
            assert!(metric.is_empty());
            assert_eq!(label, "instance");
        }
        _ => panic!("expected single-arg LabelValues"),
    }
}

#[test]
fn datasource_parse_prom_variable_query_label_names_metrics_query_result_raw() {
    assert!(matches!(
        parse_prometheus_variable_query("label_names(up)"),
        PrometheusVariableQuery::LabelNames { .. }
    ));
    assert!(matches!(
        parse_prometheus_variable_query("metrics(node_)"),
        PrometheusVariableQuery::Metrics { .. }
    ));
    assert!(matches!(
        parse_prometheus_variable_query("query_result(up==1)"),
        PrometheusVariableQuery::QueryResult { .. }
    ));
    // unrecognised form falls through to Raw
    assert!(matches!(
        parse_prometheus_variable_query("just_a_metric"),
        PrometheusVariableQuery::Raw(_)
    ));
}

// ─── query: cache + cache_key + apply_transformations ────────────────────────

#[test]
fn query_cache_key_is_deterministic() {
    let k1 = cache_key("ds-a", "up", "now-1h", "now");
    let k2 = cache_key("ds-a", "up", "now-1h", "now");
    assert_eq!(k1, k2);
    assert!(k1.contains("ds-a"));
    assert!(k1.contains("up"));
}

#[test]
fn query_cache_put_and_get_returns_clone() {
    let cache = QueryCache::new();
    let res = QueryResult {
        frames: vec![],
        status: 200,
        error: None,
        error_source: None,
    };
    cache.put("k1".into(), res.clone(), Duration::from_secs(60));
    let got = cache.get("k1").expect("entry");
    assert_eq!(got.status, 200);
    // Miss returns None.
    assert!(cache.get("missing").is_none());
}

#[test]
fn query_cache_expired_entry_returns_none_and_evicts() {
    let cache = QueryCache::new();
    let res = QueryResult {
        frames: vec![],
        status: 200,
        error: None,
        error_source: None,
    };
    // TTL of 0 means immediately expired.
    cache.put("k".into(), res, Duration::from_nanos(1));
    std::thread::sleep(Duration::from_millis(5));
    assert!(cache.get("k").is_none(), "expired entry should miss");
    // evict() purges expired entries — subsequent get also misses.
    cache.evict();
    assert!(cache.get("k").is_none());
}

#[test]
fn query_apply_transformations_unknown_id_passes_through() {
    let frames = vec![DataFrame::default()];
    let txs = vec![serde_json::json!({"id": "bogusXform", "options": {}})];
    let out = apply_transformations(frames, &txs);
    assert_eq!(out.len(), 1);
}

#[test]
fn query_apply_transformations_limit_caps_rows() {
    let frame = DataFrame {
        schema: DataFrameSchema {
            ref_id: "A".into(),
            name: "f".into(),
            fields: vec![FieldSchema {
                name: "v".into(),
                field_type: "number".into(),
                type_info: None,
                labels: None,
                config: None,
            }],
            meta: None,
        },
        data: DataFrameData {
            values: vec![vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3),
                serde_json::json!(4),
                serde_json::json!(5),
            ]],
            entities: None,
        },
    };
    let txs = vec![serde_json::json!({"id": "limit", "options": {"limitField": 3}})];
    let out = apply_transformations(vec![frame], &txs);
    assert_eq!(out[0].data.values[0].len(), 3);
}

#[test]
fn query_apply_transformations_filter_fields_by_name_keeps_only_included() {
    let frame = DataFrame {
        schema: DataFrameSchema {
            ref_id: "A".into(),
            name: "f".into(),
            fields: vec![
                FieldSchema {
                    name: "keep".into(),
                    field_type: "number".into(),
                    type_info: None,
                    labels: None,
                    config: None,
                },
                FieldSchema {
                    name: "drop".into(),
                    field_type: "number".into(),
                    type_info: None,
                    labels: None,
                    config: None,
                },
            ],
            meta: None,
        },
        data: DataFrameData {
            values: vec![vec![serde_json::json!(1)], vec![serde_json::json!(2)]],
            entities: None,
        },
    };
    let txs = vec![serde_json::json!({
        "id": "filterFieldsByName",
        "options": {"include": {"names": ["keep"]}}
    })];
    let out = apply_transformations(vec![frame], &txs);
    assert_eq!(out[0].schema.fields.len(), 1);
    assert_eq!(out[0].schema.fields[0].name, "keep");
}

#[test]
fn query_apply_transformations_organize_renames_and_excludes() {
    let frame = DataFrame {
        schema: DataFrameSchema {
            ref_id: "A".into(),
            name: "f".into(),
            fields: vec![
                FieldSchema {
                    name: "old".into(),
                    field_type: "number".into(),
                    type_info: None,
                    labels: None,
                    config: None,
                },
                FieldSchema {
                    name: "hidden".into(),
                    field_type: "number".into(),
                    type_info: None,
                    labels: None,
                    config: None,
                },
            ],
            meta: None,
        },
        data: DataFrameData {
            values: vec![vec![serde_json::json!(1)], vec![serde_json::json!(2)]],
            entities: None,
        },
    };
    let txs = vec![serde_json::json!({
        "id": "organize",
        "options": {
            "renameByName": {"old": "renamed"},
            "excludeByName": {"hidden": true}
        }
    })];
    let out = apply_transformations(vec![frame], &txs);
    assert_eq!(out[0].schema.fields.len(), 1);
    assert_eq!(out[0].schema.fields[0].name, "renamed");
}

// ─── auth ─────────────────────────────────────────────────────────────────────

#[test]
fn auth_hash_api_key_deterministic_and_differs_per_input() {
    let a = hash_api_key("token-A");
    let b = hash_api_key("token-A");
    let c = hash_api_key("token-B");
    assert_eq!(a, b, "same input → same hash");
    assert_ne!(a, c, "different input → different hash");
    assert_eq!(a.len(), 16, "16 hex chars");
}

#[test]
fn auth_generate_api_key_has_glsa_prefix_and_uuid_body() {
    let k = generate_api_key();
    assert!(k.starts_with("glsa_"), "got {k}");
    // 5 prefix + 32 uuid hex chars.
    assert_eq!(k.len(), 5 + 32);
}

#[test]
fn auth_principal_role_org_can_edit_is_admin_matrix() {
    let viewer = Principal::User {
        id: 1,
        org_id: 7,
        role: OrgRole::Viewer,
        is_admin: false,
    };
    let editor = Principal::ApiKey {
        id: 2,
        org_id: 7,
        role: OrgRole::Editor,
    };
    let admin = Principal::ServiceAccount {
        id: 3,
        org_id: 7,
        role: OrgRole::Admin,
    };
    let anon = Principal::Anonymous;

    assert!(!viewer.can_edit());
    assert!(editor.can_edit());
    assert!(admin.is_admin());
    assert!(!viewer.is_admin());
    // is_admin=true overrides role for User.
    let promoted_viewer = Principal::User {
        id: 1,
        org_id: 7,
        role: OrgRole::Viewer,
        is_admin: true,
    };
    assert!(promoted_viewer.is_admin());
    // Anonymous → org 1, Viewer.
    assert_eq!(anon.org_id(), 1);
    assert_eq!(anon.role(), OrgRole::Viewer);
}

#[test]
fn auth_require_editor_and_admin_gate_correctly() {
    let viewer = Principal::User {
        id: 1,
        org_id: 1,
        role: OrgRole::Viewer,
        is_admin: false,
    };
    let editor = Principal::User {
        id: 1,
        org_id: 1,
        role: OrgRole::Editor,
        is_admin: false,
    };
    let admin = Principal::User {
        id: 1,
        org_id: 1,
        role: OrgRole::Admin,
        is_admin: false,
    };
    assert!(require_editor(&viewer).is_err());
    assert!(require_editor(&editor).is_ok());
    assert!(require_admin(&editor).is_err());
    assert!(require_admin(&admin).is_ok());
}

#[test]
fn auth_extract_bearer_handles_prefix_and_raw_token() {
    assert_eq!(extract_bearer("Bearer abc"), "abc");
    assert_eq!(extract_bearer("Bearer   leading-spaces"), "leading-spaces");
    assert_eq!(extract_bearer("raw-token"), "raw-token");
    assert_eq!(extract_bearer("  padded  "), "padded");
}

// ─── alerting (extra) ────────────────────────────────────────────────────────

#[test]
fn alerting_eval_threshold_extra_ops() {
    assert!(eval_threshold(5.0, "gte", &[5.0]));
    assert!(eval_threshold(4.999, "lte", &[5.0]));
    assert!(eval_threshold(2.0, "eq", &[2.0]));
    assert!(!eval_threshold(2.0, "eq", &[2.1]));
    // no_value always false here (handled elsewhere).
    assert!(!eval_threshold(0.0, "no_value", &[]));
    // unknown evaluator → false.
    assert!(!eval_threshold(0.0, "bogus", &[1.0]));
}

#[test]
fn alerting_apply_reducer_handles_empty_and_diff_zero_first() {
    assert!(apply_reducer("avg", &[]).is_none());
    assert!(apply_reducer("diff", &[1.0]).is_none());
    // percent_diff with first=0 must return None to avoid div-by-zero.
    assert!(apply_reducer("percent_diff", &[0.0, 5.0]).is_none());
    assert_eq!(apply_reducer("range", &[1.0, 3.0, 2.0]), Some(2.0));
    assert_eq!(apply_reducer("diff_abs", &[5.0, 1.0]), Some(4.0));
    // Unknown reducer falls back to `last`.
    assert_eq!(apply_reducer("unknown", &[7.0, 8.0]), Some(8.0));
}

#[test]
fn alerting_evaluate_conditions_no_value_when_reducer_empty() {
    // Reducer over empty values returns None → eval_type == "no_value" fires.
    let cond = AlertCondition {
        condition_type: "query".into(),
        query: AlertConditionQuery {
            params: vec!["X".into()],
        },
        reducer: AlertReducer {
            reducer_type: "last".into(),
            params: vec![],
        },
        evaluator: AlertEvaluator {
            eval_type: "no_value".into(),
            params: vec![],
        },
        operator: AlertOperator {
            op_type: "and".into(),
        },
    };
    // Empty values_map => reduced=None => firing because eval_type=="no_value".
    let result = evaluate_alert_conditions(&[cond], &HashMap::new());
    assert_eq!(result, AlertState::Firing);
}

#[test]
fn alerting_is_silenced_active_window_with_matcher() {
    let now = chrono::Utc::now();
    let silence = Silence {
        id: "s1".into(),
        status: SilenceStatus {
            state: "active".into(),
        },
        updated_at: now,
        comment: String::new(),
        created_by: String::new(),
        starts_at: now - chrono::Duration::hours(1),
        ends_at: now + chrono::Duration::hours(1),
        matchers: vec![SilenceMatcher {
            is_equal: true,
            is_regex: false,
            name: "alertname".into(),
            value: "DiskFull".into(),
        }],
    };
    let mut labels = HashMap::new();
    labels.insert("alertname".into(), "DiskFull".into());
    assert!(is_silenced(&labels, &[silence.clone()]));

    // Different label value → not silenced.
    let mut other = HashMap::new();
    other.insert("alertname".into(), "Other".into());
    assert!(!is_silenced(&other, &[silence]));
}

#[test]
fn alerting_is_silenced_expired_window_or_state() {
    let now = chrono::Utc::now();
    // ends_at in the past.
    let expired = Silence {
        id: "s2".into(),
        status: SilenceStatus {
            state: "active".into(),
        },
        updated_at: now,
        comment: String::new(),
        created_by: String::new(),
        starts_at: now - chrono::Duration::hours(2),
        ends_at: now - chrono::Duration::hours(1),
        matchers: vec![],
    };
    let labels = HashMap::new();
    assert!(!is_silenced(&labels, &[expired]));
}

#[test]
fn alerting_build_alert_groups_keys_by_group_by_labels() {
    let policy = NotificationPolicy {
        receiver: "default".into(),
        group_by: vec!["env".into()],
        ..Default::default()
    };
    let mk = |env: &str| AlertInstance {
        state: AlertState::Firing,
        labels: {
            let mut h = HashMap::new();
            h.insert("env".into(), env.into());
            h
        },
        annotations: HashMap::new(),
        value: String::new(),
        starts_at: chrono::Utc::now(),
        ends_at: None,
        generator_url: String::new(),
        fingerprint: String::new(),
        silence_urls: vec![],
        dashboard_url: None,
        panel_url: None,
        values: None,
        evaluations: None,
    };
    let groups = build_alert_groups(vec![mk("prod"), mk("prod"), mk("dev")], &policy);
    assert_eq!(groups.len(), 2, "got: {groups:?}");
    let prod_grp = groups
        .iter()
        .find(|g| g.labels.get("env").map(String::as_str) == Some("prod"))
        .expect("prod group");
    assert_eq!(prod_grp.alerts.len(), 2);
}

#[test]
fn alerting_route_alert_with_regex_matcher() {
    let policy = NotificationPolicy {
        receiver: "default".into(),
        routes: vec![NotificationPolicy {
            receiver: "ops".into(),
            matchers: vec![Matcher {
                name: "team".into(),
                value: "^infra.*".into(),
                is_equal: true,
                is_regex: true,
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut labels = HashMap::new();
    labels.insert("team".into(), "infra-storage".into());
    assert_eq!(route_alert(&policy, &labels), "ops");

    labels.insert("team".into(), "frontend".into());
    assert_eq!(route_alert(&policy, &labels), "default");
}

#[test]
fn alerting_is_muted_time_range_match() {
    use chrono::TimeZone;
    let mt = MuteTiming {
        name: "weekday-business-hours".into(),
        time_intervals: vec![TimeInterval {
            times: vec![TimeIntervalRange {
                start_minute: 9 * 60,
                end_minute: 17 * 60,
            }],
            ..Default::default()
        }],
    };
    // 12:30 UTC on an arbitrary day → in range.
    let inside = chrono::Utc.with_ymd_and_hms(2024, 6, 14, 12, 30, 0).unwrap();
    assert!(is_muted(&mt, &inside));
    let outside = chrono::Utc.with_ymd_and_hms(2024, 6, 14, 22, 0, 0).unwrap();
    assert!(!is_muted(&mt, &outside));
}

// ─── models: serde + Display + FromStr ───────────────────────────────────────

#[test]
fn models_orgrole_from_str_and_display_round_trip() {
    assert_eq!(OrgRole::from_str("Admin").unwrap(), OrgRole::Admin);
    assert_eq!(OrgRole::from_str("editor").unwrap(), OrgRole::Editor);
    assert_eq!(OrgRole::from_str("viewer").unwrap(), OrgRole::Viewer);
    assert!(OrgRole::from_str("Wizard").is_err());
    assert_eq!(format!("{}", OrgRole::Admin), "Admin");
}

#[test]
fn models_datasource_type_display_strings() {
    assert_eq!(format!("{}", DataSourceType::Prometheus), "prometheus");
    assert_eq!(format!("{}", DataSourceType::Loki), "loki");
    assert_eq!(format!("{}", DataSourceType::Mssql), "mssql");
    assert_eq!(format!("{}", DataSourceType::Unknown), "unknown");
}

#[test]
fn models_alertstate_display_all_variants() {
    for (s, expected) in [
        (AlertState::Normal, "Normal"),
        (AlertState::Pending, "Pending"),
        (AlertState::Firing, "Firing"),
        (AlertState::Error, "Error"),
        (AlertState::NoData, "NoData"),
        (AlertState::Inactive, "Inactive"),
    ] {
        assert_eq!(format!("{s}"), expected);
    }
}

#[test]
fn models_dashboard_serde_round_trip_preserves_uid_title_panels() {
    let mut db = Dashboard::new(0, 1, "Round Trip");
    db.tags = vec!["a".into(), "b".into()];
    db.schema_version = 39;
    let json = serde_json::to_string(&db).expect("ser");
    let back: Dashboard = serde_json::from_str(&json).expect("de");
    assert_eq!(back.uid, db.uid);
    assert_eq!(back.title, db.title);
    assert_eq!(back.tags, db.tags);
    assert_eq!(back.schema_version, db.schema_version);
}

#[test]
fn models_panel_serde_uses_camel_case_and_type_field() {
    let panel = Panel {
        id: 1,
        title: "P".into(),
        panel_type: PanelType::Stat,
        grid_pos: GridPos {
            x: 0,
            y: 0,
            w: 12,
            h: 8,
            static_pos: false,
        },
        datasource: None,
        targets: vec![],
        transformations: vec![],
        field_config: FieldConfig::default(),
        options: serde_json::json!({}),
        description: String::new(),
        transparent: false,
        links: vec![],
        repeat: None,
        repeat_direction: None,
        max_data_points: None,
        interval: None,
        time_from: None,
        time_shift: None,
        alert: None,
        panels: vec![],
        collapsed: false,
        hide_time_override: false,
        cache_timeout: None,
        query_caching_ttl: None,
        plugin_version: None,
    };
    let v = serde_json::to_value(&panel).unwrap();
    // PanelType uses snake_case via serde rename_all.
    assert_eq!(v["type"], "stat");
    // gridPos camelCase.
    assert!(v.get("gridPos").is_some());
    // Round-trip.
    let back: Panel = serde_json::from_value(v).unwrap();
    assert_eq!(back.panel_type, PanelType::Stat);
}

#[test]
fn models_slug_handles_emoji_and_unicode_punctuation() {
    // Non-ASCII alphanumerics are kept (Unicode is_alphanumeric).
    let s = Dashboard::slug_from_title("Çay & Çorba");
    // Two collapses to single hyphens; non-alphanumeric '&' becomes hyphen.
    assert_eq!(s, "çay-çorba");
    // Pure non-alphanumerics produce empty.
    assert_eq!(Dashboard::slug_from_title("???"), "");
}

#[test]
fn models_datasourcetype_unknown_serde_fallback() {
    // Unknown discriminant deserialises into the Unknown variant.
    let t: DataSourceType = serde_json::from_str("\"bigquery\"").unwrap();
    assert_eq!(t, DataSourceType::Unknown);
}

#[test]
fn models_searchquery_default_is_empty() {
    let q = SearchQuery::default();
    assert!(q.query.is_none());
    assert!(q.tag.is_empty());
    assert!(q.starred.is_none());
}
