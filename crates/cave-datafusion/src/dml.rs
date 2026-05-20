// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! DML statement planner — `crates/datafusion-sql/src/planner/dml.rs`.
//!
//! Adds `INSERT INTO`, `UPDATE` and `DELETE FROM` planning on top of the
//! existing `LogicalPlan` enum. The MVP scopes the supported syntax to
//! the shape needed for cave-iceberg integration:
//!
//! * `INSERT INTO <table> (col1, col2, ...) VALUES (...), (...)` —
//!   single-row and multi-row value lists.
//! * `INSERT INTO <table> SELECT ...` — pipes an arbitrary
//!   `LogicalPlan` into the destination table.
//! * `UPDATE <table> SET col = expr [, ...] [WHERE predicate]` — the
//!   `assignments` vector preserves order; the predicate may reference
//!   any column on the target table.
//! * `DELETE FROM <table> [WHERE predicate]` — the bareword form
//!   (without `WHERE`) deletes every row.
//!
//! The DML node types live alongside `LogicalPlan` so the rest of the
//! optimizer/executor can treat them as ordinary plan inputs.

use crate::logical_expr::LogicalExpr;
use crate::logical_plan::LogicalPlan;

/// Source of rows for `INSERT`.
#[derive(Debug, Clone, PartialEq)]
pub enum InsertSource {
    /// Row literals — each inner vec is one row, aligned with `columns`.
    Values(Vec<Vec<LogicalExpr>>),
    /// Pipe rows from an upstream `LogicalPlan`. `SELECT * FROM other`.
    Plan(Box<LogicalPlan>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum DmlPlan {
    Insert {
        table_name: String,
        columns: Vec<String>,
        source: InsertSource,
    },
    Update {
        table_name: String,
        /// (column-name, expression) pairs in declaration order.
        assignments: Vec<(String, LogicalExpr)>,
        predicate: Option<LogicalExpr>,
    },
    Delete {
        table_name: String,
        predicate: Option<LogicalExpr>,
    },
}

impl DmlPlan {
    pub fn statement_kind(&self) -> &'static str {
        match self {
            DmlPlan::Insert { .. } => "insert",
            DmlPlan::Update { .. } => "update",
            DmlPlan::Delete { .. } => "delete",
        }
    }

    pub fn target_table(&self) -> &str {
        match self {
            DmlPlan::Insert { table_name, .. }
            | DmlPlan::Update { table_name, .. }
            | DmlPlan::Delete { table_name, .. } => table_name,
        }
    }

    /// Validate that an Insert's value rows are all of the column-count length.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            DmlPlan::Insert {
                columns,
                source: InsertSource::Values(rows),
                ..
            } => {
                for (i, row) in rows.iter().enumerate() {
                    if row.len() != columns.len() {
                        return Err(format!(
                            "row {}: expected {} values, got {}",
                            i,
                            columns.len(),
                            row.len()
                        ));
                    }
                }
                Ok(())
            }
            DmlPlan::Insert {
                source: InsertSource::Plan(_),
                ..
            } => Ok(()),
            DmlPlan::Update { assignments, .. } => {
                if assignments.is_empty() {
                    return Err("UPDATE requires at least one assignment".into());
                }
                Ok(())
            }
            DmlPlan::Delete { .. } => Ok(()),
        }
    }

    /// Number of rows in the materialized portion (Insert::Values), or
    /// `None` if it's plan-driven / row-count-unknown.
    pub fn row_count_hint(&self) -> Option<usize> {
        match self {
            DmlPlan::Insert {
                source: InsertSource::Values(rows),
                ..
            } => Some(rows.len()),
            _ => None,
        }
    }
}

/// Builders that match upstream's `SqlToRel::sql_statement_to_plan_dml`
/// handlers but skip the full sqlparser dependency — callers (the
/// cave SQL parser) supply already-typed `LogicalExpr` values.
pub fn insert_values(
    table: impl Into<String>,
    columns: Vec<String>,
    rows: Vec<Vec<LogicalExpr>>,
) -> DmlPlan {
    DmlPlan::Insert {
        table_name: table.into(),
        columns,
        source: InsertSource::Values(rows),
    }
}

pub fn insert_from_plan(
    table: impl Into<String>,
    columns: Vec<String>,
    plan: LogicalPlan,
) -> DmlPlan {
    DmlPlan::Insert {
        table_name: table.into(),
        columns,
        source: InsertSource::Plan(Box::new(plan)),
    }
}

pub fn update(
    table: impl Into<String>,
    assignments: Vec<(String, LogicalExpr)>,
    predicate: Option<LogicalExpr>,
) -> DmlPlan {
    DmlPlan::Update {
        table_name: table.into(),
        assignments,
        predicate,
    }
}

pub fn delete(table: impl Into<String>, predicate: Option<LogicalExpr>) -> DmlPlan {
    DmlPlan::Delete {
        table_name: table.into(),
        predicate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logical_expr::LogicalExpr;
    use crate::row::Value;

    fn lit_int(v: i64) -> LogicalExpr {
        LogicalExpr::lit(Value::Int64(v))
    }

    fn lit_str(s: &str) -> LogicalExpr {
        LogicalExpr::lit(Value::Utf8(s.into()))
    }

    #[test]
    fn insert_values_round_trip_validates() {
        let p = insert_values(
            "people",
            vec!["id".into(), "name".into()],
            vec![
                vec![lit_int(1), lit_str("alice")],
                vec![lit_int(2), lit_str("bob")],
            ],
        );
        assert_eq!(p.statement_kind(), "insert");
        assert_eq!(p.target_table(), "people");
        assert_eq!(p.row_count_hint(), Some(2));
        p.validate().unwrap();
    }

    #[test]
    fn insert_with_wrong_arity_row_is_rejected() {
        let p = insert_values(
            "people",
            vec!["id".into(), "name".into()],
            vec![vec![lit_int(1)]], // missing name
        );
        let err = p.validate().unwrap_err();
        assert!(err.contains("expected 2 values"));
    }

    #[test]
    fn insert_from_plan_carries_inner_logical_plan() {
        let inner = LogicalPlan::EmptyRelation {
            schema: std::sync::Arc::new(crate::schema::TableSchema::default()),
        };
        let p = insert_from_plan("dst", vec!["id".into()], inner);
        match &p {
            DmlPlan::Insert {
                source: InsertSource::Plan(_),
                ..
            } => {}
            _ => panic!("expected Insert::Plan"),
        }
        assert_eq!(p.row_count_hint(), None);
        p.validate().unwrap();
    }

    #[test]
    fn update_requires_at_least_one_assignment() {
        let p = update("people", vec![], None);
        assert!(p.validate().is_err());
        let p = update(
            "people",
            vec![("name".into(), lit_str("eve"))],
            Some(LogicalExpr::col("id").eq(lit_int(1))),
        );
        p.validate().unwrap();
        assert_eq!(p.target_table(), "people");
        assert_eq!(p.statement_kind(), "update");
    }

    #[test]
    fn delete_without_where_is_valid() {
        let p = delete("people", None);
        p.validate().unwrap();
        assert_eq!(p.statement_kind(), "delete");
    }

    #[test]
    fn delete_with_predicate_round_trips() {
        let pred = LogicalExpr::col("id").eq(lit_int(7));
        let p = delete("people", Some(pred.clone()));
        if let DmlPlan::Delete { predicate, .. } = &p {
            assert_eq!(predicate.as_ref().unwrap(), &pred);
        } else {
            panic!("expected Delete");
        }
    }

    #[test]
    fn dml_plan_target_table_returns_consistent_string() {
        let i = insert_values("a", vec![], vec![]);
        let u = update("b", vec![("x".into(), lit_int(1))], None);
        let d = delete("c", None);
        assert_eq!(i.target_table(), "a");
        assert_eq!(u.target_table(), "b");
        assert_eq!(d.target_table(), "c");
    }

    #[test]
    fn statement_kind_matches_sql_keyword() {
        assert_eq!(
            insert_values("t", vec![], vec![]).statement_kind(),
            "insert"
        );
        assert_eq!(
            update("t", vec![("x".into(), lit_int(1))], None).statement_kind(),
            "update"
        );
        assert_eq!(delete("t", None).statement_kind(), "delete");
    }

    #[test]
    fn multi_row_insert_row_count_hint() {
        let p = insert_values(
            "t",
            vec!["x".into()],
            (0..5).map(|i| vec![lit_int(i)]).collect(),
        );
        assert_eq!(p.row_count_hint(), Some(5));
    }

    #[test]
    fn update_assignments_preserve_order() {
        let p = update(
            "t",
            vec![
                ("a".into(), lit_int(1)),
                ("b".into(), lit_int(2)),
                ("c".into(), lit_int(3)),
            ],
            None,
        );
        if let DmlPlan::Update { assignments, .. } = &p {
            let names: Vec<&str> = assignments.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, vec!["a", "b", "c"]);
        } else {
            panic!("expected Update");
        }
    }
}
