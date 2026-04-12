//! PromQL evaluation engine.

#![allow(dead_code)]

use std::collections::HashMap;
use crate::error::{MetricsError, MetricsResult};
use crate::model::{Labels, LabelMatcher, Sample};
use crate::tsdb::Tsdb;
use super::ast::*;
use super::functions;

#[derive(Debug, Clone)]
pub struct EvalContext {
    pub timestamp_ms: i64,
    pub lookback_ms: i64,
    pub step_ms: i64,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl EvalContext {
    pub fn instant(ts_ms: i64) -> Self {
        Self {
            timestamp_ms: ts_ms,
            lookback_ms: 5 * 60 * 1000, // 5 min default lookback
            step_ms: 0,
            start_ms: ts_ms,
            end_ms: ts_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstantSample {
    pub labels: Labels,
    pub value: f64,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub struct RangeSamples {
    pub labels: Labels,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone)]
pub enum QueryValue {
    InstantVector(Vec<InstantSample>),
    RangeVector(Vec<RangeSamples>),
    Scalar(f64),
    Str(String),
}

pub struct Engine;

impl Engine {
    pub fn new() -> Self {
        Engine
    }

    pub fn eval_instant(
        &self,
        expr: &Expr,
        ctx: &EvalContext,
        tsdb: &Tsdb,
    ) -> MetricsResult<QueryValue> {
        match expr {
            Expr::NumberLiteral(n) => Ok(QueryValue::Scalar(*n)),
            Expr::StringLiteral(s) => Ok(QueryValue::Str(s.clone())),
            Expr::Paren(inner) => self.eval_instant(inner, ctx, tsdb),
            Expr::Unary { op, expr } => {
                let val = self.eval_instant(expr, ctx, tsdb)?;
                match (op, val) {
                    (UnaryOp::Neg, QueryValue::Scalar(n)) => Ok(QueryValue::Scalar(-n)),
                    (UnaryOp::Pos, QueryValue::Scalar(n)) => Ok(QueryValue::Scalar(n)),
                    (UnaryOp::Neg, QueryValue::InstantVector(v)) => {
                        Ok(QueryValue::InstantVector(
                            v.into_iter().map(|mut s| { s.value = -s.value; s }).collect()
                        ))
                    }
                    (UnaryOp::Pos, QueryValue::InstantVector(v)) => Ok(QueryValue::InstantVector(v)),
                    _ => Err(MetricsError::Eval("type error in unary".to_string())),
                }
            }
            Expr::VectorSelector { matchers, offset, at, .. } => {
                let ts = at.unwrap_or(ctx.timestamp_ms) - offset.unwrap_or(0);
                let start = ts - ctx.lookback_ms;
                let pairs = tsdb.select_at(matchers, ts);
                // For instant vector, filter to lookback window
                let samples: Vec<InstantSample> = pairs
                    .into_iter()
                    .filter(|(_, s)| s.timestamp >= start)
                    .map(|(labels, s)| InstantSample {
                        labels,
                        value: s.value,
                        timestamp: s.timestamp,
                    })
                    .collect();
                Ok(QueryValue::InstantVector(samples))
            }
            Expr::MatrixSelector { selector, range_ms } => {
                if let Expr::VectorSelector { matchers, offset, at, .. } = selector.as_ref() {
                    let ts = at.unwrap_or(ctx.timestamp_ms) - offset.unwrap_or(0);
                    let start = ts - range_ms;
                    let series = tsdb.select(matchers, start, ts);
                    let range_vec: Vec<RangeSamples> = series
                        .into_iter()
                        .map(|ts_data| RangeSamples {
                            labels: ts_data.labels,
                            samples: ts_data.samples,
                        })
                        .collect();
                    Ok(QueryValue::RangeVector(range_vec))
                } else {
                    Err(MetricsError::Eval("MatrixSelector inner must be VectorSelector".to_string()))
                }
            }
            Expr::Call { func, args } => {
                self.eval_call(func, args, ctx, tsdb)
            }
            Expr::Aggregate { op, expr, param, grouping } => {
                self.eval_aggregate(*op, expr, param.as_deref(), grouping, ctx, tsdb)
            }
            Expr::Binary { op, lhs, rhs, matching, return_bool } => {
                self.eval_binary(*op, lhs, rhs, matching, *return_bool, ctx, tsdb)
            }
            Expr::Subquery { expr, range_ms, step_ms } => {
                // Evaluate the inner expression as a range vector
                let n_steps = (range_ms / step_ms).max(1) as usize;
                let mut all: HashMap<u64, RangeSamples> = HashMap::new();
                for i in 0..=n_steps {
                    let ts = ctx.timestamp_ms - range_ms + i as i64 * step_ms;
                    let mut sub_ctx = ctx.clone();
                    sub_ctx.timestamp_ms = ts;
                    match self.eval_instant(expr, &sub_ctx, tsdb)? {
                        QueryValue::InstantVector(v) => {
                            for s in v {
                                let fp = s.labels.fingerprint();
                                let entry = all.entry(fp).or_insert_with(|| RangeSamples {
                                    labels: s.labels.clone(),
                                    samples: vec![],
                                });
                                entry.samples.push(Sample { timestamp: ts, value: s.value });
                            }
                        }
                        _ => {}
                    }
                }
                Ok(QueryValue::RangeVector(all.into_values().collect()))
            }
        }
    }

    pub fn eval_range(
        &self,
        expr: &Expr,
        ctx: &EvalContext,
        tsdb: &Tsdb,
    ) -> MetricsResult<Vec<(i64, QueryValue)>> {
        let mut results = Vec::new();
        let step = ctx.step_ms.max(1);
        let mut ts = ctx.start_ms;
        while ts <= ctx.end_ms {
            let mut step_ctx = ctx.clone();
            step_ctx.timestamp_ms = ts;
            let val = self.eval_instant(expr, &step_ctx, tsdb)?;
            results.push((ts, val));
            ts += step;
        }
        Ok(results)
    }

    fn eval_call(
        &self,
        func: &str,
        args: &[Expr],
        ctx: &EvalContext,
        tsdb: &Tsdb,
    ) -> MetricsResult<QueryValue> {
        match func {
            "rate" | "irate" | "increase" | "delta" | "deriv" | "changes" => {
                let range_vec = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::RangeVector(rv) => rv,
                    _ => return Err(MetricsError::Eval(format!("{} requires range vector", func))),
                };
                let result: Vec<InstantSample> = range_vec
                    .into_iter()
                    .filter_map(|rs| {
                        let value = match func {
                            "rate" => {
                                let range_ms = if let Expr::MatrixSelector { range_ms, .. } = &args[0] {
                                    *range_ms
                                } else { ctx.lookback_ms };
                                functions::rate(&rs.samples, range_ms)?
                            }
                            "irate" => functions::irate(&rs.samples)?,
                            "increase" => {
                                let range_ms = if let Expr::MatrixSelector { range_ms, .. } = &args[0] {
                                    *range_ms
                                } else { ctx.lookback_ms };
                                functions::increase(&rs.samples, range_ms)?
                            }
                            "delta" => functions::delta(&rs.samples)?,
                            "deriv" => functions::deriv(&rs.samples)?,
                            "changes" => functions::changes(&rs.samples),
                            _ => unreachable!(),
                        };
                        Some(InstantSample {
                            labels: rs.labels,
                            value,
                            timestamp: ctx.timestamp_ms,
                        })
                    })
                    .collect();
                Ok(QueryValue::InstantVector(result))
            }
            "predict_linear" => {
                let range_vec = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::RangeVector(rv) => rv,
                    _ => return Err(MetricsError::Eval("predict_linear requires range vector".to_string())),
                };
                let duration_s = match self.eval_instant(&args[1], ctx, tsdb)? {
                    QueryValue::Scalar(n) => n,
                    _ => return Err(MetricsError::Eval("predict_linear second arg must be scalar".to_string())),
                };
                let result: Vec<InstantSample> = range_vec
                    .into_iter()
                    .filter_map(|rs| {
                        let value = functions::predict_linear(&rs.samples, duration_s)?;
                        Some(InstantSample { labels: rs.labels, value, timestamp: ctx.timestamp_ms })
                    })
                    .collect();
                Ok(QueryValue::InstantVector(result))
            }
            "histogram_quantile" => {
                let q = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::Scalar(n) => n,
                    _ => return Err(MetricsError::Eval("histogram_quantile first arg must be scalar".to_string())),
                };
                let range_vec = match self.eval_instant(&args[1], ctx, tsdb)? {
                    QueryValue::RangeVector(rv) => rv,
                    QueryValue::InstantVector(iv) => {
                        // Convert instant vector: group by metric (without le), build buckets
                        return self.eval_histogram_quantile_instant(q, iv, ctx);
                    }
                    _ => return Err(MetricsError::Eval("histogram_quantile second arg must be vector".to_string())),
                };
                // Group by metric name minus "le" label
                let mut groups: HashMap<u64, Vec<(f64, f64)>> = HashMap::new();
                let mut group_labels: HashMap<u64, Labels> = HashMap::new();
                for rs in range_vec {
                    if let Some(le_str) = rs.labels.get("le") {
                        if let Ok(le) = le_str.parse::<f64>() {
                            let mut group_lbls = rs.labels.0.clone();
                            group_lbls.remove("le");
                            let group_labels_obj = Labels(group_lbls);
                            let fp = group_labels_obj.fingerprint();
                            group_labels.entry(fp).or_insert(group_labels_obj);
                            if let Some(last) = rs.samples.last() {
                                groups.entry(fp).or_default().push((le, last.value));
                            }
                        }
                    }
                }
                let result: Vec<InstantSample> = groups
                    .into_iter()
                    .map(|(fp, buckets)| {
                        let value = functions::histogram_quantile(q, &buckets);
                        InstantSample {
                            labels: group_labels[&fp].clone(),
                            value,
                            timestamp: ctx.timestamp_ms,
                        }
                    })
                    .collect();
                Ok(QueryValue::InstantVector(result))
            }
            "label_replace" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("label_replace first arg must be vector".to_string())),
                };
                let dst = match &args[1] { Expr::StringLiteral(s) => s.clone(), _ => return Err(MetricsError::Eval("expected string".to_string())) };
                let replacement = match &args[2] { Expr::StringLiteral(s) => s.clone(), _ => return Err(MetricsError::Eval("expected string".to_string())) };
                let src = match &args[3] { Expr::StringLiteral(s) => s.clone(), _ => return Err(MetricsError::Eval("expected string".to_string())) };
                let regex = match &args[4] { Expr::StringLiteral(s) => s.clone(), _ => return Err(MetricsError::Eval("expected string".to_string())) };
                let result = iv.into_iter().map(|s| InstantSample {
                    labels: functions::label_replace(&s.labels, &dst, &replacement, &src, &regex),
                    value: s.value,
                    timestamp: s.timestamp,
                }).collect();
                Ok(QueryValue::InstantVector(result))
            }
            "sort" | "sort_asc" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("sort requires vector".to_string())),
                };
                Ok(QueryValue::InstantVector(functions::sort_asc(iv)))
            }
            "sort_desc" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("sort_desc requires vector".to_string())),
                };
                let mut v = iv;
                v.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
                Ok(QueryValue::InstantVector(v))
            }
            "absent" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("absent requires vector".to_string())),
                };
                Ok(QueryValue::InstantVector(functions::absent(&iv)))
            }
            "vector" => {
                let n = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::Scalar(n) => n,
                    _ => return Err(MetricsError::Eval("vector requires scalar".to_string())),
                };
                Ok(QueryValue::InstantVector(vec![InstantSample {
                    labels: Labels::default(),
                    value: n,
                    timestamp: ctx.timestamp_ms,
                }]))
            }
            "scalar" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("scalar requires vector".to_string())),
                };
                if iv.len() == 1 {
                    Ok(QueryValue::Scalar(iv[0].value))
                } else {
                    Ok(QueryValue::Scalar(f64::NAN))
                }
            }
            "time" => Ok(QueryValue::Scalar(ctx.timestamp_ms as f64 / 1000.0)),
            "timestamp" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("timestamp requires vector".to_string())),
                };
                let result = iv.into_iter().map(|s| InstantSample {
                    value: s.timestamp as f64 / 1000.0,
                    ..s
                }).collect();
                Ok(QueryValue::InstantVector(result))
            }
            "floor" => self.apply_math_fn(args, ctx, tsdb, f64::floor),
            "ceil" => self.apply_math_fn(args, ctx, tsdb, f64::ceil),
            "round" => self.apply_math_fn(args, ctx, tsdb, |v| v.round()),
            "abs" => self.apply_math_fn(args, ctx, tsdb, f64::abs),
            "sqrt" => self.apply_math_fn(args, ctx, tsdb, f64::sqrt),
            "exp" => self.apply_math_fn(args, ctx, tsdb, f64::exp),
            "ln" => self.apply_math_fn(args, ctx, tsdb, f64::ln),
            "log2" => self.apply_math_fn(args, ctx, tsdb, f64::log2),
            "log10" => self.apply_math_fn(args, ctx, tsdb, f64::log10),
            "sin" => self.apply_math_fn(args, ctx, tsdb, f64::sin),
            "cos" => self.apply_math_fn(args, ctx, tsdb, f64::cos),
            "tan" => self.apply_math_fn(args, ctx, tsdb, f64::tan),
            "asin" => self.apply_math_fn(args, ctx, tsdb, f64::asin),
            "acos" => self.apply_math_fn(args, ctx, tsdb, f64::acos),
            "atan" => self.apply_math_fn(args, ctx, tsdb, f64::atan),
            "deg" => self.apply_math_fn(args, ctx, tsdb, |v| v.to_degrees()),
            "rad" => self.apply_math_fn(args, ctx, tsdb, |v| v.to_radians()),
            "sgn" => self.apply_math_fn(args, ctx, tsdb, |v| if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 }),
            "clamp" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? {
                    QueryValue::InstantVector(v) => v,
                    _ => return Err(MetricsError::Eval("clamp requires vector".to_string())),
                };
                let min = match self.eval_instant(&args[1], ctx, tsdb)? { QueryValue::Scalar(n) => n, _ => return Err(MetricsError::Eval("expected scalar".to_string())) };
                let max = match self.eval_instant(&args[2], ctx, tsdb)? { QueryValue::Scalar(n) => n, _ => return Err(MetricsError::Eval("expected scalar".to_string())) };
                Ok(QueryValue::InstantVector(iv.into_iter().map(|mut s| { s.value = s.value.clamp(min, max); s }).collect()))
            }
            "clamp_min" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? { QueryValue::InstantVector(v) => v, _ => return Err(MetricsError::Eval("expected vector".to_string())) };
                let min = match self.eval_instant(&args[1], ctx, tsdb)? { QueryValue::Scalar(n) => n, _ => return Err(MetricsError::Eval("expected scalar".to_string())) };
                Ok(QueryValue::InstantVector(iv.into_iter().map(|mut s| { s.value = s.value.max(min); s }).collect()))
            }
            "clamp_max" => {
                let iv = match self.eval_instant(&args[0], ctx, tsdb)? { QueryValue::InstantVector(v) => v, _ => return Err(MetricsError::Eval("expected vector".to_string())) };
                let max = match self.eval_instant(&args[1], ctx, tsdb)? { QueryValue::Scalar(n) => n, _ => return Err(MetricsError::Eval("expected scalar".to_string())) };
                Ok(QueryValue::InstantVector(iv.into_iter().map(|mut s| { s.value = s.value.min(max); s }).collect()))
            }
            _ => Err(MetricsError::Eval(format!("unknown function: {}", func))),
        }
    }

    fn apply_math_fn(
        &self,
        args: &[Expr],
        ctx: &EvalContext,
        tsdb: &Tsdb,
        f: fn(f64) -> f64,
    ) -> MetricsResult<QueryValue> {
        match self.eval_instant(&args[0], ctx, tsdb)? {
            QueryValue::InstantVector(v) => {
                Ok(QueryValue::InstantVector(v.into_iter().map(|mut s| { s.value = f(s.value); s }).collect()))
            }
            QueryValue::Scalar(n) => Ok(QueryValue::Scalar(f(n))),
            _ => Err(MetricsError::Eval("math fn requires vector or scalar".to_string())),
        }
    }

    fn eval_histogram_quantile_instant(
        &self,
        q: f64,
        iv: Vec<InstantSample>,
        ctx: &EvalContext,
    ) -> MetricsResult<QueryValue> {
        // Group by all labels except "le"
        let mut groups: HashMap<u64, Vec<(f64, f64)>> = HashMap::new();
        let mut group_labels_map: HashMap<u64, Labels> = HashMap::new();
        for s in iv {
            if let Some(le_str) = s.labels.get("le") {
                if let Ok(le) = le_str.parse::<f64>() {
                    let mut lbls = s.labels.0.clone();
                    lbls.remove("le");
                    let group = Labels(lbls);
                    let fp = group.fingerprint();
                    group_labels_map.entry(fp).or_insert(group);
                    groups.entry(fp).or_default().push((le, s.value));
                }
            }
        }
        let result: Vec<InstantSample> = groups
            .into_iter()
            .map(|(fp, buckets)| {
                let value = functions::histogram_quantile(q, &buckets);
                InstantSample {
                    labels: group_labels_map[&fp].clone(),
                    value,
                    timestamp: ctx.timestamp_ms,
                }
            })
            .collect();
        Ok(QueryValue::InstantVector(result))
    }

    fn eval_aggregate(
        &self,
        op: AggregateOp,
        expr: &Expr,
        param: Option<&Expr>,
        grouping: &Grouping,
        ctx: &EvalContext,
        tsdb: &Tsdb,
    ) -> MetricsResult<QueryValue> {
        let iv = match self.eval_instant(expr, ctx, tsdb)? {
            QueryValue::InstantVector(v) => v,
            QueryValue::Scalar(n) => vec![InstantSample {
                labels: Labels::default(),
                value: n,
                timestamp: ctx.timestamp_ms,
            }],
            _ => return Err(MetricsError::Eval("aggregate requires instant vector".to_string())),
        };

        // Special cases
        match op {
            AggregateOp::Topk => {
                let k = match param {
                    Some(p) => match self.eval_instant(p, ctx, tsdb)? {
                        QueryValue::Scalar(n) => n as usize,
                        _ => return Err(MetricsError::Eval("topk requires scalar param".to_string())),
                    },
                    None => return Err(MetricsError::Eval("topk requires parameter".to_string())),
                };
                return Ok(QueryValue::InstantVector(functions::topk(k, iv)));
            }
            AggregateOp::Bottomk => {
                let k = match param {
                    Some(p) => match self.eval_instant(p, ctx, tsdb)? {
                        QueryValue::Scalar(n) => n as usize,
                        _ => return Err(MetricsError::Eval("bottomk requires scalar param".to_string())),
                    },
                    None => return Err(MetricsError::Eval("bottomk requires parameter".to_string())),
                };
                return Ok(QueryValue::InstantVector(functions::bottomk(k, iv)));
            }
            AggregateOp::Quantile => {
                let q = match param {
                    Some(p) => match self.eval_instant(p, ctx, tsdb)? {
                        QueryValue::Scalar(n) => n,
                        _ => return Err(MetricsError::Eval("quantile requires scalar param".to_string())),
                    },
                    None => return Err(MetricsError::Eval("quantile requires parameter".to_string())),
                };
                // Group and compute quantile per group
                let groups = group_samples(&iv, grouping);
                let result = groups.into_iter().map(|(labels, samples)| {
                    let mut vals: Vec<f64> = samples.iter().map(|s| s.value).collect();
                    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let idx = (q * vals.len() as f64).floor() as usize;
                    let value = vals.get(idx.min(vals.len().saturating_sub(1))).copied().unwrap_or(f64::NAN);
                    InstantSample { labels, value, timestamp: ctx.timestamp_ms }
                }).collect();
                return Ok(QueryValue::InstantVector(result));
            }
            _ => {}
        }

        let groups = group_samples(&iv, grouping);
        let result: Vec<InstantSample> = groups
            .into_iter()
            .filter_map(|(labels, samples)| {
                let values: Vec<f64> = samples.iter().map(|s| s.value).collect();
                let value = match op {
                    AggregateOp::Sum => Some(values.iter().sum()),
                    AggregateOp::Avg => {
                        if values.is_empty() { None } else {
                            Some(values.iter().sum::<f64>() / values.len() as f64)
                        }
                    }
                    AggregateOp::Min => values.iter().cloned().reduce(f64::min),
                    AggregateOp::Max => values.iter().cloned().reduce(f64::max),
                    AggregateOp::Count => Some(values.len() as f64),
                    AggregateOp::Stddev => {
                        if values.len() < 2 { return None; }
                        let mean = values.iter().sum::<f64>() / values.len() as f64;
                        let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
                        Some(var.sqrt())
                    }
                    AggregateOp::Stdvar => {
                        if values.len() < 2 { return None; }
                        let mean = values.iter().sum::<f64>() / values.len() as f64;
                        Some(values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64)
                    }
                    AggregateOp::CountValues | AggregateOp::Topk | AggregateOp::Bottomk | AggregateOp::Quantile => None,
                }?;
                Some(InstantSample { labels, value, timestamp: ctx.timestamp_ms })
            })
            .collect();
        Ok(QueryValue::InstantVector(result))
    }

    fn eval_binary(
        &self,
        op: BinaryOp,
        lhs_expr: &Expr,
        rhs_expr: &Expr,
        _matching: &VectorMatching,
        return_bool: bool,
        ctx: &EvalContext,
        tsdb: &Tsdb,
    ) -> MetricsResult<QueryValue> {
        let lhs = self.eval_instant(lhs_expr, ctx, tsdb)?;
        let rhs = self.eval_instant(rhs_expr, ctx, tsdb)?;

        match (lhs, rhs) {
            (QueryValue::Scalar(l), QueryValue::Scalar(r)) => {
                let val = apply_binary_op(op, l, r, return_bool);
                Ok(QueryValue::Scalar(val))
            }
            (QueryValue::InstantVector(lv), QueryValue::Scalar(r)) => {
                let result = lv.into_iter().map(|mut s| {
                    s.value = apply_binary_op(op, s.value, r, return_bool);
                    s
                }).collect();
                Ok(QueryValue::InstantVector(result))
            }
            (QueryValue::Scalar(l), QueryValue::InstantVector(rv)) => {
                let result = rv.into_iter().map(|mut s| {
                    s.value = apply_binary_op(op, l, s.value, return_bool);
                    s
                }).collect();
                Ok(QueryValue::InstantVector(result))
            }
            (QueryValue::InstantVector(lv), QueryValue::InstantVector(rv)) => {
                // Set operations
                match op {
                    BinaryOp::And => {
                        let rhs_fps: std::collections::HashSet<u64> = rv.iter().map(|s| s.labels.fingerprint()).collect();
                        let result = lv.into_iter().filter(|s| rhs_fps.contains(&s.labels.fingerprint())).collect();
                        return Ok(QueryValue::InstantVector(result));
                    }
                    BinaryOp::Or => {
                        let lhs_fps: std::collections::HashSet<u64> = lv.iter().map(|s| s.labels.fingerprint()).collect();
                        let mut result = lv;
                        for s in rv {
                            if !lhs_fps.contains(&s.labels.fingerprint()) {
                                result.push(s);
                            }
                        }
                        return Ok(QueryValue::InstantVector(result));
                    }
                    BinaryOp::Unless => {
                        let rhs_fps: std::collections::HashSet<u64> = rv.iter().map(|s| s.labels.fingerprint()).collect();
                        let result = lv.into_iter().filter(|s| !rhs_fps.contains(&s.labels.fingerprint())).collect();
                        return Ok(QueryValue::InstantVector(result));
                    }
                    _ => {}
                }
                // Arithmetic/comparison: match by labels (excluding __name__)
                let rhs_map: HashMap<u64, f64> = rv.iter().map(|s| {
                    let mut lbls = s.labels.0.clone();
                    lbls.remove("__name__");
                    (Labels(lbls).fingerprint(), s.value)
                }).collect();
                let result: Vec<InstantSample> = lv.into_iter().filter_map(|mut s| {
                    let mut lbls = s.labels.0.clone();
                    lbls.remove("__name__");
                    let fp = Labels(lbls).fingerprint();
                    if let Some(&r) = rhs_map.get(&fp) {
                        s.value = apply_binary_op(op, s.value, r, return_bool);
                        // Remove metric name from result
                        s.labels.0.remove("__name__");
                        Some(s)
                    } else {
                        None
                    }
                }).collect();
                Ok(QueryValue::InstantVector(result))
            }
            _ => Err(MetricsError::Eval("unsupported binary operand types".to_string())),
        }
    }
}

fn apply_binary_op(op: BinaryOp, l: f64, r: f64, return_bool: bool) -> f64 {
    match op {
        BinaryOp::Add => l + r,
        BinaryOp::Sub => l - r,
        BinaryOp::Mul => l * r,
        BinaryOp::Div => l / r,
        BinaryOp::Mod => l % r,
        BinaryOp::Pow => l.powf(r),
        BinaryOp::Atan2 => l.atan2(r),
        BinaryOp::Eql => bool_to_val(l == r, return_bool, l),
        BinaryOp::Neq => bool_to_val(l != r, return_bool, l),
        BinaryOp::Lss => bool_to_val(l < r, return_bool, l),
        BinaryOp::Gtr => bool_to_val(l > r, return_bool, l),
        BinaryOp::Lte => bool_to_val(l <= r, return_bool, l),
        BinaryOp::Gte => bool_to_val(l >= r, return_bool, l),
        BinaryOp::And | BinaryOp::Or | BinaryOp::Unless => l,
    }
}

fn bool_to_val(cond: bool, return_bool: bool, original: f64) -> f64 {
    if return_bool {
        if cond { 1.0 } else { 0.0 }
    } else {
        if cond { original } else { f64::NAN }
    }
}

fn group_key(labels: &Labels, grouping: &Grouping) -> Labels {
    if !grouping.specified {
        // No by/without → aggregate everything into a single empty-labels group
        return Labels::default();
    }
    if grouping.by {
        let m: std::collections::BTreeMap<String, String> = labels.0.iter()
            .filter(|(k, _)| grouping.labels.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Labels(m)
    } else {
        let m: std::collections::BTreeMap<String, String> = labels.0.iter()
            .filter(|(k, _)| !grouping.labels.contains(k) && k.as_str() != "__name__")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Labels(m)
    }
}

fn group_samples(iv: &[InstantSample], grouping: &Grouping) -> Vec<(Labels, Vec<InstantSample>)> {
    let mut map: HashMap<u64, (Labels, Vec<InstantSample>)> = HashMap::new();
    for s in iv {
        let key = group_key(&s.labels, grouping);
        let fp = key.fingerprint();
        map.entry(fp)
            .or_insert_with(|| (key, Vec::new()))
            .1.push(s.clone());
    }
    map.into_values().collect()
}

// Match a VectorSelector to get its matchers
fn extract_matchers(expr: &Expr) -> Option<&[LabelMatcher]> {
    match expr {
        Expr::VectorSelector { matchers, .. } => Some(matchers),
        _ => None,
    }
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}
