// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LogQL evaluator — takes a parsed `Query` and executes it against the store.

use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;

use super::ast::*;
use crate::models::{
    Direction, Labels, LogEntry, MatrixResult, QueryData, StreamResult, TimestampNs, VectorResult,
};
use crate::store::LogStore;

/// An evaluated log entry with extracted labels from pipeline parsers.
#[derive(Debug, Clone)]
pub struct ProcessedEntry {
    pub ts: TimestampNs,
    pub line: String,
    pub labels: HashMap<String, String>,
}

// ── Label matcher evaluation ─────────────────────────────────────────────────

/// Check if a `Labels` set satisfies a stream selector.
pub fn labels_match(labels: &Labels, selector: &StreamSelector) -> bool {
    for m in &selector.matchers {
        let actual = labels.get(&m.name);
        match (&m.op, actual) {
            (MatchOp::Eq, Some(v)) => {
                if v != m.value.as_str() {
                    return false;
                }
            }
            (MatchOp::Eq, None) => return false,
            (MatchOp::Neq, Some(v)) => {
                if v == m.value.as_str() {
                    return false;
                }
            }
            (MatchOp::Neq, None) => {} // absence satisfies !=
            (MatchOp::Re, actual_opt) => {
                let v = actual_opt.unwrap_or("");
                let re = match Regex::new(&m.value) {
                    Ok(r) => r,
                    Err(_) => return false,
                };
                if !re.is_match(v) {
                    return false;
                }
            }
            (MatchOp::NotRe, actual_opt) => {
                let v = actual_opt.unwrap_or("");
                let re = match Regex::new(&m.value) {
                    Ok(r) => r,
                    Err(_) => return false,
                };
                if re.is_match(v) {
                    return false;
                }
            }
        }
    }
    true
}

// ── Pipeline stage execution ─────────────────────────────────────────────────

/// Apply the log pipeline to a single entry, returning `None` if it is filtered out.
pub fn apply_pipeline(
    entry: &LogEntry,
    base_labels: &Labels,
    pipeline: &[PipelineStage],
) -> Option<ProcessedEntry> {
    let mut line = entry.line.clone();
    let mut extra: HashMap<String, String> = base_labels.0.clone();
    extra.extend(entry.metadata.clone());

    for stage in pipeline {
        match stage {
            PipelineStage::LineFilter(lf) => {
                if !apply_line_filter(&line, lf) {
                    return None;
                }
            }
            PipelineStage::Parser(p) => {
                apply_parser(&line, p, &mut extra);
            }
            PipelineStage::LabelFilter(lf) => {
                if !apply_label_filter(&extra, lf) {
                    return None;
                }
            }
            PipelineStage::LineFormat(lf) => {
                line = apply_line_format(&lf.template, &extra);
            }
            PipelineStage::LabelFormat(lf) => {
                apply_label_format(&lf.mappings, &mut extra);
            }
            PipelineStage::Decolorize(_) => {
                line = strip_ansi(&line);
            }
            PipelineStage::Unwrap(_) => {
                // Unwrap is used in range aggregations; skip in log queries.
            }
            PipelineStage::Drop(d) => {
                apply_drop(&d.labels, &mut extra);
            }
            PipelineStage::Keep(k) => {
                apply_keep(&k.labels, &mut extra);
            }
        }
    }

    Some(ProcessedEntry {
        ts: entry.ts,
        line,
        labels: extra,
    })
}

fn apply_line_filter(line: &str, lf: &LineFilter) -> bool {
    match lf {
        LineFilter::Contains(s) => line.contains(s.as_str()),
        LineFilter::NotContains(s) => !line.contains(s.as_str()),
        LineFilter::Matches(pat) => Regex::new(pat).map(|r| r.is_match(line)).unwrap_or(false),
        LineFilter::NotMatches(pat) => Regex::new(pat).map(|r| !r.is_match(line)).unwrap_or(true),
        LineFilter::IpMatch(pat) => pat.line_matches(line),
        LineFilter::IpNotMatch(pat) => !pat.line_matches(line),
    }
}

fn apply_parser(line: &str, parser: &Parser, extra: &mut HashMap<String, String>) {
    match parser {
        Parser::Json => parse_json(line, extra),
        Parser::Logfmt => parse_logfmt(line, extra),
        Parser::Regexp(pat) => parse_regexp(line, pat, extra),
        Parser::Pattern(pat) => parse_pattern(line, pat, extra),
        Parser::Unpack => parse_unpack(line, extra),
    }
}

fn parse_json(line: &str, extra: &mut HashMap<String, String>) {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
        if let Some(obj) = val.as_object() {
            for (k, v) in obj {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                extra.insert(k.clone(), s);
            }
        }
    }
}

fn parse_logfmt(line: &str, extra: &mut HashMap<String, String>) {
    // logfmt: key=value key="quoted value" key=value ...
    let mut rest = line;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let eq = match rest.find('=') {
            Some(i) => i,
            None => break,
        };
        let key = rest[..eq].trim();
        rest = &rest[eq + 1..];
        let (value, consumed) = if rest.starts_with('"') {
            let end = rest[1..].find('"').map(|i| i + 2).unwrap_or(rest.len());
            (&rest[1..end - 1], end)
        } else {
            let end = rest.find(' ').unwrap_or(rest.len());
            (&rest[..end], end)
        };
        if !key.is_empty() {
            extra.insert(key.to_owned(), value.to_owned());
        }
        rest = &rest[consumed..];
    }
}

fn parse_regexp(line: &str, pat: &str, extra: &mut HashMap<String, String>) {
    if let Ok(re) = Regex::new(pat) {
        if let Some(caps) = re.captures(line) {
            for name in re.capture_names().flatten() {
                if let Some(m) = caps.name(name) {
                    extra.insert(name.to_owned(), m.as_str().to_owned());
                }
            }
        }
    }
}

fn parse_pattern(line: &str, pat: &str, extra: &mut HashMap<String, String>) {
    // Loki pattern: `<label>text<label2>...` where `<_>` discards.
    // Convert to regex: replace <label> with named capture groups.
    let mut regex_str = String::new();
    let mut remaining = pat;
    while !remaining.is_empty() {
        if remaining.starts_with('<') {
            let close = remaining.find('>').unwrap_or(remaining.len());
            let label = &remaining[1..close];
            remaining = &remaining[close + 1..];
            if label == "_" {
                regex_str.push_str("(?:.*)");
            } else {
                regex_str.push_str(&format!("(?P<{}>[^<]*)", regex::escape(label)));
            }
        } else {
            let next_lt = remaining.find('<').unwrap_or(remaining.len());
            regex_str.push_str(&regex::escape(&remaining[..next_lt]));
            remaining = &remaining[next_lt..];
        }
    }
    parse_regexp(line, &regex_str, extra);
}

fn parse_unpack(line: &str, extra: &mut HashMap<String, String>) {
    // Unpack expects a JSON object with a "_entry" field for the real log line
    // and all other fields as labels.
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
        if let Some(obj) = val.as_object() {
            for (k, v) in obj {
                if k == "_entry" {
                    continue;
                }
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                extra.insert(k.clone(), s);
            }
        }
    }
}

/// Apply `label_replace` to a metric result, rewriting `dst_label` on each
/// series from regex captures of `src_label`'s value. The regex is anchored
/// to the whole source value (Prometheus semantics); non-matching series are
/// returned unchanged, and an expansion that yields an empty string deletes
/// `dst_label`.
fn apply_label_replace(data: QueryData, lr: &LabelReplace) -> QueryData {
    let re = match Regex::new(&format!("^(?:{})$", lr.regex)) {
        Ok(r) => r,
        // An invalid regex matches nothing → every series passes through.
        Err(_) => return data,
    };
    let rewrite = |metric: &mut HashMap<String, String>| {
        let src_val = metric.get(&lr.src_label).cloned().unwrap_or_default();
        if let Some(caps) = re.captures(&src_val) {
            let mut expanded = String::new();
            caps.expand(&lr.replacement, &mut expanded);
            if expanded.is_empty() {
                metric.remove(&lr.dst_label);
            } else {
                metric.insert(lr.dst_label.clone(), expanded);
            }
        }
    };
    match data {
        QueryData::Vector(mut v) => {
            for r in &mut v {
                rewrite(&mut r.metric);
            }
            QueryData::Vector(v)
        }
        QueryData::Matrix(mut m) => {
            for r in &mut m {
                rewrite(&mut r.metric);
            }
            QueryData::Matrix(m)
        }
        // label_replace is undefined over log streams — pass through.
        other => other,
    }
}

fn apply_label_filter(labels: &HashMap<String, String>, lf: &LabelFilter) -> bool {
    let actual = labels.get(&lf.label);
    match &lf.value {
        LabelFilterValue::String(expected) => {
            let v = actual.map(|s| s.as_str()).unwrap_or("");
            match &lf.op {
                CompareOp::Eq => v == expected.as_str(),
                CompareOp::Neq => v != expected.as_str(),
                CompareOp::Re => Regex::new(expected).map(|r| r.is_match(v)).unwrap_or(false),
                CompareOp::NotRe => Regex::new(expected).map(|r| !r.is_match(v)).unwrap_or(true),
                _ => false,
            }
        }
        LabelFilterValue::Float(expected) => {
            let v: f64 = actual.and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
            match &lf.op {
                CompareOp::Eq => (v - expected).abs() < f64::EPSILON,
                CompareOp::Neq => (v - expected).abs() >= f64::EPSILON,
                CompareOp::Gt => v > *expected,
                CompareOp::Gte => v >= *expected,
                CompareOp::Lt => v < *expected,
                CompareOp::Lte => v <= *expected,
                _ => false,
            }
        }
        LabelFilterValue::Duration(expected) => {
            // Compare as nanoseconds.
            let v_ns: u64 = actual.and_then(|s| parse_duration_str(s)).unwrap_or(0);
            let exp_ns = expected.as_nanos() as u64;
            match &lf.op {
                CompareOp::Eq => v_ns == exp_ns,
                CompareOp::Neq => v_ns != exp_ns,
                CompareOp::Gt => v_ns > exp_ns,
                CompareOp::Gte => v_ns >= exp_ns,
                CompareOp::Lt => v_ns < exp_ns,
                CompareOp::Lte => v_ns <= exp_ns,
                _ => false,
            }
        }
        LabelFilterValue::Bytes(expected) => {
            let v: u64 = actual.and_then(|s| s.parse().ok()).unwrap_or(0);
            match &lf.op {
                CompareOp::Eq => v == *expected,
                CompareOp::Neq => v != *expected,
                CompareOp::Gt => v > *expected,
                CompareOp::Gte => v >= *expected,
                CompareOp::Lt => v < *expected,
                CompareOp::Lte => v <= *expected,
                _ => false,
            }
        }
        LabelFilterValue::Ip(pat) => {
            // Parse the label value as an address; an unparseable value never
            // matches (so `=` is false and `!=` is true for it).
            let hit = actual
                .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
                .map(|addr| pat.matches(addr))
                .unwrap_or(false);
            match &lf.op {
                CompareOp::Eq => hit,
                CompareOp::Neq => !hit,
                _ => false,
            }
        }
    }
}

fn parse_duration_str(s: &str) -> Option<u64> {
    if let Some(rest) = s.strip_suffix("ms") {
        rest.parse::<f64>().ok().map(|n| (n * 1_000_000.0) as u64)
    } else if let Some(rest) = s.strip_suffix('s') {
        rest.parse::<f64>()
            .ok()
            .map(|n| (n * 1_000_000_000.0) as u64)
    } else if let Some(rest) = s.strip_suffix('m') {
        rest.parse::<f64>()
            .ok()
            .map(|n| (n * 60_000_000_000.0) as u64)
    } else if let Some(rest) = s.strip_suffix('h') {
        rest.parse::<f64>()
            .ok()
            .map(|n| (n * 3_600_000_000_000.0) as u64)
    } else {
        s.parse::<u64>().ok() // raw nanoseconds
    }
}

fn apply_line_format(template: &str, labels: &HashMap<String, String>) -> String {
    // Simple `{{.label}}` substitution.
    let mut out = template.to_owned();
    for (k, v) in labels {
        out = out.replace(&format!("{{{{.{}}}}}", k), v);
    }
    out
}

/// Does the current value of a label satisfy a `drop`/`keep` value matcher?
fn drop_keep_matcher_passes(value: &str, m: &LabelMatcher) -> bool {
    match m.op {
        MatchOp::Eq => value == m.value,
        MatchOp::Neq => value != m.value,
        MatchOp::Re => Regex::new(&m.value).map(|r| r.is_match(value)).unwrap_or(false),
        MatchOp::NotRe => Regex::new(&m.value).map(|r| !r.is_match(value)).unwrap_or(false),
    }
}

/// `| drop` — remove a label if any entry targets it (bare name always removes;
/// a matcher removes only when the label's value passes the matcher).
fn apply_drop(entries: &[DropKeepLabel], labels: &mut HashMap<String, String>) {
    labels.retain(|name, value| {
        let targeted = entries.iter().any(|e| match e {
            DropKeepLabel::Name(n) => n == name,
            DropKeepLabel::Matcher(m) => &m.name == name && drop_keep_matcher_passes(value, m),
        });
        !targeted
    });
}

/// `| keep` — retain a label only if some entry approves it (bare name approves
/// the label; a matcher approves only when the label's value passes). All other
/// labels — including the original stream labels — are removed.
fn apply_keep(entries: &[DropKeepLabel], labels: &mut HashMap<String, String>) {
    labels.retain(|name, value| {
        entries.iter().any(|e| match e {
            DropKeepLabel::Name(n) => n == name,
            DropKeepLabel::Matcher(m) => &m.name == name && drop_keep_matcher_passes(value, m),
        })
    });
}

fn apply_label_format(mappings: &[(String, String)], labels: &mut HashMap<String, String>) {
    for (new_name, old_name) in mappings {
        if let Some(v) = labels.remove(old_name.as_str()) {
            labels.insert(new_name.clone(), v);
        }
    }
}

fn strip_ansi(s: &str) -> String {
    // Remove ESC[...m sequences.
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
    re.replace_all(s, "").into_owned()
}

// ── Main evaluator ────────────────────────────────────────────────────────────

pub struct Evaluator {
    store: Arc<LogStore>,
}

impl Evaluator {
    pub fn new(store: Arc<LogStore>) -> Self {
        Self { store }
    }

    /// Execute a log query and return `QueryData::Streams`.
    pub fn eval_log_query(
        &self,
        tenant: &str,
        query: &LogQuery,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        limit: usize,
        direction: Direction,
    ) -> QueryData {
        let fps = self
            .store
            .matching_fps(tenant, |labels| labels_match(labels, &query.selector));

        let raw = self
            .store
            .query_entries(tenant, &fps, start_ns, end_ns, limit, direction);

        let mut streams: Vec<StreamResult> = Vec::new();
        let mut total = 0usize;

        'outer: for (_, labels, entries) in raw {
            let mut values = Vec::new();
            for entry in &entries {
                if total >= limit {
                    break 'outer;
                }
                if let Some(processed) = apply_pipeline(entry, &labels, &query.pipeline) {
                    values.push((processed.ts.to_string(), processed.line));
                    total += 1;
                }
            }
            if !values.is_empty() {
                streams.push(StreamResult {
                    stream: labels.0,
                    values,
                });
            }
        }

        QueryData::Streams(streams)
    }

    /// Execute a metric (range aggregation or vector aggregation) query.
    pub fn eval_metric_query(
        &self,
        tenant: &str,
        query: &MetricQuery,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        step_ns: i64,
    ) -> QueryData {
        match query {
            MetricQuery::RangeAgg(ra) => self.eval_range_agg(tenant, ra, start_ns, end_ns, step_ns),
            MetricQuery::VectorAgg(va) => {
                self.eval_vector_agg(tenant, va, start_ns, end_ns, step_ns)
            }
            MetricQuery::BinaryExpr(be) => self.eval_binary(tenant, be, start_ns, end_ns, step_ns),
            MetricQuery::Literal(n) => QueryData::Vector(vec![VectorResult {
                metric: HashMap::new(),
                value: (start_ns as f64 / 1e9, n.to_string()),
            }]),
            MetricQuery::LabelReplace(lr) => {
                let inner = self.eval_metric_query(tenant, &lr.inner, start_ns, end_ns, step_ns);
                apply_label_replace(inner, lr)
            }
        }
    }

    fn eval_range_agg(
        &self,
        tenant: &str,
        ra: &LogRangeAggregation,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        step_ns: i64,
    ) -> QueryData {
        let fps = self
            .store
            .matching_fps(tenant, |labels| labels_match(labels, &ra.query.selector));

        // `offset <d>` shifts the lookup window back by `d`; results are then
        // re-plotted on the original time axis (PromQL/LogQL semantics).
        let offset_ns = ra.offset.map(|d| d.as_nanos() as i64).unwrap_or(0);
        let start_ns = start_ns - offset_ns;
        let end_ns = end_ns - offset_ns;

        // Compute buckets across all matching streams.
        let buckets = match ra.agg {
            RangeAgg::Rate => {
                let range_ns = ra.range.as_nanos() as i64;
                let counts = self
                    .store
                    .count_over_buckets(tenant, &fps, start_ns, end_ns, step_ns);
                counts
                    .into_iter()
                    .map(|(ts, c)| (ts, c / (range_ns as f64 / 1e9)))
                    .collect::<Vec<_>>()
            }
            RangeAgg::CountOverTime => self
                .store
                .count_over_buckets(tenant, &fps, start_ns, end_ns, step_ns),
            RangeAgg::BytesOverTime => self
                .store
                .bytes_over_buckets(tenant, &fps, start_ns, end_ns, step_ns),
            RangeAgg::BytesRate => {
                let range_ns = ra.range.as_nanos() as i64;
                self.store
                    .bytes_over_buckets(tenant, &fps, start_ns, end_ns, step_ns)
                    .into_iter()
                    .map(|(ts, b)| (ts, b / (range_ns as f64 / 1e9)))
                    .collect()
            }
            RangeAgg::AbsentOverTime => {
                // Returns 1 if there are NO entries in the range, else no result.
                let counts = self
                    .store
                    .count_over_buckets(tenant, &fps, start_ns, end_ns, step_ns);
                counts
                    .into_iter()
                    .map(|(ts, c)| (ts, if c == 0.0 { 1.0 } else { 0.0 }))
                    .collect()
            }
            // For unwrap-based range aggs we default to count_over_time as a stub;
            // full implementation requires extracting numeric values from log lines.
            _ => self
                .store
                .count_over_buckets(tenant, &fps, start_ns, end_ns, step_ns),
        };

        // Aggregate per stream into a matrix result.
        let metric: HashMap<String, String> = HashMap::new();
        let values: Vec<(f64, String)> = buckets
            .into_iter()
            .filter(|(_, v)| *v != 0.0)
            .map(|(ts, v)| ((ts + offset_ns) as f64 / 1e9, format!("{:.6}", v)))
            .collect();

        QueryData::Matrix(vec![MatrixResult { metric, values }])
    }

    fn eval_vector_agg(
        &self,
        tenant: &str,
        va: &VectorAggregation,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        step_ns: i64,
    ) -> QueryData {
        // Evaluate inner query as matrix, then aggregate vectors.
        let inner_data = self.eval_metric_query(tenant, &va.inner, start_ns, end_ns, step_ns);

        let matrix = match inner_data {
            QueryData::Matrix(m) => m,
            other => return other,
        };

        // Group by labels according to grouping spec.
        let mut groups: HashMap<Vec<(String, String)>, Vec<f64>> = HashMap::new();

        for series in &matrix {
            let group_key: Vec<(String, String)> = match &va.grouping {
                None => vec![],
                Some(g) if g.without => {
                    let mut pairs: Vec<(String, String)> = series
                        .metric
                        .iter()
                        .filter(|(k, _)| !g.labels.contains(k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    pairs.sort();
                    pairs
                }
                Some(g) => {
                    let mut pairs: Vec<(String, String)> = g
                        .labels
                        .iter()
                        .filter_map(|l| series.metric.get(l).map(|v| (l.clone(), v.clone())))
                        .collect();
                    pairs.sort();
                    pairs
                }
            };

            let vals: Vec<f64> = series
                .values
                .iter()
                .map(|(_, v)| v.parse::<f64>().unwrap_or(0.0))
                .collect();

            groups.entry(group_key).or_default().extend(vals);
        }

        // Apply aggregation function.
        let results: Vec<MatrixResult> = groups
            .into_iter()
            .map(|(key, vals)| {
                let metric: HashMap<String, String> = key.into_iter().collect();
                let agg_val = match &va.agg {
                    VectorAgg::Sum => vals.iter().sum(),
                    VectorAgg::Avg => vals.iter().sum::<f64>() / vals.len() as f64,
                    VectorAgg::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    VectorAgg::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                    VectorAgg::Count => vals.len() as f64,
                    VectorAgg::Stddev => {
                        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                        (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64)
                            .sqrt()
                    }
                    VectorAgg::Stdvar => {
                        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                        vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64
                    }
                    VectorAgg::Topk(_) | VectorAgg::Bottomk(_) | VectorAgg::Quantile(_) => {
                        vals.iter().sum::<f64>() // simplified; real impl needs per-series tracking
                    }
                };
                MatrixResult {
                    metric,
                    values: vec![(start_ns as f64 / 1e9, format!("{:.6}", agg_val))],
                }
            })
            .collect();

        QueryData::Matrix(results)
    }

    fn eval_binary(
        &self,
        tenant: &str,
        be: &BinaryExpr,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        step_ns: i64,
    ) -> QueryData {
        let lhs = self.eval_metric_query(tenant, &be.lhs, start_ns, end_ns, step_ns);
        let rhs = self.eval_metric_query(tenant, &be.rhs, start_ns, end_ns, step_ns);

        let lhs_vals: Vec<f64> = extract_values(&lhs);
        let rhs_vals: Vec<f64> = extract_values(&rhs);

        let result_vals: Vec<f64> = lhs_vals
            .iter()
            .zip(rhs_vals.iter())
            .map(|(l, r)| match &be.op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                BinOp::Div => {
                    if *r == 0.0 {
                        f64::NAN
                    } else {
                        l / r
                    }
                }
                BinOp::Mod => l % r,
                BinOp::Pow => l.powf(*r),
                BinOp::CmpEq(b) => {
                    if (l - r).abs() < f64::EPSILON {
                        if *b { 1.0 } else { *l }
                    } else {
                        if *b { 0.0 } else { f64::NAN }
                    }
                }
                BinOp::CmpNeq(b) => {
                    if (l - r).abs() >= f64::EPSILON {
                        if *b { 1.0 } else { *l }
                    } else {
                        if *b { 0.0 } else { f64::NAN }
                    }
                }
                BinOp::CmpGt(b) => {
                    if l > r {
                        if *b { 1.0 } else { *l }
                    } else {
                        if *b { 0.0 } else { f64::NAN }
                    }
                }
                BinOp::CmpGte(b) => {
                    if l >= r {
                        if *b { 1.0 } else { *l }
                    } else {
                        if *b { 0.0 } else { f64::NAN }
                    }
                }
                BinOp::CmpLt(b) => {
                    if l < r {
                        if *b { 1.0 } else { *l }
                    } else {
                        if *b { 0.0 } else { f64::NAN }
                    }
                }
                BinOp::CmpLte(b) => {
                    if l <= r {
                        if *b { 1.0 } else { *l }
                    } else {
                        if *b { 0.0 } else { f64::NAN }
                    }
                }
                _ => l + r,
            })
            .collect();

        let values: Vec<(f64, String)> = result_vals
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    (start_ns + i as i64 * step_ns) as f64 / 1e9,
                    format!("{:.6}", v),
                )
            })
            .collect();

        QueryData::Matrix(vec![MatrixResult {
            metric: HashMap::new(),
            values,
        }])
    }
}

fn extract_values(data: &QueryData) -> Vec<f64> {
    match data {
        QueryData::Matrix(m) => m
            .iter()
            .flat_map(|r| {
                r.values
                    .iter()
                    .map(|(_, v)| v.parse::<f64>().unwrap_or(0.0))
            })
            .collect(),
        QueryData::Vector(v) => v
            .iter()
            .map(|r| r.value.1.parse::<f64>().unwrap_or(0.0))
            .collect(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Labels, LogEntry};
    use std::time::Duration;
    use std::collections::HashMap;

    fn make_labels(pairs: &[(&str, &str)]) -> Labels {
        Labels::new(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn labels_match_eq() {
        let labels = make_labels(&[("app", "nginx"), ("env", "prod")]);
        let sel = StreamSelector {
            matchers: vec![
                LabelMatcher {
                    name: "app".into(),
                    op: MatchOp::Eq,
                    value: "nginx".into(),
                },
                LabelMatcher {
                    name: "env".into(),
                    op: MatchOp::Eq,
                    value: "prod".into(),
                },
            ],
        };
        assert!(labels_match(&labels, &sel));
    }

    #[test]
    fn labels_match_neq() {
        let labels = make_labels(&[("app", "nginx")]);
        let sel = StreamSelector {
            matchers: vec![LabelMatcher {
                name: "app".into(),
                op: MatchOp::Neq,
                value: "apache".into(),
            }],
        };
        assert!(labels_match(&labels, &sel));
    }

    #[test]
    fn labels_match_regex() {
        let labels = make_labels(&[("env", "production")]);
        let sel = StreamSelector {
            matchers: vec![LabelMatcher {
                name: "env".into(),
                op: MatchOp::Re,
                value: "prod.*".into(),
            }],
        };
        assert!(labels_match(&labels, &sel));
    }

    #[test]
    fn pipeline_line_filter_contains() {
        let entry = LogEntry::new(0, "error: something went wrong");
        let labels = make_labels(&[]);
        let pipeline = vec![PipelineStage::LineFilter(LineFilter::Contains(
            "error".into(),
        ))];
        assert!(apply_pipeline(&entry, &labels, &pipeline).is_some());

        let pipeline_neg = vec![PipelineStage::LineFilter(LineFilter::Contains(
            "debug".into(),
        ))];
        assert!(apply_pipeline(&entry, &labels, &pipeline_neg).is_none());
    }

    #[test]
    fn pipeline_json_parser() {
        let entry = LogEntry::new(0, r#"{"status":200,"method":"GET","path":"/api"}"#);
        let labels = make_labels(&[]);
        let pipeline = vec![
            PipelineStage::Parser(Parser::Json),
            PipelineStage::LabelFilter(LabelFilter {
                label: "status".into(),
                op: CompareOp::Gte,
                value: LabelFilterValue::Float(200.0),
            }),
        ];
        let result = apply_pipeline(&entry, &labels, &pipeline);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.labels.get("method").map(|s| s.as_str()), Some("GET"));
    }

    #[test]
    fn pipeline_logfmt_parser() {
        let entry = LogEntry::new(0, r#"level=info msg="request done" status=200"#);
        let labels = make_labels(&[]);
        let pipeline = vec![PipelineStage::Parser(Parser::Logfmt)];
        let result = apply_pipeline(&entry, &labels, &pipeline).unwrap();
        assert_eq!(result.labels.get("level").map(|s| s.as_str()), Some("info"));
        assert_eq!(result.labels.get("status").map(|s| s.as_str()), Some("200"));
    }

    #[test]
    fn pipeline_line_format() {
        let entry = LogEntry::new(0, "ignored");
        let labels = make_labels(&[("method", "GET"), ("status", "200")]);
        let pipeline = vec![PipelineStage::LineFormat(LineFormat {
            template: "{{.method}} {{.status}}".into(),
        })];
        let result = apply_pipeline(&entry, &labels, &pipeline).unwrap();
        assert_eq!(result.line, "GET 200");
    }

    #[test]
    fn pipeline_decolorize() {
        let entry = LogEntry::new(0, "\x1b[31mERROR\x1b[0m: something bad");
        let labels = make_labels(&[]);
        let pipeline = vec![PipelineStage::Decolorize(Decolorize)];
        let result = apply_pipeline(&entry, &labels, &pipeline).unwrap();
        assert_eq!(result.line, "ERROR: something bad");
    }

    #[test]
    fn pipeline_drop_bare_names() {
        let entry = LogEntry::new(0, "ignored");
        let labels = make_labels(&[("app", "api"), ("level", "info"), ("status", "200")]);
        let pipeline = vec![PipelineStage::Drop(DropLabels {
            labels: vec![
                DropKeepLabel::Name("level".into()),
                DropKeepLabel::Name("status".into()),
            ],
        })];
        let r = apply_pipeline(&entry, &labels, &pipeline).unwrap();
        assert_eq!(r.labels.get("app").map(|s| s.as_str()), Some("api"));
        assert!(!r.labels.contains_key("level"));
        assert!(!r.labels.contains_key("status"));
    }

    #[test]
    fn pipeline_drop_conditional_matcher() {
        // `| drop level="debug"` removes `level` ONLY when its value is "debug".
        let drop = PipelineStage::Drop(DropLabels {
            labels: vec![DropKeepLabel::Matcher(LabelMatcher {
                name: "level".into(),
                op: MatchOp::Eq,
                value: "debug".into(),
            })],
        });

        let debug = make_labels(&[("level", "debug"), ("app", "api")]);
        let r1 = apply_pipeline(&LogEntry::new(0, "x"), &debug, std::slice::from_ref(&drop)).unwrap();
        assert!(!r1.labels.contains_key("level"));

        let info = make_labels(&[("level", "info"), ("app", "api")]);
        let r2 = apply_pipeline(&LogEntry::new(0, "x"), &info, std::slice::from_ref(&drop)).unwrap();
        assert_eq!(r2.labels.get("level").map(|s| s.as_str()), Some("info"));
    }

    #[test]
    fn pipeline_keep_drops_everything_else() {
        // `| keep level` keeps only `level`, dropping app + status (incl. stream labels).
        let entry = LogEntry::new(0, "ignored");
        let labels = make_labels(&[("app", "api"), ("level", "warn"), ("status", "200")]);
        let pipeline = vec![PipelineStage::Keep(KeepLabels {
            labels: vec![DropKeepLabel::Name("level".into())],
        })];
        let r = apply_pipeline(&entry, &labels, &pipeline).unwrap();
        assert_eq!(r.labels.get("level").map(|s| s.as_str()), Some("warn"));
        assert!(!r.labels.contains_key("app"));
        assert!(!r.labels.contains_key("status"));
    }

    #[test]
    fn pipeline_keep_conditional_matcher() {
        // `| keep level, status="500"` keeps level always, status only when ==500.
        let keep = PipelineStage::Keep(KeepLabels {
            labels: vec![
                DropKeepLabel::Name("level".into()),
                DropKeepLabel::Matcher(LabelMatcher {
                    name: "status".into(),
                    op: MatchOp::Eq,
                    value: "500".into(),
                }),
            ],
        });

        let hit = make_labels(&[("level", "error"), ("status", "500"), ("app", "api")]);
        let r1 = apply_pipeline(&LogEntry::new(0, "x"), &hit, std::slice::from_ref(&keep)).unwrap();
        assert_eq!(r1.labels.get("status").map(|s| s.as_str()), Some("500"));
        assert_eq!(r1.labels.get("level").map(|s| s.as_str()), Some("error"));
        assert!(!r1.labels.contains_key("app"));

        let miss = make_labels(&[("level", "info"), ("status", "200"), ("app", "api")]);
        let r2 = apply_pipeline(&LogEntry::new(0, "x"), &miss, std::slice::from_ref(&keep)).unwrap();
        assert!(!r2.labels.contains_key("status")); // matcher failed → dropped
        assert_eq!(r2.labels.get("level").map(|s| s.as_str()), Some("info"));
    }

    #[test]
    fn range_agg_offset_shifts_window_back() {
        let store = LogStore::new();
        let hour = 3_600_000_000_000i64; // 1h in ns
        let base = 100 * hour;
        // Two entries live at `base`; the evaluation window sits 1h LATER.
        store
            .push(
                "t",
                make_labels(&[("app", "a")]),
                vec![LogEntry::new(base, "hit"), LogEntry::new(base + 1, "hit2")],
            )
            .unwrap();
        let eval = Evaluator::new(store);

        let selector = StreamSelector {
            matchers: vec![LabelMatcher {
                name: "app".into(),
                op: MatchOp::Eq,
                value: "a".into(),
            }],
        };
        let mk = |offset: Option<Duration>| {
            MetricQuery::RangeAgg(LogRangeAggregation {
                agg: RangeAgg::CountOverTime,
                query: LogQuery {
                    selector: selector.clone(),
                    pipeline: vec![],
                },
                range: Duration::from_nanos(hour as u64),
                grouping: None,
                offset,
            })
        };
        let (start, end, step) = (base + hour, base + 2 * hour, hour);

        // Without offset, the window is empty (data is 1h in the past).
        let no_off = eval.eval_metric_query("t", &mk(None), start, end, step);
        let sum_no = matrix_sum(&no_off);
        assert_eq!(sum_no, 0.0, "window 1h after data must be empty without offset");

        // `offset 1h` pulls the window back onto the data → both entries counted.
        let with_off =
            eval.eval_metric_query("t", &mk(Some(Duration::from_nanos(hour as u64))), start, end, step);
        let sum_off = matrix_sum(&with_off);
        assert_eq!(sum_off, 2.0, "offset 1h must capture both entries");
    }

    fn matrix_sum(d: &QueryData) -> f64 {
        if let QueryData::Matrix(m) = d {
            m.iter()
                .flat_map(|s| s.values.iter())
                .map(|(_, v)| v.parse::<f64>().unwrap_or(0.0))
                .sum()
        } else {
            panic!("expected matrix, got {:?}", d);
        }
    }

    #[test]
    fn evaluator_log_query() {
        let store = LogStore::new();
        let t = 1_000_000_000i64;
        store
            .push(
                "tenant",
                make_labels(&[("app", "test")]),
                vec![
                    LogEntry::new(t, "error: disk full"),
                    LogEntry::new(t + 1_000_000, "info: started"),
                ],
            )
            .unwrap();

        let eval = Evaluator::new(store);
        let selector = StreamSelector {
            matchers: vec![LabelMatcher {
                name: "app".into(),
                op: MatchOp::Eq,
                value: "test".into(),
            }],
        };
        let pipeline = vec![PipelineStage::LineFilter(LineFilter::Contains(
            "error".into(),
        ))];
        let query = LogQuery { selector, pipeline };

        let data = eval.eval_log_query("tenant", &query, 0, t + 2_000_000, 100, Direction::Forward);
        if let QueryData::Streams(streams) = data {
            assert_eq!(streams.len(), 1);
            assert_eq!(streams[0].values.len(), 1);
            assert!(streams[0].values[0].1.contains("error"));
        } else {
            panic!("expected streams");
        }
    }

    #[test]
    fn ip_line_filter_keeps_only_matching_lines() {
        use super::super::parser::Parser;
        let store = LogStore::new();
        let t = 1_000_000_000i64;
        store
            .push(
                "tenant",
                make_labels(&[("app", "gw")]),
                vec![
                    LogEntry::new(t, "req from 192.168.4.5 ok"),
                    LogEntry::new(t + 1, "req from 10.0.0.9 ok"),
                    LogEntry::new(t + 2, "req from 192.168.250.1 ok"),
                ],
            )
            .unwrap();
        let eval = Evaluator::new(store);

        // `|= ip("192.168.0.0/16")` keeps the two 192.168.x lines.
        let q = match Parser::parse_query(r#"{app="gw"} |= ip("192.168.0.0/16")"#).unwrap() {
            Query::Log(lq) => lq,
            _ => panic!("expected log query"),
        };
        let data = eval.eval_log_query("tenant", &q, 0, t + 10, 100, Direction::Forward);
        let QueryData::Streams(streams) = data else {
            panic!("expected streams");
        };
        assert_eq!(streams[0].values.len(), 2);
        assert!(streams[0].values.iter().all(|(_, l)| l.contains("192.168")));

        // `!= ip(...)` is the complement: only the 10.0.0.9 line survives.
        let qn = match Parser::parse_query(r#"{app="gw"} != ip("192.168.0.0/16")"#).unwrap() {
            Query::Log(lq) => lq,
            _ => panic!("expected log query"),
        };
        let data = eval.eval_log_query("tenant", &qn, 0, t + 10, 100, Direction::Forward);
        let QueryData::Streams(streams) = data else {
            panic!("expected streams");
        };
        assert_eq!(streams[0].values.len(), 1);
        assert!(streams[0].values[0].1.contains("10.0.0.9"));
    }

    #[test]
    fn ip_label_filter_matches_on_label_value() {
        use super::super::parser::Parser;
        let store = LogStore::new();
        let t = 1_000_000_000i64;
        store
            .push(
                "tenant",
                make_labels(&[("app", "gw"), ("addr", "192.168.4.5")]),
                vec![LogEntry::new(t, "inside")],
            )
            .unwrap();
        store
            .push(
                "tenant",
                make_labels(&[("app", "gw"), ("addr", "10.0.0.9")]),
                vec![LogEntry::new(t + 1, "outside")],
            )
            .unwrap();
        let eval = Evaluator::new(store);

        let q = match Parser::parse_query(r#"{app="gw"} | addr = ip("192.168.0.0/16")"#).unwrap() {
            Query::Log(lq) => lq,
            _ => panic!("expected log query"),
        };
        let data = eval.eval_log_query("tenant", &q, 0, t + 10, 100, Direction::Forward);
        let QueryData::Streams(streams) = data else {
            panic!("expected streams");
        };
        // Only the 192.168.4.5 stream survives.
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].values[0].1, "inside");

        // `!=` selects the complement.
        let qn = match Parser::parse_query(r#"{app="gw"} | addr != ip("192.168.0.0/16")"#).unwrap()
        {
            Query::Log(lq) => lq,
            _ => panic!("expected log query"),
        };
        let data = eval.eval_log_query("tenant", &qn, 0, t + 10, 100, Direction::Forward);
        let QueryData::Streams(streams) = data else {
            panic!("expected streams");
        };
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].values[0].1, "outside");
    }

    fn metric_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn label_replace_sets_dst_from_capture_group() {
        let lr = LabelReplace {
            inner: Box::new(MetricQuery::Literal(0.0)),
            dst_label: "env".into(),
            replacement: "$1".into(),
            src_label: "app".into(),
            regex: "(.*)-prod".into(),
        };
        let input = QueryData::Vector(vec![
            VectorResult {
                metric: metric_map(&[("app", "api-prod")]),
                value: (1.0, "5".into()),
            },
            VectorResult {
                metric: metric_map(&[("app", "other")]),
                value: (1.0, "3".into()),
            },
        ]);
        let QueryData::Vector(out) = apply_label_replace(input, &lr) else {
            panic!("expected vector");
        };
        // First series matched → env="api"; second didn't match → unchanged.
        assert_eq!(out[0].metric.get("env").map(String::as_str), Some("api"));
        assert!(out[1].metric.get("env").is_none());
    }

    #[test]
    fn label_replace_empty_replacement_removes_label() {
        let lr = LabelReplace {
            inner: Box::new(MetricQuery::Literal(0.0)),
            dst_label: "drop_me".into(),
            replacement: "".into(),
            src_label: "app".into(),
            regex: "(.*)".into(),
        };
        let input = QueryData::Vector(vec![VectorResult {
            metric: metric_map(&[("app", "x"), ("drop_me", "stale")]),
            value: (1.0, "1".into()),
        }]);
        let QueryData::Vector(out) = apply_label_replace(input, &lr) else {
            panic!("expected vector");
        };
        assert!(out[0].metric.get("drop_me").is_none());
    }

    #[test]
    fn label_replace_anchors_full_value() {
        // Prometheus anchors the regex: it must match the WHOLE src value.
        let lr = LabelReplace {
            inner: Box::new(MetricQuery::Literal(0.0)),
            dst_label: "out".into(),
            replacement: "hit".into(),
            src_label: "app".into(),
            regex: "prod".into(),
        };
        let input = QueryData::Vector(vec![VectorResult {
            metric: metric_map(&[("app", "myprodsvc")]),
            value: (1.0, "1".into()),
        }]);
        let QueryData::Vector(out) = apply_label_replace(input, &lr) else {
            panic!("expected vector");
        };
        // "prod" does not match the whole "myprodsvc" → unchanged.
        assert!(out[0].metric.get("out").is_none());
    }
}
