// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! PhysicalPlan + ExecutionPlan — DataFusion physical operators.
//!
//! Upstream: `crates/datafusion-physical-plan/src/*.rs`
//!
//! Each PhysicalPlan node lowers from a LogicalPlan node. The MVP
//! ships the eight operators the user spec lists: scan / filter /
//! project / aggregate / sort / limit / cross-join / hash-join. The
//! executor runs row-at-a-time over Vec<Row>; swapping to vectorized
//! Arrow batches is a v0.2 milestone.

use crate::error::Result;
use crate::functions::AggregateKind;
use crate::physical_expr::PhysicalExpr;
use crate::row::{Row, Value};
use crate::schema::SchemaRef;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalPlan {
    /// In-memory scan — `rows` already materialized.
    InMemoryScan { rows: Vec<Row>, schema: SchemaRef },
    /// Filter rows by predicate.
    Filter { predicate: PhysicalExpr, input: Box<PhysicalPlan>, schema: SchemaRef },
    /// Project to a new column list (selection + computed columns).
    Projection { expressions: Vec<PhysicalExpr>, input: Box<PhysicalPlan>, schema: SchemaRef },
    /// GROUP BY + aggregate. Each aggregate is `(kind, column_index)`.
    Aggregate {
        group_by: Vec<PhysicalExpr>,
        aggr: Vec<(AggregateKind, PhysicalExpr)>,
        input: Box<PhysicalPlan>,
        schema: SchemaRef,
    },
    Sort { keys: Vec<SortPhysical>, input: Box<PhysicalPlan>, schema: SchemaRef },
    Limit { skip: usize, fetch: Option<usize>, input: Box<PhysicalPlan>, schema: SchemaRef },
    /// Cross join (nested-loop). Used as both the cross-product
    /// fallback and as the start of a hash join after the build side
    /// has been keyed.
    CrossJoin { left: Box<PhysicalPlan>, right: Box<PhysicalPlan>, schema: SchemaRef },
    /// Hash inner-join — naive HashMap on a single key index per side.
    HashJoin {
        left: Box<PhysicalPlan>,
        right: Box<PhysicalPlan>,
        left_key: PhysicalExpr,
        right_key: PhysicalExpr,
        schema: SchemaRef,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SortPhysical {
    pub expr: PhysicalExpr,
    pub ascending: bool,
    pub nulls_first: bool,
}

impl PhysicalPlan {
    pub fn schema(&self) -> SchemaRef {
        match self {
            Self::InMemoryScan { schema, .. }
            | Self::Filter { schema, .. }
            | Self::Projection { schema, .. }
            | Self::Aggregate { schema, .. }
            | Self::Sort { schema, .. }
            | Self::Limit { schema, .. }
            | Self::CrossJoin { schema, .. }
            | Self::HashJoin { schema, .. } => schema.clone(),
        }
    }
}

pub trait ExecutionPlan {
    fn execute(&self) -> Result<Vec<Row>>;
}

impl ExecutionPlan for PhysicalPlan {
    fn execute(&self) -> Result<Vec<Row>> {
        match self {
            Self::InMemoryScan { rows, .. } => Ok(rows.clone()),

            Self::Filter { predicate, input, .. } => {
                let rows = input.execute()?;
                let mut out = Vec::new();
                for r in rows {
                    let v = predicate.evaluate(&r)?;
                    if matches!(v, Value::Bool(true)) {
                        out.push(r);
                    }
                }
                Ok(out)
            }

            Self::Projection { expressions, input, .. } => {
                let rows = input.execute()?;
                let mut out = Vec::with_capacity(rows.len());
                for r in rows {
                    let mut new_vals = Vec::with_capacity(expressions.len());
                    for e in expressions {
                        new_vals.push(e.evaluate(&r)?);
                    }
                    out.push(Row::new(new_vals));
                }
                Ok(out)
            }

            Self::Sort { keys, input, .. } => {
                let mut rows = input.execute()?;
                rows.sort_by(|a, b| {
                    for k in keys {
                        let av = k.expr.evaluate(a).unwrap_or(Value::Null);
                        let bv = k.expr.evaluate(b).unwrap_or(Value::Null);
                        let mut ord = av.cmp_nulls_first(&bv);
                        if !k.ascending {
                            ord = ord.reverse();
                        }
                        // Push NULLs to the end when nulls_first=false.
                        if !k.nulls_first {
                            ord = match (av.is_null(), bv.is_null()) {
                                (true, false) => std::cmp::Ordering::Greater,
                                (false, true) => std::cmp::Ordering::Less,
                                _ => ord,
                            };
                        }
                        if !ord.is_eq() {
                            return ord;
                        }
                    }
                    std::cmp::Ordering::Equal
                });
                Ok(rows)
            }

            Self::Limit { skip, fetch, input, .. } => {
                let rows = input.execute()?;
                let it = rows.into_iter().skip(*skip);
                Ok(match fetch {
                    Some(n) => it.take(*n).collect(),
                    None => it.collect(),
                })
            }

            Self::Aggregate { group_by, aggr, input, .. } => exec_aggregate(group_by, aggr, input.execute()?),

            Self::CrossJoin { left, right, .. } => {
                let l = left.execute()?;
                let r = right.execute()?;
                let mut out = Vec::with_capacity(l.len() * r.len());
                for lr in &l {
                    for rr in &r {
                        let mut v = lr.values.clone();
                        v.extend(rr.values.clone());
                        out.push(Row::new(v));
                    }
                }
                Ok(out)
            }

            Self::HashJoin { left, right, left_key, right_key, .. } => {
                let l = left.execute()?;
                let r = right.execute()?;
                // Build phase — hash right side by key.
                let mut by_key: std::collections::HashMap<String, Vec<Row>> =
                    std::collections::HashMap::new();
                for rr in r {
                    let k = right_key.evaluate(&rr)?;
                    by_key.entry(key_to_string(&k)).or_default().push(rr);
                }
                // Probe phase.
                let mut out = Vec::new();
                for lr in l {
                    let k = left_key.evaluate(&lr)?;
                    if let Some(matches) = by_key.get(&key_to_string(&k)) {
                        for m in matches {
                            let mut vals = lr.values.clone();
                            vals.extend(m.values.clone());
                            out.push(Row::new(vals));
                        }
                    }
                }
                Ok(out)
            }
        }
    }
}

fn key_to_string(v: &Value) -> String {
    match v {
        Value::Null => "__null__".into(),
        Value::Bool(b) => format!("b:{}", b),
        Value::Int32(n) => format!("i:{}", n),
        Value::Int64(n) => format!("i:{}", n),
        Value::Float64(n) => format!("f:{}", n.to_bits()),
        Value::Utf8(s) => format!("s:{}", s),
    }
}

fn exec_aggregate(
    group_by: &[PhysicalExpr],
    aggr: &[(AggregateKind, PhysicalExpr)],
    rows: Vec<Row>,
) -> Result<Vec<Row>> {
    // Per-group accumulator state.
    let mut groups: BTreeMap<String, (Vec<Value>, Vec<Accumulator>)> = BTreeMap::new();

    for r in rows {
        let mut key_parts = Vec::with_capacity(group_by.len());
        let mut key_str = String::new();
        for g in group_by {
            let v = g.evaluate(&r)?;
            key_str.push_str(&key_to_string(&v));
            key_str.push('|');
            key_parts.push(v);
        }
        let entry = groups.entry(key_str).or_insert_with(|| {
            (
                key_parts.clone(),
                aggr.iter().map(|(k, _)| Accumulator::new(*k)).collect(),
            )
        });
        for (i, (_kind, expr)) in aggr.iter().enumerate() {
            let v = expr.evaluate(&r)?;
            entry.1[i].update(&v);
        }
    }

    // Emit one row per group.
    let mut out = Vec::with_capacity(groups.len());
    for (_, (key_vals, accs)) in groups {
        let mut vals = key_vals;
        for a in accs {
            vals.push(a.finalize());
        }
        out.push(Row::new(vals));
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct Accumulator {
    kind: AggregateKind,
    count: i64,
    sum: f64,
    min: Option<Value>,
    max: Option<Value>,
}

impl Accumulator {
    fn new(kind: AggregateKind) -> Self {
        Self {
            kind,
            count: 0,
            sum: 0.0,
            min: None,
            max: None,
        }
    }

    fn update(&mut self, v: &Value) {
        if v.is_null() && self.kind != AggregateKind::Count {
            return;
        }
        if !v.is_null() {
            self.count += 1;
            if let Some(f) = v.as_f64() {
                self.sum += f;
            }
            match &self.min {
                None => self.min = Some(v.clone()),
                Some(m) => {
                    if v.cmp_nulls_first(m).is_lt() {
                        self.min = Some(v.clone());
                    }
                }
            }
            match &self.max {
                None => self.max = Some(v.clone()),
                Some(m) => {
                    if v.cmp_nulls_first(m).is_gt() {
                        self.max = Some(v.clone());
                    }
                }
            }
        }
    }

    fn finalize(self) -> Value {
        match self.kind {
            AggregateKind::Count => Value::Int64(self.count),
            AggregateKind::Sum => {
                if self.count == 0 {
                    Value::Null
                } else {
                    Value::Float64(self.sum)
                }
            }
            AggregateKind::Avg => {
                if self.count == 0 {
                    Value::Null
                } else {
                    Value::Float64(self.sum / self.count as f64)
                }
            }
            AggregateKind::Min => self.min.unwrap_or(Value::Null),
            AggregateKind::Max => self.max.unwrap_or(Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_expr::BinaryPhysicalOp;
    use crate::schema::{DataType, Field, TableSchema};
    use std::sync::Arc;

    fn schema_ab() -> SchemaRef {
        Arc::new(TableSchema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Int64, true),
        ]))
    }

    fn rows_ab() -> Vec<Row> {
        vec![
            Row::new(vec![Value::Int64(1), Value::Int64(10)]),
            Row::new(vec![Value::Int64(2), Value::Int64(20)]),
            Row::new(vec![Value::Int64(3), Value::Null]),
        ]
    }

    #[test]
    fn filter_drops_non_matching_rows() {
        let scan = PhysicalPlan::InMemoryScan { rows: rows_ab(), schema: schema_ab() };
        let f = PhysicalPlan::Filter {
            predicate: PhysicalExpr::Binary {
                op: BinaryPhysicalOp::Gt,
                left: Box::new(PhysicalExpr::Column { index: 0 }),
                right: Box::new(PhysicalExpr::Literal { value: Value::Int64(1) }),
            },
            input: Box::new(scan),
            schema: schema_ab(),
        };
        let out = f.execute().unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn projection_emits_computed_columns() {
        let scan = PhysicalPlan::InMemoryScan { rows: rows_ab(), schema: schema_ab() };
        let p = PhysicalPlan::Projection {
            expressions: vec![
                PhysicalExpr::Column { index: 0 },
                PhysicalExpr::Binary {
                    op: BinaryPhysicalOp::Multiply,
                    left: Box::new(PhysicalExpr::Column { index: 0 }),
                    right: Box::new(PhysicalExpr::Literal { value: Value::Int64(10) }),
                },
            ],
            input: Box::new(scan),
            schema: schema_ab(),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].values[1], Value::Int64(10));
        assert_eq!(out[2].values[1], Value::Int64(30));
    }

    #[test]
    fn sort_orders_asc_by_default_with_null_first() {
        let scan = PhysicalPlan::InMemoryScan { rows: rows_ab(), schema: schema_ab() };
        let s = PhysicalPlan::Sort {
            keys: vec![SortPhysical {
                expr: PhysicalExpr::Column { index: 1 },
                ascending: true,
                nulls_first: true,
            }],
            input: Box::new(scan),
            schema: schema_ab(),
        };
        let out = s.execute().unwrap();
        assert!(out[0].values[1].is_null());
    }

    #[test]
    fn limit_skips_and_fetches() {
        let scan = PhysicalPlan::InMemoryScan { rows: rows_ab(), schema: schema_ab() };
        let l = PhysicalPlan::Limit {
            skip: 1,
            fetch: Some(1),
            input: Box::new(scan),
            schema: schema_ab(),
        };
        let out = l.execute().unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].values[0], Value::Int64(2));
    }

    #[test]
    fn aggregate_sum_and_count_by_group() {
        let rows = vec![
            Row::new(vec![Value::Utf8("x".into()), Value::Int64(10)]),
            Row::new(vec![Value::Utf8("x".into()), Value::Int64(20)]),
            Row::new(vec![Value::Utf8("y".into()), Value::Int64(5)]),
        ];
        let schema: SchemaRef = Arc::new(TableSchema::new(vec![
            Field::new("g", DataType::Utf8, false),
            Field::new("v", DataType::Int64, false),
        ]));
        let scan = PhysicalPlan::InMemoryScan { rows, schema: schema.clone() };
        let agg = PhysicalPlan::Aggregate {
            group_by: vec![PhysicalExpr::Column { index: 0 }],
            aggr: vec![
                (AggregateKind::Sum, PhysicalExpr::Column { index: 1 }),
                (AggregateKind::Count, PhysicalExpr::Column { index: 1 }),
            ],
            input: Box::new(scan),
            schema,
        };
        let out = agg.execute().unwrap();
        assert_eq!(out.len(), 2);
        // BTreeMap ordering means "x" before "y".
        assert_eq!(out[0].values[0], Value::Utf8("x".into()));
        assert_eq!(out[0].values[1], Value::Float64(30.0));
        assert_eq!(out[0].values[2], Value::Int64(2));
        assert_eq!(out[1].values[1], Value::Float64(5.0));
    }

    #[test]
    fn cross_join_emits_cartesian_product() {
        let l = PhysicalPlan::InMemoryScan {
            rows: vec![Row::new(vec![Value::Int64(1)]), Row::new(vec![Value::Int64(2)])],
            schema: Arc::new(TableSchema::new(vec![Field::new(
                "l",
                DataType::Int64,
                false,
            )])),
        };
        let r = PhysicalPlan::InMemoryScan {
            rows: vec![Row::new(vec![Value::Utf8("x".into())])],
            schema: Arc::new(TableSchema::new(vec![Field::new(
                "r",
                DataType::Utf8,
                false,
            )])),
        };
        let cj = PhysicalPlan::CrossJoin {
            left: Box::new(l),
            right: Box::new(r),
            schema: Arc::new(TableSchema::new(vec![])),
        };
        let out = cj.execute().unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn hash_join_inner_matches() {
        let l = PhysicalPlan::InMemoryScan {
            rows: vec![
                Row::new(vec![Value::Int64(1), Value::Utf8("a".into())]),
                Row::new(vec![Value::Int64(2), Value::Utf8("b".into())]),
            ],
            schema: Arc::new(TableSchema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("v", DataType::Utf8, false),
            ])),
        };
        let r = PhysicalPlan::InMemoryScan {
            rows: vec![
                Row::new(vec![Value::Int64(2), Value::Utf8("Z".into())]),
                Row::new(vec![Value::Int64(3), Value::Utf8("W".into())]),
            ],
            schema: Arc::new(TableSchema::new(vec![
                Field::new("id2", DataType::Int64, false),
                Field::new("v2", DataType::Utf8, false),
            ])),
        };
        let hj = PhysicalPlan::HashJoin {
            left: Box::new(l),
            right: Box::new(r),
            left_key: PhysicalExpr::Column { index: 0 },
            right_key: PhysicalExpr::Column { index: 0 },
            schema: Arc::new(TableSchema::new(vec![])),
        };
        let out = hj.execute().unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].values[0], Value::Int64(2));
    }
}
