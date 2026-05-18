// SPDX-License-Identifier: AGPL-3.0-or-later
//! LogicalPlan — declarative operator tree (Scan, Projection, Filter, Limit, Aggregate).
//!
//! Mirrors apache/datafusion datafusion-expr/src/logical_plan/plan.rs `LogicalPlan` enum
//! (subset of operators shipped here).

use crate::engine::datafusion::error::{DataFusionError, DfResult};
use crate::engine::datafusion::expr::Expr;
use crate::engine::datafusion::tenant::{default_tenant_id, validate_tenant_id};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateFunc {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

impl AggregateFunc {
    pub const fn name(self) -> &'static str {
        match self {
            AggregateFunc::Count => "COUNT",
            AggregateFunc::Sum => "SUM",
            AggregateFunc::Min => "MIN",
            AggregateFunc::Max => "MAX",
            AggregateFunc::Avg => "AVG",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateExpr {
    pub func: AggregateFunc,
    /// None for COUNT(*).
    pub column: Option<String>,
    pub output_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LogicalPlan {
    Scan {
        table_name: String,
        projection: Option<Vec<String>>,
        tenant_id: String,
    },
    Projection {
        input: Box<LogicalPlan>,
        columns: Vec<String>,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr,
    },
    Limit {
        input: Box<LogicalPlan>,
        skip: usize,
        fetch: Option<usize>,
    },
    Aggregate {
        input: Box<LogicalPlan>,
        group_by: Vec<String>,
        aggregates: Vec<AggregateExpr>,
    },
}

impl LogicalPlan {
    pub fn scan(table_name: impl Into<String>) -> Self {
        LogicalPlan::Scan {
            table_name: table_name.into(),
            projection: None,
            tenant_id: default_tenant_id(),
        }
    }

    pub fn scan_for_tenant(table_name: impl Into<String>, tenant: impl Into<String>) -> Self {
        LogicalPlan::Scan {
            table_name: table_name.into(),
            projection: None,
            tenant_id: tenant.into(),
        }
    }

    pub fn project(self, columns: Vec<String>) -> Self {
        LogicalPlan::Projection {
            input: Box::new(self),
            columns,
        }
    }

    pub fn filter(self, predicate: Expr) -> Self {
        LogicalPlan::Filter {
            input: Box::new(self),
            predicate,
        }
    }

    pub fn limit(self, skip: usize, fetch: Option<usize>) -> Self {
        LogicalPlan::Limit {
            input: Box::new(self),
            skip,
            fetch,
        }
    }

    pub fn aggregate(self, group_by: Vec<String>, aggregates: Vec<AggregateExpr>) -> Self {
        LogicalPlan::Aggregate {
            input: Box::new(self),
            group_by,
            aggregates,
        }
    }

    /// Children of this plan node.
    pub fn children(&self) -> Vec<&LogicalPlan> {
        match self {
            LogicalPlan::Scan { .. } => Vec::new(),
            LogicalPlan::Projection { input, .. }
            | LogicalPlan::Filter { input, .. }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Aggregate { input, .. } => vec![input.as_ref()],
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            LogicalPlan::Scan { .. } => "Scan",
            LogicalPlan::Projection { .. } => "Projection",
            LogicalPlan::Filter { .. } => "Filter",
            LogicalPlan::Limit { .. } => "Limit",
            LogicalPlan::Aggregate { .. } => "Aggregate",
        }
    }

    /// Tree depth — Scan = 1.
    pub fn depth(&self) -> usize {
        match self {
            LogicalPlan::Scan { .. } => 1,
            _ => 1 + self.children().iter().map(|c| c.depth()).max().unwrap_or(0),
        }
    }

    /// Walk the plan and collect tenant_ids found on every Scan leaf.
    /// All Scan leaves must share the same tenant_id.
    pub fn tenant_id(&self) -> DfResult<&str> {
        match self {
            LogicalPlan::Scan { tenant_id, .. } => Ok(tenant_id.as_str()),
            other => {
                let mut iter = other.children().into_iter();
                let first_tenant = iter
                    .next()
                    .ok_or_else(|| DataFusionError::Plan("plan has no input".into()))?
                    .tenant_id()?;
                for child in iter {
                    if child.tenant_id()? != first_tenant {
                        return Err(DataFusionError::Plan(
                            "plan mixes tenants — cross-tenant query forbidden".into(),
                        ));
                    }
                }
                Ok(first_tenant)
            }
        }
    }

    /// Validate the plan: tenant_id valid + every limit has a fetch≥0 +
    /// aggregate uses valid columns syntactically (real schema check happens
    /// at execution time).
    pub fn validate(&self) -> DfResult<()> {
        let tenant = self.tenant_id()?;
        validate_tenant_id(tenant)?;
        match self {
            LogicalPlan::Scan { table_name, .. } => {
                if table_name.is_empty() {
                    return Err(DataFusionError::Plan("scan table_name empty".into()));
                }
            }
            LogicalPlan::Projection { input, columns } => {
                if columns.is_empty() {
                    return Err(DataFusionError::Plan("projection must select ≥ 1 column".into()));
                }
                input.validate()?;
            }
            LogicalPlan::Filter { input, .. } => input.validate()?,
            LogicalPlan::Limit { input, fetch, .. } => {
                if let Some(0) = fetch {
                    return Err(DataFusionError::Plan(
                        "limit fetch=0 makes the plan return nothing — likely a bug".into(),
                    ));
                }
                input.validate()?;
            }
            LogicalPlan::Aggregate { input, aggregates, .. } => {
                if aggregates.is_empty() {
                    return Err(DataFusionError::Plan(
                        "aggregate must compute ≥ 1 aggregate function".into(),
                    ));
                }
                for a in aggregates {
                    if a.output_name.is_empty() {
                        return Err(DataFusionError::Plan(
                            "aggregate output_name must not be empty".into(),
                        ));
                    }
                    if a.func != AggregateFunc::Count && a.column.is_none() {
                        return Err(DataFusionError::Plan(format!(
                            "aggregate {} requires a column",
                            a.func.name()
                        )));
                    }
                }
                input.validate()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::datafusion::batch::Value;

    // ── AggregateFunc ─────────────────────────────────────────────────────────

    #[test]
    fn agg_func_names() {
        // citation: SQL standard aggregate function names
        assert_eq!(AggregateFunc::Count.name(), "COUNT");
        assert_eq!(AggregateFunc::Sum.name(), "SUM");
        assert_eq!(AggregateFunc::Min.name(), "MIN");
        assert_eq!(AggregateFunc::Max.name(), "MAX");
        assert_eq!(AggregateFunc::Avg.name(), "AVG");
    }

    // ── Plan constructors / fluent ────────────────────────────────────────────

    #[test]
    fn scan_default_tenant() {
        let p = LogicalPlan::scan("users");
        if let LogicalPlan::Scan { tenant_id, .. } = p {
            assert_eq!(tenant_id, "default");
        } else {
            panic!("expected Scan");
        }
    }

    #[test]
    fn scan_for_tenant_records_tenant() {
        let p = LogicalPlan::scan_for_tenant("users", "acme");
        assert_eq!(p.tenant_id().unwrap(), "acme");
    }

    #[test]
    fn fluent_chain_filter_project_limit() {
        let p = LogicalPlan::scan("users")
            .filter(Expr::col("age").gt(Expr::lit(Value::Int64(18))))
            .project(vec!["name".into()])
            .limit(0, Some(10));
        assert_eq!(p.depth(), 4);
        assert_eq!(p.name(), "Limit");
    }

    // ── name + depth ──────────────────────────────────────────────────────────

    #[test]
    fn name_for_each_variant() {
        assert_eq!(LogicalPlan::scan("t").name(), "Scan");
        assert_eq!(
            LogicalPlan::scan("t").project(vec!["x".into()]).name(),
            "Projection"
        );
        assert_eq!(
            LogicalPlan::scan("t").filter(Expr::lit(Value::Bool(true))).name(),
            "Filter"
        );
        assert_eq!(
            LogicalPlan::scan("t").limit(0, Some(5)).name(),
            "Limit"
        );
        assert_eq!(
            LogicalPlan::scan("t")
                .aggregate(
                    vec![],
                    vec![AggregateExpr {
                        func: AggregateFunc::Count,
                        column: None,
                        output_name: "n".into(),
                    }],
                )
                .name(),
            "Aggregate"
        );
    }

    #[test]
    fn depth_scan_is_1() {
        assert_eq!(LogicalPlan::scan("t").depth(), 1);
    }

    #[test]
    fn depth_increments_per_layer() {
        let p = LogicalPlan::scan("t").project(vec!["x".into()]);
        assert_eq!(p.depth(), 2);
    }

    // ── tenant_id walk ────────────────────────────────────────────────────────

    #[test]
    fn tenant_id_propagates_through_tree() {
        let p = LogicalPlan::scan_for_tenant("t", "acme")
            .filter(Expr::lit(Value::Bool(true)))
            .limit(0, Some(5));
        assert_eq!(p.tenant_id().unwrap(), "acme");
    }

    // ── validate — happy paths ────────────────────────────────────────────────

    #[test]
    fn validate_simple_scan_ok() {
        assert!(LogicalPlan::scan("t").validate().is_ok());
    }

    #[test]
    fn validate_filter_project_limit_ok() {
        let p = LogicalPlan::scan("t")
            .filter(Expr::col("a").gt(Expr::lit(Value::Int64(0))))
            .project(vec!["a".into()])
            .limit(0, Some(10));
        assert!(p.validate().is_ok());
    }

    #[test]
    fn validate_aggregate_count_star_ok() {
        let p = LogicalPlan::scan("t").aggregate(
            vec![],
            vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "n".into(),
            }],
        );
        assert!(p.validate().is_ok());
    }

    #[test]
    fn validate_aggregate_sum_with_col_ok() {
        let p = LogicalPlan::scan("t").aggregate(
            vec!["dept".into()],
            vec![AggregateExpr {
                func: AggregateFunc::Sum,
                column: Some("salary".into()),
                output_name: "total".into(),
            }],
        );
        assert!(p.validate().is_ok());
    }

    // ── validate — failures ───────────────────────────────────────────────────

    #[test]
    fn validate_empty_scan_table_err() {
        let p = LogicalPlan::Scan {
            table_name: "".into(),
            projection: None,
            tenant_id: "default".into(),
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_empty_projection_err() {
        let p = LogicalPlan::scan("t").project(vec![]);
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_zero_fetch_err() {
        let p = LogicalPlan::scan("t").limit(0, Some(0));
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_aggregate_no_aggregates_err() {
        let p = LogicalPlan::scan("t").aggregate(vec!["x".into()], vec![]);
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_aggregate_sum_without_column_err() {
        let p = LogicalPlan::scan("t").aggregate(
            vec![],
            vec![AggregateExpr {
                func: AggregateFunc::Sum,
                column: None,
                output_name: "n".into(),
            }],
        );
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_aggregate_empty_output_name_err() {
        let p = LogicalPlan::scan("t").aggregate(
            vec![],
            vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "".into(),
            }],
        );
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_invalid_tenant_err() {
        let p = LogicalPlan::scan_for_tenant("t", "BAD");
        assert!(p.validate().is_err());
    }

    // ── tenant mixing ─────────────────────────────────────────────────────────
    // (synthesize a multi-input via a hypothetical join; here we just demo
    // that tenant_id() walks correctly through a chain)

    #[test]
    fn tenant_id_unfiltered_path() {
        let p = LogicalPlan::scan_for_tenant("t", "burak").project(vec!["a".into()]);
        assert_eq!(p.tenant_id().unwrap(), "burak");
    }

    // ── serde ─────────────────────────────────────────────────────────────────

    #[test]
    fn plan_serde_roundtrip() {
        let p = LogicalPlan::scan_for_tenant("users", "acme")
            .filter(Expr::col("age").gt(Expr::lit(Value::Int64(18))))
            .project(vec!["name".into()])
            .limit(0, Some(10));
        let j = serde_json::to_string(&p).unwrap();
        let back: LogicalPlan = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }
}
