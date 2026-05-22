// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Datasource proxy and health-check layer.
//!
//! Each datasource type knows how to:
//!   - Build query URLs
//!   - Translate health-check requests
//!   - Run variable/label queries (for template variable population)

use crate::models::{
    DataFrame, DataFrameData, DataFrameSchema, DataSource, DataSourceHealthStatus, DataSourceType,
    DsQuery, FieldSchema, QueryResult,
};
use std::collections::HashMap;

// ─── URL builders ─────────────────────────────────────────────────────────────

/// Build the Prometheus instant-query URL.
pub fn prometheus_query_url(base_url: &str, expr: &str, time: &str) -> String {
    format!(
        "{}/api/v1/query?query={}&time={}",
        base_url.trim_end_matches('/'),
        urlencoded(expr),
        time
    )
}

/// Build the Prometheus range-query URL.
pub fn prometheus_range_url(
    base_url: &str,
    expr: &str,
    start: &str,
    end: &str,
    step: &str,
) -> String {
    format!(
        "{}/api/v1/query_range?query={}&start={}&end={}&step={}",
        base_url.trim_end_matches('/'),
        urlencoded(expr),
        start,
        end,
        step
    )
}

/// Build the Prometheus label values URL (used by variable queries).
pub fn prometheus_label_values_url(base_url: &str, label: &str) -> String {
    format!(
        "{}/api/v1/label/{}/values",
        base_url.trim_end_matches('/'),
        urlencoded(label)
    )
}

/// Build the Prometheus series URL (used by `label_values(metric, label)` variable queries).
pub fn prometheus_series_url(base_url: &str, selector: &str) -> String {
    format!(
        "{}/api/v1/series?match[]={}",
        base_url.trim_end_matches('/'),
        urlencoded(selector)
    )
}

/// Build the Loki log-query URL.
pub fn loki_query_url(base_url: &str, expr: &str, start: &str, end: &str, limit: u32) -> String {
    format!(
        "{}/loki/api/v1/query_range?query={}&start={}&end={}&limit={}",
        base_url.trim_end_matches('/'),
        urlencoded(expr),
        start,
        end,
        limit
    )
}

/// Build the Jaeger trace URL.
pub fn jaeger_trace_url(base_url: &str, trace_id: &str) -> String {
    format!("{}/api/traces/{}", base_url.trim_end_matches('/'), trace_id)
}

/// Build Tempo trace URL.
pub fn tempo_trace_url(base_url: &str, trace_id: &str) -> String {
    format!("{}/api/traces/{}", base_url.trim_end_matches('/'), trace_id)
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "+".to_string(),
            c if c.is_alphanumeric() || "-._~".contains(c) => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

// ─── Health check ─────────────────────────────────────────────────────────────

/// Build a health-check request for a datasource.
/// Returns the URL to GET; a 200 response means healthy.
pub fn health_check_url(ds: &DataSource) -> Option<String> {
    match ds.ds_type {
        DataSourceType::Prometheus => Some(format!(
            "{}/api/v1/query?query=1",
            ds.url.trim_end_matches('/')
        )),
        DataSourceType::Loki => Some(format!("{}/ready", ds.url.trim_end_matches('/'))),
        DataSourceType::Jaeger => Some(format!("{}/api/services", ds.url.trim_end_matches('/'))),
        DataSourceType::Tempo => Some(format!("{}/ready", ds.url.trim_end_matches('/'))),
        DataSourceType::Elasticsearch => {
            Some(format!("{}/_cluster/health", ds.url.trim_end_matches('/')))
        }
        DataSourceType::InfluxDb => Some(format!("{}/ping", ds.url.trim_end_matches('/'))),
        DataSourceType::Postgres | DataSourceType::Mysql | DataSourceType::Mssql => {
            // SQL health checks need a DB connection — not applicable for HTTP proxy
            None
        }
        _ => None,
    }
}

/// Perform a health check using reqwest (async).
pub async fn check_health(ds: &DataSource) -> DataSourceHealthStatus {
    let Some(url) = health_check_url(ds) else {
        return DataSourceHealthStatus {
            status: "ok".into(),
            message: "Health check not supported for this datasource type".into(),
            details: None,
        };
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap_or_default();

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => DataSourceHealthStatus {
            status: "ok".into(),
            message: "Data source is working and can connect to Prometheus".into(),
            details: None,
        },
        Ok(resp) => DataSourceHealthStatus {
            status: "error".into(),
            message: format!("HTTP {} from {}", resp.status(), url),
            details: None,
        },
        Err(e) => DataSourceHealthStatus {
            status: "error".into(),
            message: format!("Connection failed: {e}"),
            details: None,
        },
    }
}

// ─── Variable query ───────────────────────────────────────────────────────────

/// Parse a Prometheus variable query (e.g. `label_values(metric, label)` or `label_names()`).
pub fn parse_prometheus_variable_query(query: &str) -> PrometheusVariableQuery {
    let q = query.trim();
    if let Some(inner) = q
        .strip_prefix("label_values(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.splitn(2, ',').collect();
        if parts.len() == 2 {
            return PrometheusVariableQuery::LabelValues {
                metric: parts[0].trim().to_string(),
                label: parts[1].trim().to_string(),
            };
        } else if parts.len() == 1 {
            return PrometheusVariableQuery::LabelValues {
                metric: String::new(),
                label: parts[0].trim().to_string(),
            };
        }
    }
    if let Some(inner) = q
        .strip_prefix("label_names(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return PrometheusVariableQuery::LabelNames {
            selector: inner.trim().to_string(),
        };
    }
    if let Some(inner) = q.strip_prefix("metrics(").and_then(|s| s.strip_suffix(')')) {
        return PrometheusVariableQuery::Metrics {
            filter: inner.trim().to_string(),
        };
    }
    if let Some(inner) = q
        .strip_prefix("query_result(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return PrometheusVariableQuery::QueryResult {
            expr: inner.trim().to_string(),
        };
    }
    PrometheusVariableQuery::Raw(q.to_string())
}

#[derive(Debug, Clone)]
pub enum PrometheusVariableQuery {
    LabelValues { metric: String, label: String },
    LabelNames { selector: String },
    Metrics { filter: String },
    QueryResult { expr: String },
    Raw(String),
}

// ─── Query proxy ──────────────────────────────────────────────────────────────

/// Execute a datasource query (stub — returns empty frames in demo mode).
/// A production implementation would forward to the actual backend.
pub async fn execute_query(ds: &DataSource, query: &DsQuery) -> QueryResult {
    // Extract common fields
    let expr = query
        .params
        .get("expr")
        .or_else(|| query.params.get("query"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let ref_id = query.ref_id.clone();

    match ds.ds_type {
        DataSourceType::Prometheus => execute_prometheus_query(ds, &ref_id, expr).await,
        DataSourceType::Loki => execute_loki_query(ds, &ref_id, expr).await,
        _ => empty_result(&ref_id),
    }
}

async fn execute_prometheus_query(ds: &DataSource, ref_id: &str, expr: &str) -> QueryResult {
    let url = prometheus_range_url(&ds.url, expr, "now-1h", "now", "60s");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap_or_default();

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(json) => parse_prometheus_response(ref_id, &json),
            Err(e) => error_result(ref_id, &e.to_string()),
        },
        Ok(resp) => error_result(ref_id, &format!("HTTP {}", resp.status())),
        Err(e) => error_result(ref_id, &e.to_string()),
    }
}

fn parse_prometheus_response(ref_id: &str, json: &serde_json::Value) -> QueryResult {
    let mut frames = Vec::new();
    if let Some(results) = json
        .get("data")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.as_array())
    {
        for result in results {
            let metric = result
                .get("metric")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let labels: HashMap<String, String> =
                serde_json::from_value(metric).unwrap_or_default();

            let mut times: Vec<serde_json::Value> = Vec::new();
            let mut values: Vec<serde_json::Value> = Vec::new();

            if let Some(vals) = result.get("values").and_then(|v| v.as_array()) {
                for pair in vals {
                    if let Some(arr) = pair.as_array() {
                        if arr.len() >= 2 {
                            let ts_ms = arr[0].as_f64().map(|t| t * 1000.0).unwrap_or(0.0) as i64;
                            let val_str = arr[1].as_str().unwrap_or("0");
                            let val: f64 = val_str.parse().unwrap_or(0.0);
                            times.push(serde_json::Value::Number(serde_json::Number::from(ts_ms)));
                            values.push(serde_json::json!(val));
                        }
                    }
                }
            }

            frames.push(DataFrame {
                schema: DataFrameSchema {
                    ref_id: ref_id.to_string(),
                    name: labels.get("__name__").cloned().unwrap_or_default(),
                    fields: vec![
                        FieldSchema {
                            name: "Time".into(),
                            field_type: "time".into(),
                            type_info: None,
                            labels: None,
                            config: None,
                        },
                        FieldSchema {
                            name: "Value".into(),
                            field_type: "number".into(),
                            type_info: None,
                            labels: Some(labels),
                            config: None,
                        },
                    ],
                    meta: None,
                },
                data: DataFrameData {
                    values: vec![times, values],
                    entities: None,
                },
            });
        }
    }
    QueryResult {
        frames,
        status: 200,
        error: None,
        error_source: None,
    }
}

async fn execute_loki_query(ds: &DataSource, ref_id: &str, expr: &str) -> QueryResult {
    let url = loki_query_url(&ds.url, expr, "now-1h", "now", 1000);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(json) => parse_loki_response(ref_id, &json),
            Err(e) => error_result(ref_id, &e.to_string()),
        },
        Ok(resp) => error_result(ref_id, &format!("HTTP {}", resp.status())),
        Err(e) => error_result(ref_id, &e.to_string()),
    }
}

fn parse_loki_response(ref_id: &str, json: &serde_json::Value) -> QueryResult {
    let mut times = Vec::new();
    let mut lines = Vec::new();

    if let Some(streams) = json
        .get("data")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.as_array())
    {
        for stream in streams {
            if let Some(values) = stream.get("values").and_then(|v| v.as_array()) {
                for entry in values {
                    if let Some(pair) = entry.as_array() {
                        if pair.len() >= 2 {
                            let ts_ns = pair[0].as_str().unwrap_or("0").parse::<i64>().unwrap_or(0);
                            let ts_ms = ts_ns / 1_000_000;
                            let line = pair[1].as_str().unwrap_or("").to_string();
                            times.push(serde_json::Value::Number(serde_json::Number::from(ts_ms)));
                            lines.push(serde_json::Value::String(line));
                        }
                    }
                }
            }
        }
    }

    let frame = DataFrame {
        schema: DataFrameSchema {
            ref_id: ref_id.to_string(),
            name: "logs".into(),
            fields: vec![
                FieldSchema {
                    name: "Time".into(),
                    field_type: "time".into(),
                    type_info: None,
                    labels: None,
                    config: None,
                },
                FieldSchema {
                    name: "Line".into(),
                    field_type: "string".into(),
                    type_info: None,
                    labels: None,
                    config: None,
                },
            ],
            meta: Some(serde_json::json!({"type": "log"})),
        },
        data: DataFrameData {
            values: vec![times, lines],
            entities: None,
        },
    };
    QueryResult {
        frames: vec![frame],
        status: 200,
        error: None,
        error_source: None,
    }
}

fn empty_result(ref_id: &str) -> QueryResult {
    QueryResult {
        frames: vec![DataFrame {
            schema: DataFrameSchema {
                ref_id: ref_id.to_string(),
                name: String::new(),
                fields: vec![],
                meta: None,
            },
            data: DataFrameData {
                values: vec![],
                entities: None,
            },
        }],
        status: 200,
        error: None,
        error_source: None,
    }
}

fn error_result(ref_id: &str, msg: &str) -> QueryResult {
    QueryResult {
        frames: vec![],
        status: 500,
        error: Some(msg.to_string()),
        error_source: Some("downstream".into()),
    }
}
