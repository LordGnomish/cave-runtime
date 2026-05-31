// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SessionContext — DataFusion's user-facing entry point.
//!
//! Upstream: `crates/datafusion/src/execution/session_state.rs` +
//! `crates/datafusion/src/execution/context/mod.rs`
//!
//! `SessionContext::new()` is the canonical "spin up an engine".
//! It owns a SessionCatalog, a FunctionRegistry, and the entry-point
//! methods that let callers register tables, build a DataFrame, parse
//! SQL into a LogicalPlan, and (eventually) execute against the row
//! engine.

use crate::catalog::SessionCatalog;
use crate::data_source::{MemTable, TableProvider};
use crate::dataframe::DataFrame;
use crate::error::{Error, Result};
use crate::functions::{AggregateKind, FunctionRegistry};
use crate::logical_expr::LogicalExpr;
use crate::logical_plan::{LogicalPlan, SortKey};
use crate::physical_expr::{BinaryPhysicalOp, PhysicalExpr};
use crate::physical_plan::{ExecutionPlan, PhysicalPlan, SortPhysical};
use crate::row::{Row, Value};
use crate::schema::SchemaRef;
use crate::sql_parser::{SelectStatement, parse_sql};
use std::sync::Arc;

#[derive(Default)]
pub struct SessionContext {
    catalog: SessionCatalog,
    functions: FunctionRegistry,
}

impl SessionContext {
    pub fn new() -> Self {
        Self {
            catalog: SessionCatalog::new(),
            functions: FunctionRegistry::new(),
        }
    }

    pub fn catalog(&self) -> &SessionCatalog {
        &self.catalog
    }

    pub fn functions(&self) -> &FunctionRegistry {
        &self.functions
    }

    /// Register an in-memory table for SQL/DataFrame access.
    pub async fn register_table(
        &self,
        name: impl Into<String>,
        provider: Arc<dyn TableProvider>,
    ) -> Result<()> {
        self.catalog.register_table(name, provider).await
    }

    /// Convenience: register an in-memory `MemTable` constructed from
    /// a schema + rows pair.
    pub async fn register_mem_table(
        &self,
        name: impl Into<String>,
        schema: SchemaRef,
        rows: Vec<Row>,
    ) -> Result<()> {
        let t = MemTable::new(schema, rows)?;
        self.catalog.register_table(name, Arc::new(t)).await
    }

    /// Build a DataFrame off a registered table.
    pub async fn table(&self, name: &str) -> Result<DataFrame> {
        let p = self.catalog.table(name).await?;
        Ok(DataFrame::from_plan(LogicalPlan::TableScan {
            table_name: name.to_string(),
            schema: p.schema(),
            projection: None,
            filters: vec![],
        }))
    }

    /// Parse a SQL string into a LogicalPlan. The table referenced in
    /// `FROM` must already be registered.
    pub async fn sql_to_plan(&self, sql: &str) -> Result<LogicalPlan> {
        let stmt = parse_sql(sql)?;
        let mut plan = match &stmt.from {
            Some(name) => {
                let p = self.catalog.table(name).await?;
                LogicalPlan::TableScan {
                    table_name: name.clone(),
                    schema: p.schema(),
                    projection: None,
                    filters: vec![],
                }
            }
            None => {
                return Err(Error::Plan("MVP SQL requires FROM".into()));
            }
        };
        if let Some(p) = stmt.where_clause.clone() {
            plan = LogicalPlan::Filter {
                predicate: p,
                input: Box::new(plan),
            };
        }
        let sort_keys: Vec<SortKey> = stmt
            .order_by
            .iter()
            .map(|(e, asc)| {
                if *asc {
                    SortKey::asc(e.clone())
                } else {
                    SortKey::desc(e.clone())
                }
            })
            .collect();

        // A query aggregates when it has a GROUP BY *or* the projection
        // contains an aggregate function with no GROUP BY (implicit
        // whole-table aggregation → a single group over all rows).
        let is_aggregate = !stmt.group_by.is_empty() || self.select_has_aggregate(&stmt);

        if is_aggregate {
            // Aggregates in the select list are detected by name lookup
            // against the function registry. ORDER BY for a grouped query
            // sorts the *aggregated* output, so the Sort sits above the
            // Aggregate node.
            let (group_by, aggr) = self.partition_select_list_for_aggregate(&stmt);
            plan = LogicalPlan::Aggregate {
                group_by,
                aggr,
                input: Box::new(plan),
            };
            if !sort_keys.is_empty() {
                plan = LogicalPlan::Sort {
                    keys: sort_keys,
                    input: Box::new(plan),
                };
            }
        } else {
            // Non-aggregate: ORDER BY expressions are resolved against the
            // *input* to the projection (port of DataFusion's
            // LogicalPlanBuilder, which sorts before the final projection).
            // This lets a sort key reference a scalar function of a column
            // or a column that is not in the SELECT list, and keeps the
            // sort-key column indices aligned with the rows the SortExec
            // actually sees. The Sort therefore sits *below* the Projection.
            if !sort_keys.is_empty() {
                plan = LogicalPlan::Sort {
                    keys: sort_keys,
                    input: Box::new(plan),
                };
            }
            if !stmt.select_list.is_empty()
                && !matches!(&stmt.select_list[0], LogicalExpr::Column { name } if name == "*")
            {
                plan = LogicalPlan::Projection {
                    expressions: stmt.select_list.clone(),
                    input: Box::new(plan),
                };
            }
        }
        if stmt.limit.is_some() || stmt.offset.is_some() {
            plan = LogicalPlan::Limit {
                skip: stmt.offset.unwrap_or(0),
                fetch: stmt.limit,
                input: Box::new(plan),
            };
        }
        Ok(plan)
    }

    /// True when any top-level select expression is an aggregate
    /// function (`count`, `sum`, `min`, `max`, …). Used to route a
    /// GROUP-BY-less query through the Aggregate plan node instead of a
    /// scalar projection.
    fn select_has_aggregate(&self, stmt: &SelectStatement) -> bool {
        stmt.select_list.iter().any(|e| {
            matches!(e, LogicalExpr::Function { name, .. }
                if self.functions.lookup_aggregate(name).is_some())
        })
    }

    fn partition_select_list_for_aggregate(
        &self,
        stmt: &SelectStatement,
    ) -> (Vec<LogicalExpr>, Vec<LogicalExpr>) {
        let mut group_by = stmt.group_by.clone();
        let mut aggr = Vec::new();
        for e in &stmt.select_list {
            match e {
                LogicalExpr::Function { name, .. }
                    if self.functions.lookup_aggregate(name).is_some() =>
                {
                    aggr.push(e.clone());
                }
                _ => {
                    if !group_by.iter().any(|g| g == e) {
                        group_by.push(e.clone());
                    }
                }
            }
        }
        (group_by, aggr)
    }

    /// Execute a SQL statement end-to-end. The MVP supports:
    /// `SELECT col* FROM table [WHERE predicate] [GROUP BY ...]
    /// [ORDER BY ...] [LIMIT n] [OFFSET n]` against in-memory
    /// MemTables registered with the catalog.
    pub async fn sql(&self, sql: &str) -> Result<Vec<Row>> {
        let plan = self.sql_to_plan(sql).await?;
        self.execute_plan(&plan).await
    }

    pub async fn execute_plan(&self, plan: &LogicalPlan) -> Result<Vec<Row>> {
        let physical = self.lower(plan).await?;
        physical.execute()
    }

    fn lower<'a>(
        &'a self,
        plan: &'a LogicalPlan,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PhysicalPlan>> + Send + 'a>>
    {
        Box::pin(self.lower_inner(plan))
    }

    async fn lower_inner(&self, plan: &LogicalPlan) -> Result<PhysicalPlan> {
        match plan {
            LogicalPlan::TableScan {
                table_name, schema, ..
            } => {
                let p = self.catalog.table(table_name).await?;
                let rows = p.scan().await?;
                Ok(PhysicalPlan::InMemoryScan {
                    rows,
                    schema: schema.clone(),
                })
            }
            LogicalPlan::Filter { predicate, input } => {
                let in_schema = schema_of(input.as_ref());
                let phys_pred = self.lower_expr(predicate, &in_schema)?;
                let lowered = Box::new(self.lower(input).await?);
                Ok(PhysicalPlan::Filter {
                    predicate: phys_pred,
                    schema: in_schema,
                    input: lowered,
                })
            }
            LogicalPlan::Projection { expressions, input } => {
                let in_schema = schema_of(input.as_ref());
                let phys_exprs: Vec<PhysicalExpr> = expressions
                    .iter()
                    .map(|e| self.lower_expr(e, &in_schema))
                    .collect::<Result<_>>()?;
                let lowered = Box::new(self.lower(input).await?);
                Ok(PhysicalPlan::Projection {
                    expressions: phys_exprs,
                    schema: in_schema,
                    input: lowered,
                })
            }
            LogicalPlan::Sort { keys, input } => {
                let in_schema = schema_of(input.as_ref());
                let phys_keys: Vec<SortPhysical> = keys
                    .iter()
                    .map(|k| {
                        Ok(SortPhysical {
                            expr: self.lower_expr(&k.expr, &in_schema)?,
                            ascending: k.ascending,
                            nulls_first: k.nulls_first,
                        })
                    })
                    .collect::<Result<_>>()?;
                let lowered = Box::new(self.lower(input).await?);
                Ok(PhysicalPlan::Sort {
                    keys: phys_keys,
                    schema: in_schema,
                    input: lowered,
                })
            }
            LogicalPlan::Limit { skip, fetch, input } => {
                let in_schema = schema_of(input.as_ref());
                let lowered = Box::new(self.lower(input).await?);
                Ok(PhysicalPlan::Limit {
                    skip: *skip,
                    fetch: *fetch,
                    input: lowered,
                    schema: in_schema,
                })
            }
            LogicalPlan::Aggregate {
                group_by,
                aggr,
                input,
            } => {
                let in_schema = schema_of(input.as_ref());
                let phys_group: Vec<PhysicalExpr> = group_by
                    .iter()
                    .map(|e| self.lower_expr(e, &in_schema))
                    .collect::<Result<_>>()?;
                let phys_aggr: Vec<(AggregateKind, PhysicalExpr)> = aggr
                    .iter()
                    .map(|e| match e {
                        LogicalExpr::Function { name, args } => {
                            let k = self.functions.lookup_aggregate(name).ok_or_else(|| {
                                Error::Plan(format!("unknown aggregate: {}", name))
                            })?;
                            let arg = args.first().cloned().unwrap_or(LogicalExpr::lit(1));
                            Ok((k, self.lower_expr(&arg, &in_schema)?))
                        }
                        _ => Err(Error::Plan("expected aggregate function".into())),
                    })
                    .collect::<Result<_>>()?;
                let lowered = Box::new(self.lower(input).await?);
                Ok(PhysicalPlan::Aggregate {
                    group_by: phys_group,
                    aggr: phys_aggr,
                    input: lowered,
                    schema: in_schema,
                })
            }
            LogicalPlan::Join {
                kind: _,
                on,
                left,
                right,
            } => {
                let left_schema = schema_of(left.as_ref());
                let right_schema = schema_of(right.as_ref());
                let l = Box::new(self.lower(left).await?);
                let r = Box::new(self.lower(right).await?);
                if on.is_empty() {
                    Ok(PhysicalPlan::CrossJoin {
                        left: l,
                        right: r,
                        schema: left_schema,
                    })
                } else {
                    let (le, re) = on[0].clone();
                    let lk = self.lower_expr(&le, &left_schema)?;
                    let rk = self.lower_expr(&re, &right_schema)?;
                    Ok(PhysicalPlan::HashJoin {
                        left: l,
                        right: r,
                        left_key: lk,
                        right_key: rk,
                        schema: left_schema,
                    })
                }
            }
            LogicalPlan::Union { .. } | LogicalPlan::EmptyRelation { .. } => {
                Err(Error::Plan("Union/EmptyRelation not yet executable".into()))
            }
        }
    }

    fn lower_expr(&self, e: &LogicalExpr, schema: &SchemaRef) -> Result<PhysicalExpr> {
        match e {
            LogicalExpr::Column { name } => {
                let idx = schema
                    .index_of(name)
                    .ok_or_else(|| Error::Schema(format!("column not found: {}", name)))?;
                Ok(PhysicalExpr::Column { index: idx })
            }
            LogicalExpr::Literal { value } => Ok(PhysicalExpr::Literal {
                value: value.clone(),
            }),
            LogicalExpr::BinaryOp { op, left, right } => Ok(PhysicalExpr::Binary {
                op: match op {
                    crate::logical_expr::BinaryOp::Plus => BinaryPhysicalOp::Plus,
                    crate::logical_expr::BinaryOp::Minus => BinaryPhysicalOp::Minus,
                    crate::logical_expr::BinaryOp::Multiply => BinaryPhysicalOp::Multiply,
                    crate::logical_expr::BinaryOp::Divide => BinaryPhysicalOp::Divide,
                    crate::logical_expr::BinaryOp::Modulo => BinaryPhysicalOp::Modulo,
                    crate::logical_expr::BinaryOp::Eq => BinaryPhysicalOp::Eq,
                    crate::logical_expr::BinaryOp::NotEq => BinaryPhysicalOp::NotEq,
                    crate::logical_expr::BinaryOp::Lt => BinaryPhysicalOp::Lt,
                    crate::logical_expr::BinaryOp::LtEq => BinaryPhysicalOp::LtEq,
                    crate::logical_expr::BinaryOp::Gt => BinaryPhysicalOp::Gt,
                    crate::logical_expr::BinaryOp::GtEq => BinaryPhysicalOp::GtEq,
                    crate::logical_expr::BinaryOp::And => BinaryPhysicalOp::And,
                    crate::logical_expr::BinaryOp::Or => BinaryPhysicalOp::Or,
                },
                left: Box::new(self.lower_expr(left, schema)?),
                right: Box::new(self.lower_expr(right, schema)?),
            }),
            LogicalExpr::Not { expr } => Ok(PhysicalExpr::Not {
                expr: Box::new(self.lower_expr(expr, schema)?),
            }),
            LogicalExpr::IsNull { expr } => Ok(PhysicalExpr::IsNull {
                expr: Box::new(self.lower_expr(expr, schema)?),
            }),
            LogicalExpr::IsNotNull { expr } => Ok(PhysicalExpr::IsNotNull {
                expr: Box::new(self.lower_expr(expr, schema)?),
            }),
            LogicalExpr::Cast { expr, to } => Ok(PhysicalExpr::Cast {
                expr: Box::new(self.lower_expr(expr, schema)?),
                to: *to,
            }),
            LogicalExpr::Alias { expr, .. } => self.lower_expr(expr, schema),
            LogicalExpr::Function { name, args } => {
                // Scalar function. Lower each argument; if every argument
                // folds to a literal, invoke eagerly at plan time (constant
                // folding). Otherwise emit a `PhysicalExpr::Call` carrying
                // the resolved function for per-row invocation by the
                // executor (port of `ScalarFunctionExpr`).
                let f = self
                    .functions
                    .lookup_scalar(name)
                    .ok_or_else(|| Error::Plan(format!("unknown scalar function: {}", name)))?;
                let lowered_args: Vec<PhysicalExpr> = args
                    .iter()
                    .map(|a| self.lower_expr(a, schema))
                    .collect::<Result<_>>()?;
                if lowered_args
                    .iter()
                    .all(|a| matches!(a, PhysicalExpr::Literal { .. }))
                {
                    let evaluated: Vec<Value> = lowered_args
                        .iter()
                        .map(|a| match a {
                            PhysicalExpr::Literal { value } => value.clone(),
                            _ => unreachable!("guarded by all-literal check"),
                        })
                        .collect();
                    let val = (f.fun)(&evaluated)?;
                    Ok(PhysicalExpr::Literal { value: val })
                } else {
                    Ok(PhysicalExpr::Call {
                        name: name.clone(),
                        fun: crate::functions::ScalarFnHandle(f.fun.clone()),
                        args: lowered_args,
                    })
                }
            }
        }
    }
}

fn schema_of(p: &LogicalPlan) -> SchemaRef {
    match p {
        LogicalPlan::TableScan { schema, .. } | LogicalPlan::EmptyRelation { schema } => {
            schema.clone()
        }
        LogicalPlan::Projection { input, .. }
        | LogicalPlan::Filter { input, .. }
        | LogicalPlan::Aggregate { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Limit { input, .. } => schema_of(input),
        LogicalPlan::Join { left, .. } => schema_of(left),
        LogicalPlan::Union { inputs } => schema_of(&inputs[0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DataType, Field, TableSchema};

    fn schema_ab() -> SchemaRef {
        Arc::new(TableSchema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, true),
        ]))
    }

    fn rows_ab() -> Vec<Row> {
        vec![
            Row::new(vec![Value::Int64(1), Value::Utf8("x".into())]),
            Row::new(vec![Value::Int64(2), Value::Utf8("y".into())]),
            Row::new(vec![Value::Int64(3), Value::Null]),
        ]
    }

    #[tokio::test]
    async fn sql_select_star_returns_all() {
        let ctx = SessionContext::new();
        ctx.register_mem_table("t", schema_ab(), rows_ab())
            .await
            .unwrap();
        let rows = ctx.sql("SELECT * FROM t").await.unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn sql_where_filters() {
        let ctx = SessionContext::new();
        ctx.register_mem_table("t", schema_ab(), rows_ab())
            .await
            .unwrap();
        let rows = ctx.sql("SELECT * FROM t WHERE a > 1").await.unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn sql_order_by_desc_and_limit() {
        let ctx = SessionContext::new();
        ctx.register_mem_table("t", schema_ab(), rows_ab())
            .await
            .unwrap();
        let rows = ctx
            .sql("SELECT a FROM t ORDER BY a DESC LIMIT 2")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values[0], Value::Int64(3));
    }

    #[tokio::test]
    async fn sql_aggregate_count_by_group() {
        let ctx = SessionContext::new();
        let rows = vec![
            Row::new(vec![Value::Utf8("x".into()), Value::Int64(10)]),
            Row::new(vec![Value::Utf8("x".into()), Value::Int64(20)]),
            Row::new(vec![Value::Utf8("y".into()), Value::Int64(5)]),
        ];
        let schema: SchemaRef = Arc::new(TableSchema::new(vec![
            Field::new("g", DataType::Utf8, false),
            Field::new("v", DataType::Int64, false),
        ]));
        ctx.register_mem_table("t", schema, rows).await.unwrap();
        let out = ctx
            .sql("SELECT g, count(v) FROM t GROUP BY g ORDER BY g")
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn dataframe_filter_then_project() {
        let ctx = SessionContext::new();
        ctx.register_mem_table("t", schema_ab(), rows_ab())
            .await
            .unwrap();
        let df = ctx
            .table("t")
            .await
            .unwrap()
            .filter(LogicalExpr::col("a").gt(LogicalExpr::lit(1)))
            .select(vec![LogicalExpr::col("a")]);
        let rows = ctx.execute_plan(&df.plan).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values.len(), 1);
    }
}
