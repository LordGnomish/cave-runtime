// SPDX-License-Identifier: AGPL-3.0-or-later
//! PromQL-like query execution: instant and range queries.

use crate::models::{QueryData, QueryResult, SeriesResult};
use crate::storage::{range_samples, TimeSeriesStore};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use std::collections::HashMap;

/// Parsed query operation.
#[derive(Debug, Clone)]
pub enum QueryOp {
    /// Raw metric selector, e.g. `http_requests_total{job="api"}`
    Select { metric: String, matchers: HashMap<String, String> },
    /// rate(metric[5m])
    Rate { inner: Box<QueryOp>, range_seconds: i64 },
    /// sum(expr) by (label, ...)
    Sum { inner: Box<QueryOp>, by: Vec<String> },
    /// avg(expr) by (label, ...)
    Avg { inner: Box<QueryOp>, by: Vec<String> },
    /// topk(k, expr)
    TopK { k: usize, inner: Box<QueryOp> },
    /// histogram_quantile(q, expr)
    HistogramQuantile { quantile: f64, inner: Box<QueryOp> },
}

/// Parse a simplified PromQL expression.
/// Supports: metric_name, metric{l=v}, rate(...[Xm]), sum(...) by (...),
///           avg(...) by (...), topk(k, ...), histogram_quantile(q, ...)
pub fn parse_expr(expr: &str) -> QueryOp {
    let expr = expr.trim();

    if let Some(rest) = expr.strip_prefix("rate(") {
        // rate(metric[Xm]) or rate(metric[Xs])
        if let Some(inner_str) = rest.strip_suffix(')') {
            if let Some(bracket) = inner_str.rfind('[') {
                let metric_part = &inner_str[..bracket];
                let range_part = &inner_str[bracket + 1..inner_str.len() - 1];
                let range_seconds = parse_duration(range_part);
                return QueryOp::Rate {
                    inner: Box::new(parse_metric_selector(metric_part)),
                    range_seconds,
                };
            }
        }
    }

    if let Some(rest) = expr.strip_prefix("sum(") {
        return parse_aggregation_op(rest, "sum");
    }

    if let Some(rest) = expr.strip_prefix("avg(") {
        return parse_aggregation_op(rest, "avg");
    }

    if let Some(rest) = expr.strip_prefix("topk(") {
        if let Some(inner_str) = rest.strip_suffix(')') {
            if let Some(comma) = inner_str.find(',') {
                let k: usize = inner_str[..comma].trim().parse().unwrap_or(5);
                let inner_expr = &inner_str[comma + 1..];
                return QueryOp::TopK {
                    k,
                    inner: Box::new(parse_expr(inner_expr)),
                };
            }
        }
    }

    if let Some(rest) = expr.strip_prefix("histogram_quantile(") {
        if let Some(inner_str) = rest.strip_suffix(')') {
            if let Some(comma) = inner_str.find(',') {
                let q: f64 = inner_str[..comma].trim().parse().unwrap_or(0.99);
                let inner_expr = &inner_str[comma + 1..];
                return QueryOp::HistogramQuantile {
                    quantile: q,
                    inner: Box::new(parse_expr(inner_expr)),
                };
            }
        }
    }

    parse_metric_selector(expr)
}

fn parse_aggregation_op(rest: &str, op: &str) -> QueryOp {
    // Find matching closing paren for the aggregation body
    let mut depth = 1i32;
    let mut close = 0usize;
    for (i, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner_expr = &rest[..close];
    let after = rest[close + 1..].trim();

    let by_labels = if let Some(by_str) = after.strip_prefix("by (").and_then(|s| s.strip_suffix(')')) {
        by_str.split(',').map(|s| s.trim().to_string()).collect()
    } else if let Some(by_str) = after.strip_prefix("by(").and_then(|s| s.strip_suffix(')')) {
        by_str.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        vec![]
    };

    let inner = Box::new(parse_expr(inner_expr));
    match op {
        "avg" => QueryOp::Avg { inner, by: by_labels },
        _ => QueryOp::Sum { inner, by: by_labels },
    }
}

fn parse_metric_selector(expr: &str) -> QueryOp {
    let expr = expr.trim();
    if let Some(brace) = expr.find('{') {
        let metric = expr[..brace].to_string();
        let label_str = expr[brace + 1..].trim_end_matches('}');
        let matchers = parse_label_matchers(label_str);
        QueryOp::Select { metric, matchers }
    } else {
        QueryOp::Select {
            metric: expr.to_string(),
            matchers: HashMap::new(),
        }
    }
}

fn parse_label_matchers(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in s.split(',') {
        let part = part.trim();
        // supports key="value" and key=value
        if let Some(eq) = part.find('=') {
            let key = part[..eq].trim().to_string();
            let val = part[eq + 1..].trim().trim_matches('"').to_string();
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
    }
    out
}

fn parse_duration(s: &str) -> i64 {
    let s = s.trim();
    if let Some(mins) = s.strip_suffix('m') {
        mins.parse::<i64>().unwrap_or(5) * 60
    } else if let Some(hrs) = s.strip_suffix('h') {
        hrs.parse::<i64>().unwrap_or(1) * 3600
    } else if let Some(days) = s.strip_suffix('d') {
        days.parse::<i64>().unwrap_or(1) * 86400
    } else {
        s.strip_suffix('s').and_then(|n| n.parse().ok()).unwrap_or(300)
    }
}

/// Execute an instant query at `at` time.
pub fn execute_query(store: &TimeSeriesStore, expr: &str, at: DateTime<Utc>) -> QueryResult {
    let op = parse_expr(expr);
    let series = eval_op(store, &op, at - Duration::hours(1), at);
    let result: Vec<SeriesResult> = series
        .into_iter()
        .map(|(labels, value)| SeriesResult {
            metric: labels,
            values: vec![[json!(at.timestamp()), json!(value.to_string())]],
        })
        .collect();

    QueryResult {
        status: "success".to_string(),
        data: QueryData {
            result_type: "vector".to_string(),
            result,
        },
    }
}

/// Execute a range query from `start` to `end` with `step` seconds between points.
pub fn execute_range_query(
    store: &TimeSeriesStore,
    expr: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    step_seconds: i64,
) -> QueryResult {
    let op = parse_expr(expr);
    let mut all: HashMap<String, (HashMap<String, String>, Vec<[serde_json::Value; 2]>)> =
        HashMap::new();

    let mut t = start;
    while t <= end {
        let window_start = t - Duration::hours(1);
        let points = eval_op(store, &op, window_start, t);
        for (labels, value) in points {
            let fp = {
                let mut pairs: Vec<_> = labels.iter().collect();
                pairs.sort_by_key(|(k, _)| k.as_str());
                pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
            };
            let entry = all.entry(fp).or_insert_with(|| (labels, vec![]));
            entry.1.push([json!(t.timestamp()), json!(value.to_string())]);
        }
        t += Duration::seconds(step_seconds);
    }

    let result: Vec<SeriesResult> = all
        .into_values()
        .map(|(labels, values)| SeriesResult { metric: labels, values })
        .collect();

    QueryResult {
        status: "success".to_string(),
        data: QueryData {
            result_type: "matrix".to_string(),
            result,
        },
    }
}

/// Evaluate a query operation over [start, end], returning (labels, scalar) pairs.
fn eval_op(
    store: &TimeSeriesStore,
    op: &QueryOp,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<(HashMap<String, String>, f64)> {
    match op {
        QueryOp::Select { metric, matchers } => {
            let mut out = vec![];
            for ts in store.series.values() {
                if ts.metric_name != *metric {
                    continue;
                }
                if !matchers.iter().all(|(k, v)| ts.labels.get(k).map(|lv| lv == v).unwrap_or(false)) {
                    continue;
                }
                let samples: Vec<f64> = range_samples(ts, start, end).map(|s| s.value).collect();
                if let Some(&last) = samples.last() {
                    let mut labels = ts.labels.clone();
                    labels.insert("__name__".to_string(), ts.metric_name.clone());
                    out.push((labels, last));
                }
            }
            out
        }

        QueryOp::Rate { inner, range_seconds } => {
            let inner_start = end - Duration::seconds(*range_seconds);
            let inner_results = eval_op(store, inner, inner_start, end);
            // For counters: approximate rate = (last - first) / range
            inner_results
                .into_iter()
                .map(|(labels, _last)| {
                    let fp = labels_to_fp(&labels);
                    let rate = store.series.get(&fp).map(|ts| {
                        let samples: Vec<f64> = range_samples(ts, inner_start, end)
                            .map(|s| s.value)
                            .collect();
                        if samples.len() < 2 {
                            return 0.0;
                        }
                        let delta = samples.last().unwrap() - samples.first().unwrap();
                        delta / *range_seconds as f64
                    }).unwrap_or(0.0);
                    (labels, rate)
                })
                .collect()
        }

        QueryOp::Sum { inner, by } => {
            aggregate(eval_op(store, inner, start, end), by, |vals| vals.iter().sum())
        }

        QueryOp::Avg { inner, by } => {
            aggregate(eval_op(store, inner, start, end), by, |vals| {
                if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / vals.len() as f64 }
            })
        }

        QueryOp::TopK { k, inner } => {
            let mut results = eval_op(store, inner, start, end);
            results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            results.truncate(*k);
            results
        }

        QueryOp::HistogramQuantile { quantile, inner } => {
            // Simplified: sort values and return the quantile position
            let mut results = eval_op(store, inner, start, end);
            if results.is_empty() {
                return results;
            }
            results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let idx = ((results.len() as f64 * quantile) as usize).min(results.len() - 1);
            let val = results[idx].1;
            // Return single result with quantile label
            let mut labels = HashMap::new();
            labels.insert("quantile".to_string(), quantile.to_string());
            vec![(labels, val)]
        }
    }
}

fn labels_to_fp(labels: &HashMap<String, String>) -> String {
    let name = labels.get("__name__").cloned().unwrap_or_default();
    let mut pairs: Vec<_> = labels.iter().filter(|(k, _)| *k != "__name__").collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    let label_str: String = pairs
        .into_iter()
        .map(|(k, v)| format!("{k}=\"{v}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}{{{label_str}}}")
}

fn aggregate(
    results: Vec<(HashMap<String, String>, f64)>,
    by: &[String],
    agg_fn: impl Fn(&[f64]) -> f64,
) -> Vec<(HashMap<String, String>, f64)> {
    let mut groups: HashMap<String, (HashMap<String, String>, Vec<f64>)> = HashMap::new();
    for (labels, val) in results {
        let group_labels: HashMap<String, String> = if by.is_empty() {
            HashMap::new()
        } else {
            by.iter()
                .filter_map(|k| labels.get(k).map(|v| (k.clone(), v.clone())))
                .collect()
        };
        let key = {
            let mut pairs: Vec<_> = group_labels.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
        };
        let entry = groups.entry(key).or_insert_with(|| (group_labels, vec![]));
        entry.1.push(val);
    }
    groups
        .into_values()
        .map(|(labels, vals)| (labels, agg_fn(&vals)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("5m"), 300);
        assert_eq!(parse_duration("1h"), 3600);
        assert_eq!(parse_duration("2d"), 172800);
        assert_eq!(parse_duration("30s"), 30);
    }

    #[test]
    fn test_parse_metric_selector() {
        match parse_expr("http_requests_total") {
            QueryOp::Select { metric, .. } => assert_eq!(metric, "http_requests_total"),
            _ => panic!("expected Select"),
        }
    }

    #[test]
    fn test_parse_selector_with_labels() {
        match parse_expr(r#"http_requests_total{job="api",status="200"}"#) {
            QueryOp::Select { metric, matchers } => {
                assert_eq!(metric, "http_requests_total");
                assert_eq!(matchers.get("job").map(|s| s.as_str()), Some("api"));
            }
            _ => panic!("expected Select"),
        }
    }

    #[test]
    fn test_parse_rate() {
        match parse_expr("rate(http_requests_total[5m])") {
            QueryOp::Rate { range_seconds, .. } => assert_eq!(range_seconds, 300),
            _ => panic!("expected Rate"),
        }
    }

    #[test]
    fn test_execute_query_empty_store() {
        let store = TimeSeriesStore::default();
        let result = execute_query(&store, "missing_metric", Utc::now());
        assert_eq!(result.status, "success");
        assert!(result.data.result.is_empty());
    }
}
