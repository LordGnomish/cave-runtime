// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! DataFrame — DataFusion's fluent plan builder.
//!
//! Upstream: `crates/datafusion/src/dataframe/mod.rs`
//!
//! Each chainable method appends a LogicalPlan node above the current
//! one and returns a new DataFrame. The MVP surface covers the most
//! common verbs (`select / filter / aggregate / sort / limit / join`).

use crate::error::Result;
use crate::logical_expr::LogicalExpr;
use crate::logical_plan::{JoinKind, LogicalPlan, SortKey};
use crate::schema::SchemaRef;

#[derive(Debug, Clone)]
pub struct DataFrame {
    pub plan: LogicalPlan,
}

impl DataFrame {
    pub fn from_plan(plan: LogicalPlan) -> Self {
        Self { plan }
    }

    pub fn schema(&self) -> SchemaRef {
        match &self.plan {
            LogicalPlan::TableScan { schema, .. } => schema.clone(),
            LogicalPlan::EmptyRelation { schema } => schema.clone(),
            LogicalPlan::Projection { input, .. }
            | LogicalPlan::Filter { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Limit { input, .. } => {
                // For projection/aggregate the column list changes, but the
                // MVP keeps the input schema for binding purposes — the
                // executor's projection determines the output shape.
                Self::from_plan(*input.clone()).schema()
            }
            LogicalPlan::Join { left, .. } => Self::from_plan(*left.clone()).schema(),
            LogicalPlan::Union { inputs } => Self::from_plan(inputs[0].clone()).schema(),
        }
    }

    pub fn select(self, expressions: Vec<LogicalExpr>) -> Self {
        Self::from_plan(LogicalPlan::Projection {
            expressions,
            input: Box::new(self.plan),
        })
    }

    pub fn filter(self, predicate: LogicalExpr) -> Self {
        Self::from_plan(LogicalPlan::Filter {
            predicate,
            input: Box::new(self.plan),
        })
    }

    pub fn aggregate(self, group_by: Vec<LogicalExpr>, aggr: Vec<LogicalExpr>) -> Self {
        Self::from_plan(LogicalPlan::Aggregate {
            group_by,
            aggr,
            input: Box::new(self.plan),
        })
    }

    pub fn sort(self, keys: Vec<SortKey>) -> Self {
        Self::from_plan(LogicalPlan::Sort {
            keys,
            input: Box::new(self.plan),
        })
    }

    pub fn limit(self, skip: usize, fetch: Option<usize>) -> Self {
        Self::from_plan(LogicalPlan::Limit {
            skip,
            fetch,
            input: Box::new(self.plan),
        })
    }

    pub fn join(
        self,
        right: DataFrame,
        kind: JoinKind,
        on: Vec<(LogicalExpr, LogicalExpr)>,
    ) -> Self {
        Self::from_plan(LogicalPlan::Join {
            kind,
            on,
            left: Box::new(self.plan),
            right: Box::new(right.plan),
        })
    }

    pub fn union(self, other: DataFrame) -> Result<Self> {
        let inputs = vec![self.plan, other.plan];
        Ok(Self::from_plan(LogicalPlan::Union { inputs }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{DataType, Field, TableSchema};
    use std::sync::Arc;

    fn scan() -> DataFrame {
        DataFrame::from_plan(LogicalPlan::TableScan {
            table_name: "T".into(),
            schema: Arc::new(TableSchema::new(vec![Field::new(
                "a",
                DataType::Int64,
                false,
            )])),
            projection: None,
            filters: vec![],
        })
    }

    #[test]
    fn select_filter_sort_chain_grows_plan() {
        let df = scan()
            .filter(LogicalExpr::col("a").gt(LogicalExpr::lit(0)))
            .select(vec![LogicalExpr::col("a")])
            .sort(vec![SortKey::asc(LogicalExpr::col("a"))])
            .limit(0, Some(10));
        assert_eq!(df.plan.depth(), 5);
    }

    #[test]
    fn join_builds_join_node() {
        let l = scan();
        let r = scan();
        let df = l.join(
            r,
            JoinKind::Inner,
            vec![(LogicalExpr::col("a"), LogicalExpr::col("a"))],
        );
        match df.plan {
            LogicalPlan::Join {
                kind: JoinKind::Inner,
                ..
            } => {}
            _ => panic!("expected Join"),
        }
    }
}
