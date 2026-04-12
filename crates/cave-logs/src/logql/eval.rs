//! LogQL evaluator — runs an AST against the LogStore.

use super::ast::*;
use crate::models::{LabelMatcher, Labels, LogEntry, MatrixResult, StreamResult};
use crate::store::LogStore;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// An entry after passing through the log pipeline.
#[derive(Debug, Clone)]
pub struct ProcessedEntry {
    pub timestamp: DateTime<Utc>,
    pub line: String,
    /// Labels extracted by parser stages, merged with stream labels.
    pub extracted: HashMap<String, String>,
}

pub struct Evaluator<'a> {
    store: &'a LogStore,
}

impl<'a> Evaluator<'a> {
    pub fn new(store: &'a LogStore) -> Self {
        Self { store }
    }

    // ─── Public entry points ──────────────────────────────────────────────────

    /// Evaluate a log stream expression, returning matching stream results.
    pub fn eval_log(
        &self,
        expr: &LogStreamExpr,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: usize,
        tenant: Option<&str>,
    ) -> Vec<StreamResult> {
        let raw = self
            .store
            .query_streams(&expr.matchers, start, end, limit, true, tenant);

        raw.into_iter()
            .filter_map(|(labels, entries)| {
                let processed = self.apply_pipeline(entries, &expr.pipeline, &labels);
                if processed.is_empty() {
                    return None;
                }
                let values = processed
                    .into_iter()
                    .map(|e| {
                        let ts_ns = e.timestamp.timestamp_nanos_opt().unwrap_or(0).to_string();
                        [ts_ns, e.line]
                    })
                    .collect();
                Some(StreamResult {
                    stream: labels.0.into_iter().collect(),
                    values,
                })
            })
            .collect()
    }

    /// Evaluate a metric expression over a range, returning matrix results.
    pub fn eval_metric(
        &self,
        expr: &MetricExpr,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        step_secs: i64,
        tenant: Option<&str>,
    ) -> Vec<MatrixResult> {
        match expr {
            MetricExpr::RangeAgg(ra) => self.eval_range_agg(ra, start, end, step_secs, tenant),
            MetricExpr::VectorAgg(va) => self.eval_vector_agg(va, start, end, step_secs, tenant),
        }
    }

    /// Evaluate an expression down to a single scalar (for alerting).
    pub fn eval_scalar(
        &self,
        expr: &Expr,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        tenant: Option<&str>,
    ) -> Option<f64> {
        match expr {
            Expr::Log(ls) => {
                let results = self.eval_log(ls, start, end, usize::MAX, tenant);
                let count: usize = results.iter().map(|r| r.values.len()).sum();
                Some(count as f64)
            }
            Expr::Metric(m) => {
                let results = self.eval_metric(m, start, end, (end - start).num_seconds().max(1), tenant);
                let last_val = results
                    .iter()
                    .flat_map(|r| r.values.iter())
                    .last()
                    .and_then(|(_, v)| v.parse::<f64>().ok());
                last_val
            }
        }
    }

    // ─── Range aggregations ───────────────────────────────────────────────────

    fn eval_range_agg(
        &self,
        ra: &RangeAggExpr,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        step_secs: i64,
        tenant: Option<&str>,
    ) -> Vec<MatrixResult> {
        let range_secs = ra.range.as_secs() as i64;
        let buckets = match &ra.op {
            RangeAggOp::Rate => self.store.rate_over_buckets(
                &ra.stream.matchers,
                start, end, range_secs, step_secs, tenant,
            ),
            RangeAggOp::CountOverTime => self.store.count_over_buckets(
                &ra.stream.matchers,
                start, end, range_secs, step_secs, tenant,
            ),
            RangeAggOp::BytesOverTime => self.store.bytes_over_buckets(
                &ra.stream.matchers,
                start, end, range_secs, step_secs, tenant,
            ),
            RangeAggOp::BytesRate => {
                self.store.bytes_over_buckets(
                    &ra.stream.matchers,
                    start, end, range_secs, step_secs, tenant,
                )
                .into_iter()
                .map(|(t, b)| (t, b / range_secs as f64))
                .collect()
            }
            // For label-based agg ops we treat them as count for now
            _ => self.store.count_over_buckets(
                &ra.stream.matchers,
                start, end, range_secs, step_secs, tenant,
            ),
        };

        if buckets.is_empty() {
            return vec![];
        }

        // Collect the label set of matching streams for the metric labels
        let series = self.store.series(&ra.stream.matchers, start, end, tenant);
        let metric_labels = if series.is_empty() {
            HashMap::new()
        } else {
            apply_grouping(&series[0].0, ra.grouping.as_ref())
        };

        let values = buckets
            .into_iter()
            .map(|(t, v)| (t.timestamp() as f64, format!("{v:.6}")))
            .collect();

        vec![MatrixResult { metric: metric_labels, values }]
    }

    // ─── Vector aggregations ──────────────────────────────────────────────────

    fn eval_vector_agg(
        &self,
        va: &VectorAggExpr,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        step_secs: i64,
        tenant: Option<&str>,
    ) -> Vec<MatrixResult> {
        let inner = self.eval_metric(&va.expr, start, end, step_secs, tenant);

        // Collect all sample values grouped by step bucket
        let mut buckets: HashMap<i64, Vec<f64>> = HashMap::new();
        for mr in &inner {
            for (ts, v) in &mr.values {
                if let Ok(f) = v.parse::<f64>() {
                    buckets.entry(*ts as i64).or_default().push(f);
                }
            }
        }

        let k = va.param.unwrap_or(5) as usize;
        let mut step_times: Vec<i64> = buckets.keys().copied().collect();
        step_times.sort();

        let values: Vec<(f64, String)> = step_times
            .into_iter()
            .map(|ts| {
                let samples = buckets.get(&ts).cloned().unwrap_or_default();
                let agg = match va.op {
                    VectorAggOp::Sum => samples.iter().sum(),
                    VectorAggOp::Avg => {
                        if samples.is_empty() { 0.0 } else { samples.iter().sum::<f64>() / samples.len() as f64 }
                    }
                    VectorAggOp::Max => samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    VectorAggOp::Min => samples.iter().cloned().fold(f64::INFINITY, f64::min),
                    VectorAggOp::Count => samples.len() as f64,
                    VectorAggOp::Topk => {
                        let mut s = samples.clone();
                        s.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                        s.into_iter().take(k).sum()
                    }
                    VectorAggOp::Bottomk => {
                        let mut s = samples.clone();
                        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        s.into_iter().take(k).sum()
                    }
                };
                (ts as f64, format!("{agg:.6}"))
            })
            .collect();

        let metric = HashMap::new();
        vec![MatrixResult { metric, values }]
    }

    // ─── Pipeline execution ───────────────────────────────────────────────────

    fn apply_pipeline(
        &self,
        entries: Vec<LogEntry>,
        pipeline: &[PipelineStage],
        stream_labels: &Labels,
    ) -> Vec<ProcessedEntry> {
        let mut result: Vec<ProcessedEntry> = entries
            .into_iter()
            .map(|e| ProcessedEntry {
                timestamp: e.timestamp,
                line: e.line,
                extracted: e.structured_metadata,
            })
            .collect();

        for stage in pipeline {
            result = match stage {
                PipelineStage::Filter(f) => {
                    result.into_iter().filter(|e| self.apply_filter(e, f)).collect()
                }
                PipelineStage::Parser(p) => result
                    .into_iter()
                    .map(|mut e| { self.apply_parser(&mut e, p); e })
                    .collect(),
                PipelineStage::LabelFilter(lf) => result
                    .into_iter()
                    .filter(|e| self.apply_label_filter(e, lf, stream_labels))
                    .collect(),
                PipelineStage::LineFormat(tmpl) => result
                    .into_iter()
                    .map(|mut e| { self.apply_line_format(&mut e, tmpl, stream_labels); e })
                    .collect(),
                PipelineStage::LabelFmt(pairs) => result
                    .into_iter()
                    .map(|mut e| {
                        for (from, to) in pairs {
                            if let Some(v) = e.extracted.remove(from) {
                                e.extracted.insert(to.clone(), v);
                            }
                        }
                        e
                    })
                    .collect(),
                PipelineStage::Decolorize => result
                    .into_iter()
                    .map(|mut e| {
                        e.line = strip_ansi(&e.line);
                        e
                    })
                    .collect(),
            };
        }
        result
    }

    // ── Filter stage ──────────────────────────────────────────────────────────

    fn apply_filter(&self, entry: &ProcessedEntry, stage: &FilterStage) -> bool {
        match stage.op {
            FilterOp::Contains => entry.line.contains(&stage.value),
            FilterOp::NotContains => !entry.line.contains(&stage.value),
            FilterOp::Re => regex::Regex::new(&stage.value)
                .map(|re| re.is_match(&entry.line))
                .unwrap_or(false),
            FilterOp::NotRe => !regex::Regex::new(&stage.value)
                .map(|re| re.is_match(&entry.line))
                .unwrap_or(false),
        }
    }

    // ── Parser stage ──────────────────────────────────────────────────────────

    fn apply_parser(&self, entry: &mut ProcessedEntry, stage: &ParserStage) {
        match stage {
            ParserStage::Json => parse_json(entry),
            ParserStage::Logfmt => parse_logfmt(entry),
            ParserStage::Regexp(re) => parse_regexp(entry, re),
            ParserStage::Pattern(pat) => parse_pattern(entry, pat),
            ParserStage::Unpack => parse_unpack(entry),
        }
    }

    // ── Label filter stage ────────────────────────────────────────────────────

    fn apply_label_filter(
        &self,
        entry: &ProcessedEntry,
        stage: &LabelFilterStage,
        stream_labels: &Labels,
    ) -> bool {
        // Look up in extracted fields first, then stream labels
        let val = entry
            .extracted
            .get(&stage.label)
            .or_else(|| stream_labels.0.get(&stage.label))
            .map(|s| s.as_str());

        match &stage.op {
            LabelFilterOp::Re => {
                let s = val.unwrap_or("");
                if let LabelFilterValue::String(pat) = &stage.value {
                    regex::Regex::new(pat).map(|re| re.is_match(s)).unwrap_or(false)
                } else {
                    false
                }
            }
            LabelFilterOp::NRe => {
                let s = val.unwrap_or("");
                if let LabelFilterValue::String(pat) = &stage.value {
                    !regex::Regex::new(pat).map(|re| re.is_match(s)).unwrap_or(false)
                } else {
                    true
                }
            }
            LabelFilterOp::Eq => match &stage.value {
                LabelFilterValue::String(s) => val == Some(s.as_str()),
                LabelFilterValue::Float(f) => {
                    val.and_then(|v| v.parse::<f64>().ok()).map(|n| (n - f).abs() < f64::EPSILON).unwrap_or(false)
                }
                _ => false,
            },
            LabelFilterOp::Ne => match &stage.value {
                LabelFilterValue::String(s) => val != Some(s.as_str()),
                LabelFilterValue::Float(f) => {
                    val.and_then(|v| v.parse::<f64>().ok()).map(|n| (n - f).abs() >= f64::EPSILON).unwrap_or(true)
                }
                _ => true,
            },
            op => {
                // Numeric comparison
                let num = val.and_then(|v| v.parse::<f64>().ok());
                let threshold = match &stage.value {
                    LabelFilterValue::Float(f) => Some(*f),
                    LabelFilterValue::Duration(d) => Some(d.as_secs_f64()),
                    _ => None,
                };
                match (num, threshold) {
                    (Some(n), Some(t)) => match op {
                        LabelFilterOp::Gt => n > t,
                        LabelFilterOp::Gte => n >= t,
                        LabelFilterOp::Lt => n < t,
                        LabelFilterOp::Lte => n <= t,
                        _ => false,
                    },
                    _ => false,
                }
            }
        }
    }

    // ── Line format stage ─────────────────────────────────────────────────────

    fn apply_line_format(
        &self,
        entry: &mut ProcessedEntry,
        template: &str,
        stream_labels: &Labels,
    ) {
        // Simple Go-template-style replacement: {{.field}} → field value
        let mut result = template.to_string();
        // Merge stream labels + extracted for lookup
        let mut all: HashMap<&str, &str> = HashMap::new();
        for (k, v) in &stream_labels.0 {
            all.insert(k.as_str(), v.as_str());
        }
        for (k, v) in &entry.extracted {
            all.insert(k.as_str(), v.as_str());
        }
        for (k, v) in &all {
            result = result.replace(&format!("{{{{.{k}}}}}"), v);
        }
        entry.line = result;
    }
}

// ─── Parser implementations ───────────────────────────────────────────────────

fn parse_json(entry: &mut ProcessedEntry) {
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&entry.line) {
        for (k, v) in map {
            let s = match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            entry.extracted.insert(k, s);
        }
    }
}

/// Public re-export for integration tests.
pub fn parse_logfmt_pub(entry: &mut ProcessedEntry) {
    parse_logfmt(entry);
}

fn parse_logfmt(entry: &mut ProcessedEntry) {
    // key=value or key="quoted value" or bare_key
    let mut rest = entry.line.as_str();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        // Find key
        let eq_pos = rest.find('=');
        let sp_pos = rest.find(' ');
        match (eq_pos, sp_pos) {
            (Some(e), sp) if sp.map(|s| e < s).unwrap_or(true) => {
                let key = &rest[..e];
                rest = &rest[e + 1..];
                let (val, remaining) = if rest.starts_with('"') {
                    let end = rest[1..].find('"').map(|i| i + 1).unwrap_or(rest.len() - 1);
                    (&rest[1..end], &rest[end + 1..])
                } else {
                    match rest.find(' ') {
                        Some(sp2) => (&rest[..sp2], &rest[sp2..]),
                        None => (rest, ""),
                    }
                };
                entry.extracted.insert(key.to_string(), val.to_string());
                rest = remaining;
            }
            (_, Some(sp)) => {
                // bare key (boolean)
                let key = &rest[..sp];
                entry.extracted.insert(key.to_string(), "true".into());
                rest = &rest[sp..];
            }
            _ => {
                // last bare key
                entry.extracted.insert(rest.trim().to_string(), "true".into());
                break;
            }
        }
    }
}

fn parse_regexp(entry: &mut ProcessedEntry, pattern: &str) {
    if let Ok(re) = regex::Regex::new(pattern) {
        if let Some(caps) = re.captures(&entry.line) {
            for name in re.capture_names().flatten() {
                if let Some(m) = caps.name(name) {
                    entry.extracted.insert(name.to_string(), m.as_str().to_string());
                }
            }
        }
    }
}

fn parse_pattern(entry: &mut ProcessedEntry, pattern: &str) {
    // Pattern syntax: `<ip> - <user> [<_>]` where <name> captures and <_> discards.
    // Build a regex from the pattern.
    let mut regex_str = String::from("^");
    let mut in_bracket = false;
    let mut capture_name = String::new();

    for ch in pattern.chars() {
        match ch {
            '<' => {
                in_bracket = true;
                capture_name.clear();
            }
            '>' if in_bracket => {
                in_bracket = false;
                if capture_name == "_" {
                    regex_str.push_str(r"[^\s]+");
                } else {
                    regex_str.push_str(&format!(r"(?P<{}>[\S]+)", capture_name));
                }
            }
            c if in_bracket => capture_name.push(c),
            c => {
                // Escape regex special chars
                for rc in regex::escape(&c.to_string()).chars() {
                    regex_str.push(rc);
                }
            }
        }
    }
    regex_str.push('$');
    parse_regexp_with_pattern(entry, &regex_str);
}

fn parse_regexp_with_pattern(entry: &mut ProcessedEntry, pattern: &str) {
    parse_regexp(entry, pattern);
}

fn parse_unpack(entry: &mut ProcessedEntry) {
    // Loki's unpack: expects JSON with a "_entry" field for the original log line
    // and other fields as labels.
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&entry.line) {
        for (k, v) in &map {
            if k == "_entry" {
                if let serde_json::Value::String(s) = v {
                    entry.line = s.clone();
                }
            } else {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                entry.extracted.insert(k.clone(), s);
            }
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn strip_ansi(s: &str) -> String {
    // Remove ANSI escape sequences: ESC [ ... m
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").into_owned()
}

fn apply_grouping(labels: &HashMap<String, String>, grouping: Option<&Grouping>) -> HashMap<String, String> {
    match grouping {
        None => labels.clone(),
        Some(g) if g.by => labels
            .iter()
            .filter(|(k, _)| g.labels.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        Some(g) => labels
            .iter()
            .filter(|(k, _)| !g.labels.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logql::parser::parse;
    use crate::models::LogEntry;
    use crate::store::LogStore;
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn make_store() -> LogStore {
        LogStore::new(Duration::days(7))
    }

    fn push_entry(store: &LogStore, labels: &[(&str, &str)], line: &str, offset_secs: i64) {
        let label_map = labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        store.push(
            Labels::new(label_map),
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(60 - offset_secs),
                line: line.to_string(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );
    }

    #[test]
    fn eval_stream_filter_contains() {
        let store = make_store();
        push_entry(&store, &[("app", "web")], "ERROR: connection refused", 0);
        push_entry(&store, &[("app", "web")], "INFO: started successfully", 1);

        let expr = parse(r#"{app="web"} |= "ERROR""#).unwrap();
        let crate::logql::ast::Expr::Log(ls) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_log(&ls, now - Duration::hours(1), now, 100, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].values.len(), 1);
        assert!(results[0].values[0][1].contains("ERROR"));
    }

    #[test]
    fn eval_stream_filter_not_contains() {
        let store = make_store();
        push_entry(&store, &[("app", "api")], "ERROR timeout", 0);
        push_entry(&store, &[("app", "api")], "INFO ok", 1);

        let expr = parse(r#"{app="api"} != "ERROR""#).unwrap();
        let crate::logql::ast::Expr::Log(ls) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_log(&ls, now - Duration::hours(1), now, 100, None);
        assert_eq!(results[0].values.len(), 1);
        assert!(results[0].values[0][1].contains("INFO"));
    }

    #[test]
    fn eval_json_parser() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "svc".into())].into());
        store.push(
            labels,
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(10),
                line: r#"{"level":"error","msg":"oops","status":500}"#.to_string(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        let expr = parse(r#"{app="svc"} | json | status >= 400"#).unwrap();
        let crate::logql::ast::Expr::Log(ls) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_log(&ls, now - Duration::hours(1), now, 100, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].values.len(), 1);
    }

    #[test]
    fn eval_logfmt_parser() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "svc".into())].into());
        store.push(
            labels,
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(5),
                line: r#"level=error method=GET status=500 path="/api""#.to_string(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        let expr = parse(r#"{app="svc"} | logfmt | level = "error""#).unwrap();
        let crate::logql::ast::Expr::Log(ls) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_log(&ls, now - Duration::hours(1), now, 100, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn eval_regexp_parser() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "nginx".into())].into());
        store.push(
            labels,
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(5),
                line: "GET /api/users 200 1234".to_string(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        let expr = parse(
            r#"{app="nginx"} | regexp "(?P<method>[A-Z]+) (?P<path>/[^ ]*) (?P<status>[0-9]+)""#,
        ).unwrap();
        let crate::logql::ast::Expr::Log(ls) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_log(&ls, now - Duration::hours(1), now, 100, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn eval_line_format() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "x".into())].into());
        store.push(
            labels,
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(5),
                line: r#"{"status":"200","method":"GET"}"#.to_string(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        let expr = parse(r#"{app="x"} | json | line_format "{{.method}} {{.status}}""#).unwrap();
        let crate::logql::ast::Expr::Log(ls) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_log(&ls, now - Duration::hours(1), now, 100, None);
        assert!(!results.is_empty());
        assert!(results[0].values[0][1].contains("GET"));
    }

    #[test]
    fn eval_count_over_time() {
        let store = make_store();
        for i in 0..5 {
            push_entry(&store, &[("job", "app")], &format!("log {i}"), i * 10);
        }
        let expr = parse(r#"count_over_time({job="app"}[1h])"#).unwrap();
        let crate::logql::ast::Expr::Metric(m) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_metric(&m, now - Duration::hours(1), now, 3600, None);
        assert!(!results.is_empty());
        let total: f64 = results[0]
            .values
            .iter()
            .flat_map(|(_, v)| v.parse::<f64>())
            .sum();
        assert!(total >= 5.0);
    }

    #[test]
    fn eval_rate() {
        let store = make_store();
        for i in 0..10 {
            push_entry(&store, &[("job", "rate-test")], "msg", i * 5);
        }
        let expr = parse(r#"rate({job="rate-test"}[5m])"#).unwrap();
        let crate::logql::ast::Expr::Metric(m) = expr else { panic!() };
        let eval = Evaluator::new(&store);
        let now = Utc::now();
        let results = eval.eval_metric(&m, now - Duration::minutes(5), now, 300, None);
        assert!(!results.is_empty());
    }
}
