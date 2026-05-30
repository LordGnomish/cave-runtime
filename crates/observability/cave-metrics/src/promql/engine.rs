// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PromQL evaluation engine.
//! Evaluates a parsed Expr against the TSDB.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{MetricsError, Result};
use crate::model::{Labels, QueryResult, Sample};
use crate::promql::ast::*;
use crate::promql::functions as fns;
use crate::tsdb::Tsdb;

const DEFAULT_LOOKBACK_MS: i64 = 5 * 60 * 1000; // 5 minutes

pub struct Engine {
    tsdb: Arc<Tsdb>,
}

impl Engine {
    pub fn new(tsdb: Arc<Tsdb>) -> Self {
        Self { tsdb }
    }

    /// Evaluate an expression at an instant.
    pub fn eval_instant(&self, expr: &Expr, ts_ms: i64) -> Result<QueryResult> {
        match expr {
            Expr::NumberLiteral(n) => Ok(QueryResult::Scalar(*n)),
            Expr::StringLiteral(s) => Ok(QueryResult::String(s.clone())),

            Expr::VectorSelector(vs) => {
                let eval_ts = vs.at.unwrap_or(ts_ms) - vs.offset.unwrap_or(0);
                let pairs = self
                    .tsdb
                    .select_at(&vs.matchers, eval_ts, DEFAULT_LOOKBACK_MS);
                Ok(QueryResult::InstantVector(
                    pairs
                        .into_iter()
                        .map(|(labels, s)| (labels, s.value))
                        .collect(),
                ))
            }

            Expr::MatrixSelector(ms) => {
                let eval_ts = ms.at.unwrap_or(ts_ms) - ms.offset.unwrap_or(0);
                let start = eval_ts - ms.range_ms;
                let result = self.tsdb.select(&ms.selector.matchers, start, eval_ts);
                Ok(QueryResult::RangeVector(result))
            }

            Expr::Subquery(sq) => {
                // Evaluate the inner expression at each step within the subquery range.
                let eval_ts = sq.at.unwrap_or(ts_ms) - sq.offset.unwrap_or(0);
                let step = if sq.step_ms > 0 { sq.step_ms } else { 60_000 };
                let start = eval_ts - sq.range_ms;
                let mut all: HashMap<u64, (Labels, Vec<Sample>)> = HashMap::new();
                let mut t = start;
                while t <= eval_ts {
                    if let Ok(QueryResult::InstantVector(iv)) = self.eval_instant(&sq.expr, t) {
                        for (labels, val) in iv {
                            let fp = labels.fingerprint();
                            let entry = all.entry(fp).or_insert_with(|| (labels, Vec::new()));
                            entry.1.push(Sample::new(t, val));
                        }
                    }
                    t += step;
                }
                Ok(QueryResult::RangeVector(all.into_values().collect()))
            }

            Expr::Unary(u) => match self.eval_instant(&u.expr, ts_ms)? {
                QueryResult::Scalar(n) => Ok(QueryResult::Scalar(-n)),
                QueryResult::InstantVector(iv) => Ok(QueryResult::InstantVector(
                    iv.into_iter().map(|(l, v)| (l, -v)).collect(),
                )),
                other => Err(MetricsError::Eval(format!(
                    "unary - cannot apply to {:?}",
                    other
                ))),
            },

            Expr::Binary(b) => self.eval_binary(b, ts_ms),

            Expr::Aggregate(agg) => self.eval_aggregate(agg, ts_ms),

            Expr::Call(call) => self.eval_call(call, ts_ms),
        }
    }

    /// Evaluate a range query: returns instant vector at each step.
    pub fn eval_range(
        &self,
        expr: &Expr,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> Result<Vec<(i64, QueryResult)>> {
        let step = step_ms.max(1);
        let mut results = Vec::new();
        let mut ts = start_ms;
        while ts <= end_ms {
            let r = self.eval_instant(expr, ts)?;
            results.push((ts, r));
            ts += step;
        }
        Ok(results)
    }

    fn eval_binary(&self, b: &BinaryExpr, ts_ms: i64) -> Result<QueryResult> {
        let lhs = self.eval_instant(&b.lhs, ts_ms)?;
        let rhs = self.eval_instant(&b.rhs, ts_ms)?;

        // Scalar op scalar
        if let (QueryResult::Scalar(l), QueryResult::Scalar(r)) = (&lhs, &rhs) {
            return Ok(QueryResult::Scalar(b.op.apply(*l, *r)));
        }

        // Scalar op vector or vector op scalar
        if let QueryResult::Scalar(scalar) = &rhs {
            if let QueryResult::InstantVector(iv) = lhs {
                return Ok(QueryResult::InstantVector(
                    iv.into_iter()
                        .map(|(l, v)| {
                            let val = b.op.apply(v, *scalar);
                            if b.op.is_comparison() && !b.return_bool {
                                if val != 0.0 { (l, v) } else { (l, f64::NAN) }
                            } else {
                                (l, val)
                            }
                        })
                        .filter(|(_, v)| !v.is_nan())
                        .collect(),
                ));
            }
        }

        if let QueryResult::Scalar(scalar) = &lhs {
            if let QueryResult::InstantVector(iv) = rhs {
                return Ok(QueryResult::InstantVector(
                    iv.into_iter()
                        .map(|(l, v)| {
                            let val = b.op.apply(*scalar, v);
                            if b.op.is_comparison() && !b.return_bool {
                                if val != 0.0 {
                                    (l, *scalar)
                                } else {
                                    (l, f64::NAN)
                                }
                            } else {
                                (l, val)
                            }
                        })
                        .filter(|(_, v)| !v.is_nan())
                        .collect(),
                ));
            }
        }

        // Vector op vector
        if let (QueryResult::InstantVector(liv), QueryResult::InstantVector(riv)) = (lhs, rhs) {
            return self.binary_vector_op(b, liv, riv);
        }

        Err(MetricsError::Eval(format!(
            "binary op {:?} on incompatible types",
            b.op
        )))
    }

    fn binary_vector_op(
        &self,
        b: &BinaryExpr,
        lhs: Vec<(Labels, f64)>,
        rhs: Vec<(Labels, f64)>,
    ) -> Result<QueryResult> {
        // Set operations
        if b.op.is_set_op() {
            return self.set_op(b.op, lhs, rhs);
        }

        // Build a lookup from the RHS side keyed by matching labels.
        let rhs_map: HashMap<Labels, f64> = rhs
            .into_iter()
            .map(|(labels, val)| {
                let key = matching_key(&labels, b.matching.as_ref());
                (key, val)
            })
            .collect();

        let mut out = Vec::new();
        for (lhs_labels, lhs_val) in lhs {
            let key = matching_key(&lhs_labels, b.matching.as_ref());
            if let Some(&rhs_val) = rhs_map.get(&key) {
                let result_val = b.op.apply(lhs_val, rhs_val);
                let final_val = if b.op.is_comparison() && !b.return_bool {
                    if result_val != 0.0 { lhs_val } else { f64::NAN }
                } else if b.op.is_comparison() && b.return_bool {
                    if result_val != 0.0 { 1.0 } else { 0.0 }
                } else {
                    result_val
                };
                if !final_val.is_nan() {
                    let result_labels = result_labels(&lhs_labels, b.matching.as_ref());
                    out.push((result_labels, final_val));
                }
            }
        }
        Ok(QueryResult::InstantVector(out))
    }

    fn set_op(
        &self,
        op: BinaryOp,
        lhs: Vec<(Labels, f64)>,
        rhs: Vec<(Labels, f64)>,
    ) -> Result<QueryResult> {
        let rhs_set: std::collections::HashSet<Labels> =
            rhs.iter().map(|(l, _)| l.without_name()).collect();
        let out = match op {
            BinaryOp::And => lhs
                .into_iter()
                .filter(|(l, _)| rhs_set.contains(&l.without_name()))
                .collect(),
            BinaryOp::Or => {
                let mut res = lhs;
                let lhs_set: std::collections::HashSet<Labels> =
                    res.iter().map(|(l, _)| l.without_name()).collect();
                for (l, v) in rhs {
                    if !lhs_set.contains(&l.without_name()) {
                        res.push((l, v));
                    }
                }
                res
            }
            BinaryOp::Unless => lhs
                .into_iter()
                .filter(|(l, _)| !rhs_set.contains(&l.without_name()))
                .collect(),
            _ => unreachable!(),
        };
        Ok(QueryResult::InstantVector(out))
    }

    fn eval_aggregate(&self, agg: &AggregateExpr, ts_ms: i64) -> Result<QueryResult> {
        let inner = self.eval_instant(&agg.expr, ts_ms)?;
        let iv = match inner {
            QueryResult::InstantVector(iv) => iv,
            QueryResult::Scalar(v) => vec![(Labels::new(), v)],
            _ => {
                return Err(MetricsError::Eval(
                    "aggregation requires an instant vector".into(),
                ));
            }
        };

        // Group series by the grouping key
        let mut groups: HashMap<Labels, Vec<(Labels, f64)>> = HashMap::new();
        for (labels, val) in iv {
            let key = group_key(&labels, &agg.grouping);
            groups.entry(key).or_default().push((labels, val));
        }

        let mut out = Vec::new();
        for (group_labels, members) in groups {
            let values: Vec<f64> = members.iter().map(|(_, v)| *v).collect();
            let result_val = match agg.op {
                AggregateOp::Sum => values.iter().sum(),
                AggregateOp::Min => values.iter().cloned().fold(f64::INFINITY, f64::min),
                AggregateOp::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                AggregateOp::Avg => values.iter().sum::<f64>() / values.len() as f64,
                AggregateOp::Count => values.len() as f64,
                AggregateOp::Group => 1.0,
                AggregateOp::Stddev => {
                    let mean = values.iter().sum::<f64>() / values.len() as f64;
                    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64)
                        .sqrt()
                }
                AggregateOp::Stdvar => {
                    let mean = values.iter().sum::<f64>() / values.len() as f64;
                    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
                }
                AggregateOp::Quantile => {
                    let q = match agg.param.as_deref() {
                        Some(Expr::NumberLiteral(q)) => *q,
                        _ => {
                            return Err(MetricsError::Eval(
                                "quantile requires a numeric parameter".into(),
                            ));
                        }
                    };
                    let mut sorted = values.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    fns::quantile_sorted(q, &sorted)
                }
                AggregateOp::Topk => {
                    let k = match agg.param.as_deref() {
                        Some(Expr::NumberLiteral(k)) => *k as usize,
                        _ => {
                            return Err(MetricsError::Eval(
                                "topk requires a numeric parameter".into(),
                            ));
                        }
                    };
                    let top = fns::topk(k, members);
                    for (l, v) in top {
                        out.push((l, v));
                    }
                    continue;
                }
                AggregateOp::Bottomk => {
                    let k = match agg.param.as_deref() {
                        Some(Expr::NumberLiteral(k)) => *k as usize,
                        _ => {
                            return Err(MetricsError::Eval(
                                "bottomk requires a numeric parameter".into(),
                            ));
                        }
                    };
                    let bot = fns::bottomk(k, members);
                    for (l, v) in bot {
                        out.push((l, v));
                    }
                    continue;
                }
                AggregateOp::CountValues => {
                    // Emit one series per distinct value, labelled with value.
                    let label_name = match agg.param.as_deref() {
                        Some(Expr::StringLiteral(s)) => s.clone(),
                        _ => "value".to_string(),
                    };
                    let mut counts: HashMap<String, f64> = HashMap::new();
                    for v in &values {
                        *counts.entry(format!("{}", v)).or_insert(0.0) += 1.0;
                    }
                    for (val_str, count) in counts {
                        let mut l = group_labels.clone();
                        l.insert(&label_name, val_str);
                        out.push((l, count));
                    }
                    continue;
                }
            };
            out.push((group_labels, result_val));
        }
        Ok(QueryResult::InstantVector(out))
    }

    fn eval_call(&self, call: &CallExpr, ts_ms: i64) -> Result<QueryResult> {
        match call.func.as_str() {
            // ── Range vector → instant vector ──────────────────────────────
            "rate" | "irate" | "increase" | "delta" | "idelta" | "deriv" | "resets" | "changes"
            | "avg_over_time" | "min_over_time" | "max_over_time" | "sum_over_time"
            | "count_over_time" | "stddev_over_time" | "stdvar_over_time" | "last_over_time"
            | "present_over_time" | "mad_over_time" | "ts_of_min_over_time"
            | "ts_of_max_over_time" | "ts_of_last_over_time" => {
                let rv = self.eval_range_vector_arg(call, ts_ms)?;
                let iv: Vec<(Labels, f64)> = rv
                    .into_iter()
                    .filter_map(|(labels, samps)| {
                        let val =
                            self.apply_range_fn(&call.func, &samps, self.range_ms_from_call(call))?;
                        Some((labels, val))
                    })
                    .collect();
                Ok(QueryResult::InstantVector(iv))
            }

            "quantile_over_time" => {
                let q = match call.args.first() {
                    Some(Expr::NumberLiteral(q)) => *q,
                    _ => {
                        return Err(MetricsError::Eval(
                            "quantile_over_time: first arg must be a number".into(),
                        ));
                    }
                };
                let rv = self.eval_range_vector_arg_at(call, 1, ts_ms)?;
                let iv: Vec<(Labels, f64)> = rv
                    .into_iter()
                    .filter_map(|(labels, samps)| {
                        fns::quantile_over_time(q, &samps).map(|v| (labels, v))
                    })
                    .collect();
                Ok(QueryResult::InstantVector(iv))
            }

            "predict_linear" => {
                let t = match call.args.get(1) {
                    Some(Expr::NumberLiteral(t)) => *t,
                    _ => {
                        return Err(MetricsError::Eval(
                            "predict_linear: second arg must be a number".into(),
                        ));
                    }
                };
                let rv = self.eval_range_vector_arg_at(call, 0, ts_ms)?;
                let iv: Vec<(Labels, f64)> = rv
                    .into_iter()
                    .filter_map(|(labels, samps)| {
                        fns::predict_linear(&samps, t).map(|v| (labels, v))
                    })
                    .collect();
                Ok(QueryResult::InstantVector(iv))
            }

            "double_exponential_smoothing" => {
                let sf = match call.args.get(1) {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => {
                        return Err(MetricsError::Eval(
                            "double_exponential_smoothing: smoothing factor must be a number".into(),
                        ));
                    }
                };
                let tf = match call.args.get(2) {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => {
                        return Err(MetricsError::Eval(
                            "double_exponential_smoothing: trend factor must be a number".into(),
                        ));
                    }
                };
                let rv = self.eval_range_vector_arg_at(call, 0, ts_ms)?;
                let iv: Vec<(Labels, f64)> = rv
                    .into_iter()
                    .filter_map(|(labels, samps)| {
                        fns::double_exponential_smoothing(&samps, sf, tf).map(|v| (labels, v))
                    })
                    .collect();
                Ok(QueryResult::InstantVector(iv))
            }

            "histogram_quantile" => {
                let q = match call.args.first() {
                    Some(Expr::NumberLiteral(q)) => *q,
                    _ => {
                        return Err(MetricsError::Eval(
                            "histogram_quantile: first arg must be a number".into(),
                        ));
                    }
                };
                // Evaluate the second arg (instant vector of _bucket series)
                let buckets_iv = match self.eval_instant(
                    call.args.get(1).ok_or_else(|| {
                        MetricsError::Eval("histogram_quantile: missing arg".into())
                    })?,
                    ts_ms,
                )? {
                    QueryResult::InstantVector(iv) => iv,
                    _ => {
                        return Err(MetricsError::Eval(
                            "histogram_quantile: arg must be an instant vector".into(),
                        ));
                    }
                };

                // Group by all labels except "le"
                let mut groups: HashMap<Labels, Vec<(f64, f64)>> = HashMap::new();
                for (labels, val) in buckets_iv {
                    let le: f64 = labels
                        .get("le")
                        .and_then(|v| {
                            if v == "+Inf" {
                                Some(f64::INFINITY)
                            } else {
                                v.parse().ok()
                            }
                        })
                        .unwrap_or(f64::INFINITY);
                    let key = labels.without(&["le"]);
                    groups.entry(key).or_default().push((le, val));
                }

                let iv: Vec<(Labels, f64)> = groups
                    .into_iter()
                    .map(|(labels, buckets)| (labels, fns::histogram_quantile(q, buckets)))
                    .collect();
                Ok(QueryResult::InstantVector(iv))
            }

            // ── Instant vector → instant vector ───────────────────────────
            "abs" => self.map_iv(call, ts_ms, |v| v.abs()),
            "ceil" => self.map_iv(call, ts_ms, |v| v.ceil()),
            "floor" => self.map_iv(call, ts_ms, |v| v.floor()),
            "exp" => self.map_iv(call, ts_ms, |v| v.exp()),
            "sqrt" => self.map_iv(call, ts_ms, |v| v.sqrt()),
            "ln" => self.map_iv(call, ts_ms, |v| v.ln()),
            "log2" => self.map_iv(call, ts_ms, |v| v.log2()),
            "log10" => self.map_iv(call, ts_ms, |v| v.log10()),
            "sgn" => self.map_iv(call, ts_ms, |v| {
                if v > 0.0 {
                    1.0
                } else if v < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }),

            // ── Trigonometric / hyperbolic / angle (Prometheus #8919) ──────
            "sin" => self.map_iv(call, ts_ms, |v| v.sin()),
            "cos" => self.map_iv(call, ts_ms, |v| v.cos()),
            "tan" => self.map_iv(call, ts_ms, |v| v.tan()),
            "asin" => self.map_iv(call, ts_ms, |v| v.asin()),
            "acos" => self.map_iv(call, ts_ms, |v| v.acos()),
            "atan" => self.map_iv(call, ts_ms, |v| v.atan()),
            "sinh" => self.map_iv(call, ts_ms, |v| v.sinh()),
            "cosh" => self.map_iv(call, ts_ms, |v| v.cosh()),
            "tanh" => self.map_iv(call, ts_ms, |v| v.tanh()),
            "asinh" => self.map_iv(call, ts_ms, |v| v.asinh()),
            "acosh" => self.map_iv(call, ts_ms, |v| v.acosh()),
            "atanh" => self.map_iv(call, ts_ms, |v| v.atanh()),
            "deg" => self.map_iv(call, ts_ms, |v| v.to_degrees()),
            "rad" => self.map_iv(call, ts_ms, |v| v.to_radians()),
            "pi" => Ok(QueryResult::Scalar(std::f64::consts::PI)),

            "round" => {
                let to = match call.args.get(1) {
                    Some(Expr::NumberLiteral(t)) => *t,
                    _ => 1.0,
                };
                self.map_iv_first(call, ts_ms, move |v| (v / to).round() * to)
            }

            "clamp" => {
                let min = match call.args.get(1) {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => f64::NEG_INFINITY,
                };
                let max = match call.args.get(2) {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => f64::INFINITY,
                };
                self.map_iv_first(call, ts_ms, move |v| fns::clamp(v, min, max))
            }

            "clamp_min" => {
                let min = match call.args.get(1) {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => f64::NEG_INFINITY,
                };
                self.map_iv_first(call, ts_ms, move |v| fns::clamp(v, min, f64::INFINITY))
            }
            "clamp_max" => {
                let max = match call.args.get(1) {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => f64::INFINITY,
                };
                self.map_iv_first(call, ts_ms, move |v| fns::clamp(v, f64::NEG_INFINITY, max))
            }

            "sort" => {
                let iv = self.eval_iv_first(call, ts_ms)?;
                Ok(QueryResult::InstantVector(fns::sort_asc(iv)))
            }
            "sort_desc" => {
                let iv = self.eval_iv_first(call, ts_ms)?;
                Ok(QueryResult::InstantVector(fns::sort_desc(iv)))
            }

            "sort_by_label" | "sort_by_label_desc" => {
                let iv = self.eval_iv_first(call, ts_ms)?;
                let sort_labels: Vec<String> = (1..call.args.len())
                    .filter_map(|i| self.string_arg(call, i).ok())
                    .collect();
                let label_refs: Vec<&str> = sort_labels.iter().map(|s| s.as_str()).collect();
                let desc = call.func == "sort_by_label_desc";
                Ok(QueryResult::InstantVector(fns::sort_by_label(
                    iv,
                    &label_refs,
                    desc,
                )))
            }

            // ── Label functions ────────────────────────────────────────────
            "label_replace" => {
                let iv = self.eval_iv_first(call, ts_ms)?;
                let dst = self.string_arg(call, 1)?;
                let rep = self.string_arg(call, 2)?;
                let src = self.string_arg(call, 3)?;
                let re = self.string_arg(call, 4)?;
                let out: Result<Vec<(Labels, f64)>> = iv
                    .into_iter()
                    .map(|(labels, val)| {
                        Ok((fns::label_replace(&labels, &dst, &rep, &src, &re)?, val))
                    })
                    .collect();
                Ok(QueryResult::InstantVector(out?))
            }

            "label_join" => {
                let iv = self.eval_iv_first(call, ts_ms)?;
                let dst = self.string_arg(call, 1)?;
                let sep = self.string_arg(call, 2)?;
                let src_labels: Vec<String> = (3..call.args.len())
                    .filter_map(|i| self.string_arg(call, i).ok())
                    .collect();
                let src_refs: Vec<&str> = src_labels.iter().map(|s| s.as_str()).collect();
                let out: Vec<(Labels, f64)> = iv
                    .into_iter()
                    .map(|(labels, val)| (fns::label_join(&labels, &dst, &sep, &src_refs), val))
                    .collect();
                Ok(QueryResult::InstantVector(out))
            }

            // ── Scalar / time ──────────────────────────────────────────────
            "scalar" => {
                match self.eval_instant(
                    call.args
                        .first()
                        .ok_or_else(|| MetricsError::Eval("scalar: missing arg".into()))?,
                    ts_ms,
                )? {
                    QueryResult::InstantVector(iv) if iv.len() == 1 => {
                        Ok(QueryResult::Scalar(iv[0].1))
                    }
                    QueryResult::Scalar(v) => Ok(QueryResult::Scalar(v)),
                    _ => Ok(QueryResult::Scalar(f64::NAN)),
                }
            }

            "vector" => {
                let val = match call.args.first() {
                    Some(Expr::NumberLiteral(v)) => *v,
                    _ => {
                        return Err(MetricsError::Eval(
                            "vector: first arg must be a number".into(),
                        ));
                    }
                };
                Ok(QueryResult::InstantVector(vec![(Labels::new(), val)]))
            }

            "time" => Ok(QueryResult::Scalar(ts_ms as f64 / 1000.0)),
            "timestamp" => {
                let iv = self.eval_iv_first(call, ts_ms)?;
                Ok(QueryResult::InstantVector(
                    iv.into_iter()
                        .map(|(l, _)| (l, ts_ms as f64 / 1000.0))
                        .collect(),
                ))
            }

            "day_of_month" => Ok(QueryResult::Scalar(fns::timestamp_to_day_of_month(ts_ms))),
            "day_of_week" => Ok(QueryResult::Scalar(fns::timestamp_to_day_of_week(ts_ms))),
            "day_of_year" => Ok(QueryResult::Scalar(fns::timestamp_to_day_of_year(ts_ms))),
            "days_in_month" => Ok(QueryResult::Scalar(fns::days_in_month(ts_ms))),
            "hour" => Ok(QueryResult::Scalar(fns::timestamp_to_hour(ts_ms))),
            "minute" => Ok(QueryResult::Scalar(fns::timestamp_to_minute(ts_ms))),
            "month" => Ok(QueryResult::Scalar(fns::timestamp_to_month(ts_ms))),
            "year" => Ok(QueryResult::Scalar(fns::timestamp_to_year(ts_ms))),

            // ── Absence detection ──────────────────────────────────────────
            "absent" => {
                let iv = self.eval_iv_first(call, ts_ms).unwrap_or_default();
                if iv.is_empty() {
                    Ok(QueryResult::InstantVector(vec![(Labels::new(), 1.0)]))
                } else {
                    Ok(QueryResult::InstantVector(vec![]))
                }
            }

            "absent_over_time" => {
                let rv = self.eval_range_vector_arg(call, ts_ms).unwrap_or_default();
                if rv.is_empty() {
                    Ok(QueryResult::InstantVector(vec![(Labels::new(), 1.0)]))
                } else {
                    Ok(QueryResult::InstantVector(vec![]))
                }
            }

            unknown => Err(MetricsError::Eval(format!("unknown function: {}", unknown))),
        }
    }

    // ─── helpers ─────────────────────────────────────────────────────────────

    fn eval_range_vector_arg(
        &self,
        call: &CallExpr,
        ts_ms: i64,
    ) -> Result<Vec<(Labels, Vec<Sample>)>> {
        self.eval_range_vector_arg_at(call, 0, ts_ms)
    }

    fn eval_range_vector_arg_at(
        &self,
        call: &CallExpr,
        idx: usize,
        ts_ms: i64,
    ) -> Result<Vec<(Labels, Vec<Sample>)>> {
        let arg = call
            .args
            .get(idx)
            .ok_or_else(|| MetricsError::Eval(format!("{}: missing arg {}", call.func, idx)))?;
        match self.eval_instant(arg, ts_ms)? {
            QueryResult::RangeVector(rv) => Ok(rv),
            _ => Err(MetricsError::Eval(format!(
                "{}: arg {} must be a range vector",
                call.func, idx
            ))),
        }
    }

    fn range_ms_from_call(&self, call: &CallExpr) -> i64 {
        if let Some(Expr::MatrixSelector(ms)) = call.args.first() {
            ms.range_ms
        } else {
            DEFAULT_LOOKBACK_MS
        }
    }

    fn apply_range_fn(&self, func: &str, samps: &[Sample], range_ms: i64) -> Option<f64> {
        match func {
            "rate" => fns::rate(samps, range_ms),
            "irate" => fns::irate(samps),
            "increase" => fns::increase(samps, range_ms),
            "delta" => fns::delta(samps, range_ms),
            "idelta" => fns::idelta(samps),
            "deriv" => fns::deriv(samps),
            "resets" => Some(fns::resets(samps)),
            "changes" => Some(fns::changes(samps)),
            "avg_over_time" => fns::avg_over_time(samps),
            "min_over_time" => fns::min_over_time(samps),
            "max_over_time" => fns::max_over_time(samps),
            "sum_over_time" => fns::sum_over_time(samps),
            "count_over_time" => fns::count_over_time(samps),
            "stddev_over_time" => fns::stddev_over_time(samps),
            "stdvar_over_time" => fns::stdvar_over_time(samps),
            "last_over_time" => fns::last_over_time(samps),
            "present_over_time" => fns::present_over_time(samps),
            "mad_over_time" => fns::mad_over_time(samps),
            "ts_of_min_over_time" => fns::ts_of_min_over_time(samps),
            "ts_of_max_over_time" => fns::ts_of_max_over_time(samps),
            "ts_of_last_over_time" => fns::ts_of_last_over_time(samps),
            _ => None,
        }
    }

    fn eval_iv_first(&self, call: &CallExpr, ts_ms: i64) -> Result<Vec<(Labels, f64)>> {
        let arg = call
            .args
            .first()
            .ok_or_else(|| MetricsError::Eval(format!("{}: missing arg", call.func)))?;
        match self.eval_instant(arg, ts_ms)? {
            QueryResult::InstantVector(iv) => Ok(iv),
            QueryResult::Scalar(v) => Ok(vec![(Labels::new(), v)]),
            _ => Err(MetricsError::Eval(format!(
                "{}: arg must be an instant vector",
                call.func
            ))),
        }
    }

    fn map_iv(&self, call: &CallExpr, ts_ms: i64, f: impl Fn(f64) -> f64) -> Result<QueryResult> {
        let iv = self.eval_iv_first(call, ts_ms)?;
        Ok(QueryResult::InstantVector(
            iv.into_iter().map(|(l, v)| (l, f(v))).collect(),
        ))
    }

    fn map_iv_first(
        &self,
        call: &CallExpr,
        ts_ms: i64,
        f: impl Fn(f64) -> f64,
    ) -> Result<QueryResult> {
        self.map_iv(call, ts_ms, f)
    }

    fn string_arg(&self, call: &CallExpr, idx: usize) -> Result<String> {
        match call.args.get(idx) {
            Some(Expr::StringLiteral(s)) => Ok(s.clone()),
            _ => Err(MetricsError::Eval(format!(
                "{}: arg {} must be a string",
                call.func, idx
            ))),
        }
    }
}

// ─── Vector matching helpers ─────────────────────────────────────────────────

fn matching_key(labels: &Labels, matching: Option<&VectorMatching>) -> Labels {
    match matching {
        None => labels.without_name(),
        Some(vm) if vm.on => {
            labels.with_only(&vm.labels.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        }
        Some(vm) => labels.without(&vm.labels.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
    }
}

fn result_labels(labels: &Labels, matching: Option<&VectorMatching>) -> Labels {
    match matching {
        None => labels.without_name(),
        Some(vm) if vm.on => {
            labels.with_only(&vm.labels.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        }
        Some(vm) => labels.without(&vm.labels.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
    }
}

fn group_key(labels: &Labels, grouping: &Grouping) -> Labels {
    if grouping.labels.is_empty() && !grouping.without {
        // No grouping: single group (empty label set)
        return Labels::new();
    }
    let keys: Vec<&str> = grouping.labels.iter().map(|s| s.as_str()).collect();
    if grouping.without {
        labels.without(&keys)
    } else {
        labels.with_only(&keys)
    }
}
