// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! LogicalPlan — DataFusion logical plan AST.
//!
//! Upstream: `crates/datafusion-expr/src/logical_plan/plan.rs`
//!
//! Each plan node carries its child (or children) and the operation.
//! The MVP supports the eight nodes the user spec calls out:
//! scan / filter / project / aggregate / sort / limit / join, plus
//! `EmptyRelation` and `Union` as cheap helpers. CTEs, subqueries,
//! recursive CTEs, and window functions are deferred (`scope_cuts`).

use crate::logical_expr::LogicalExpr;
use crate::schema::SchemaRef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Semi,
    Anti,
    Cross,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalPlan {
    /// Logical scan of a registered table.
    TableScan {
        table_name: String,
        schema: SchemaRef,
        /// Optional projection — the column-index list. None means all.
        projection: Option<Vec<usize>>,
        /// Optional filter predicates pushed down at plan time.
        filters: Vec<LogicalExpr>,
    },
    /// SELECT expression list (project).
    Projection {
        expressions: Vec<LogicalExpr>,
        input: Box<LogicalPlan>,
    },
    /// WHERE / HAVING / JOIN predicate.
    Filter {
        predicate: LogicalExpr,
        input: Box<LogicalPlan>,
    },
    /// GROUP BY + aggregate functions.
    Aggregate {
        group_by: Vec<LogicalExpr>,
        aggr: Vec<LogicalExpr>,
        input: Box<LogicalPlan>,
    },
    /// ORDER BY.
    Sort {
        keys: Vec<SortKey>,
        input: Box<LogicalPlan>,
    },
    /// LIMIT + optional OFFSET.
    Limit {
        skip: usize,
        fetch: Option<usize>,
        input: Box<LogicalPlan>,
    },
    /// JOIN (inner/outer/semi/anti).
    Join {
        kind: JoinKind,
        on: Vec<(LogicalExpr, LogicalExpr)>,
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
    },
    /// UNION ALL — children must have schema-compatible projections.
    Union { inputs: Vec<LogicalPlan> },
    /// Empty relation — yields no rows; useful for SELECT 1 or empty CTEs.
    EmptyRelation { schema: SchemaRef },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    pub expr: LogicalExpr,
    pub ascending: bool,
    pub nulls_first: bool,
}

impl SortKey {
    pub fn asc(expr: LogicalExpr) -> Self {
        Self { expr, ascending: true, nulls_first: true }
    }

    pub fn desc(expr: LogicalExpr) -> Self {
        Self { expr, ascending: false, nulls_first: false }
    }
}

impl LogicalPlan {
    /// Walk the plan and yield all distinct table names referenced by
    /// `TableScan` nodes. Used by the catalog binder + optimizer.
    pub fn table_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_table_names(&mut out);
        out
    }

    fn collect_table_names(&self, out: &mut Vec<String>) {
        match self {
            Self::TableScan { table_name, .. } => {
                if !out.contains(table_name) {
                    out.push(table_name.clone());
                }
            }
            Self::Projection { input, .. }
            | Self::Filter { input, .. }
            | Self::Aggregate { input, .. }
            | Self::Sort { input, .. }
            | Self::Limit { input, .. } => input.collect_table_names(out),
            Self::Join { left, right, .. } => {
                left.collect_table_names(out);
                right.collect_table_names(out);
            }
            Self::Union { inputs } => {
                for i in inputs {
                    i.collect_table_names(out);
                }
            }
            Self::EmptyRelation { .. } => {}
        }
    }

    /// Depth — useful for plan-tree printing + optimizer fuel.
    pub fn depth(&self) -> usize {
        match self {
            Self::TableScan { .. } | Self::EmptyRelation { .. } => 1,
            Self::Projection { input, .. }
            | Self::Filter { input, .. }
            | Self::Aggregate { input, .. }
            | Self::Sort { input, .. }
            | Self::Limit { input, .. } => 1 + input.depth(),
            Self::Join { left, right, .. } => 1 + left.depth().max(right.depth()),
            Self::Union { inputs } => {
                1 + inputs.iter().map(|i| i.depth()).max().unwrap_or(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DataType, Field, TableSchema};
    use std::sync::Arc;

    fn empty_schema() -> SchemaRef {
        Arc::new(TableSchema::new(vec![Field::new("a", DataType::Int64, false)]))
    }

    #[test]
    fn table_names_walks_join_tree() {
        let left = LogicalPlan::TableScan {
            table_name: "L".into(),
            schema: empty_schema(),
            projection: None,
            filters: vec![],
        };
        let right = LogicalPlan::TableScan {
            table_name: "R".into(),
            schema: empty_schema(),
            projection: None,
            filters: vec![],
        };
        let join = LogicalPlan::Join {
            kind: JoinKind::Inner,
            on: vec![],
            left: Box::new(left),
            right: Box::new(right),
        };
        let names = join.table_names();
        assert_eq!(names, vec!["L".to_string(), "R".to_string()]);
    }

    #[test]
    fn depth_accumulates_through_projection() {
        let scan = LogicalPlan::TableScan {
            table_name: "T".into(),
            schema: empty_schema(),
            projection: None,
            filters: vec![],
        };
        let p = LogicalPlan::Projection {
            expressions: vec![LogicalExpr::col("a")],
            input: Box::new(scan),
        };
        assert_eq!(p.depth(), 2);
    }

    #[test]
    fn sort_key_asc_defaults_nulls_first() {
        let k = SortKey::asc(LogicalExpr::col("a"));
        assert!(k.ascending);
        assert!(k.nulls_first);
    }

    #[test]
    fn depth_of_union_takes_max() {
        let s = || LogicalPlan::TableScan {
            table_name: "T".into(),
            schema: empty_schema(),
            projection: None,
            filters: vec![],
        };
        let p = LogicalPlan::Projection {
            expressions: vec![LogicalExpr::col("a")],
            input: Box::new(s()),
        };
        let u = LogicalPlan::Union { inputs: vec![s(), p] };
        assert_eq!(u.depth(), 3);
    }
}
