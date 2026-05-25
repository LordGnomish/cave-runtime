// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! LogicalExpr — DataFusion expression AST.
//!
//! Upstream: `crates/datafusion-expr/src/expr.rs`
//!
//! DataFusion's logical expression tree carries column references,
//! literals, binary ops, function calls, comparisons, IS-NULL, CAST,
//! and aggregates. The MVP carries the same enum shape so a SQL
//! planner can lower into PhysicalExpr without re-shaping.

use crate::row::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BinaryOp {
    // Arithmetic
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    // Comparison
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    // Logical
    And,
    Or,
}

impl BinaryOp {
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Eq | Self::NotEq | Self::Lt | Self::LtEq | Self::Gt | Self::GtEq
        )
    }

    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            Self::Plus | Self::Minus | Self::Multiply | Self::Divide | Self::Modulo
        )
    }

    pub fn is_logical(self) -> bool {
        matches!(self, Self::And | Self::Or)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogicalExpr {
    Column {
        name: String,
    },
    Literal {
        value: Value,
    },
    BinaryOp {
        op: BinaryOp,
        left: Box<LogicalExpr>,
        right: Box<LogicalExpr>,
    },
    Not {
        expr: Box<LogicalExpr>,
    },
    IsNull {
        expr: Box<LogicalExpr>,
    },
    IsNotNull {
        expr: Box<LogicalExpr>,
    },
    Cast {
        expr: Box<LogicalExpr>,
        to: crate::schema::DataType,
    },
    /// `name(arg, arg, …)` — scalar or aggregate; the planner decides
    /// from the FunctionRegistry which one it is.
    Function {
        name: String,
        args: Vec<LogicalExpr>,
    },
    /// SQL `AS` alias — rewrites the output column name.
    Alias {
        expr: Box<LogicalExpr>,
        alias: String,
    },
}

impl LogicalExpr {
    pub fn col(name: impl Into<String>) -> Self {
        Self::Column { name: name.into() }
    }

    pub fn lit(v: impl Into<Value>) -> Self {
        Self::Literal { value: v.into() }
    }

    pub fn binary(left: Self, op: BinaryOp, right: Self) -> Self {
        Self::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    pub fn eq(self, other: Self) -> Self {
        Self::binary(self, BinaryOp::Eq, other)
    }
    pub fn lt(self, other: Self) -> Self {
        Self::binary(self, BinaryOp::Lt, other)
    }
    pub fn gt(self, other: Self) -> Self {
        Self::binary(self, BinaryOp::Gt, other)
    }
    pub fn and(self, other: Self) -> Self {
        Self::binary(self, BinaryOp::And, other)
    }
    pub fn or(self, other: Self) -> Self {
        Self::binary(self, BinaryOp::Or, other)
    }

    pub fn alias(self, alias: impl Into<String>) -> Self {
        Self::Alias {
            expr: Box::new(self),
            alias: alias.into(),
        }
    }

    pub fn is_null(self) -> Self {
        Self::IsNull {
            expr: Box::new(self),
        }
    }

    pub fn cast_to(self, to: crate::schema::DataType) -> Self {
        Self::Cast {
            expr: Box::new(self),
            to,
        }
    }

    /// The output column name when this expression appears in a SELECT
    /// list. `Alias` wins; otherwise mirror DataFusion's default
    /// behavior of using the column name or the function name.
    pub fn output_name(&self) -> String {
        match self {
            Self::Alias { alias, .. } => alias.clone(),
            Self::Column { name } => name.clone(),
            Self::Literal { .. } => "literal".to_string(),
            Self::Function { name, .. } => name.clone(),
            Self::BinaryOp { op, .. } => format!("{:?}", op).to_lowercase(),
            Self::Not { .. } => "not".to_string(),
            Self::IsNull { .. } => "is_null".to_string(),
            Self::IsNotNull { .. } => "is_not_null".to_string(),
            Self::Cast { to, .. } => format!("cast_{:?}", to).to_lowercase(),
        }
    }

    /// Collect all unique `Column { name }` references in this expression
    /// tree — used by the planner to compute the projection set.
    pub fn collect_columns(&self) -> Vec<String> {
        fn walk(e: &LogicalExpr, out: &mut Vec<String>) {
            match e {
                LogicalExpr::Column { name } => {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
                LogicalExpr::Literal { .. } => {}
                LogicalExpr::BinaryOp { left, right, .. } => {
                    walk(left, out);
                    walk(right, out);
                }
                LogicalExpr::Not { expr }
                | LogicalExpr::IsNull { expr }
                | LogicalExpr::IsNotNull { expr }
                | LogicalExpr::Cast { expr, .. }
                | LogicalExpr::Alias { expr, .. } => walk(expr, out),
                LogicalExpr::Function { args, .. } => {
                    for a in args {
                        walk(a, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int64(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Int32(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float64(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Utf8(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Utf8(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_eq_chain_builds_tree() {
        let e = LogicalExpr::col("a").eq(LogicalExpr::lit(7));
        match e {
            LogicalExpr::BinaryOp {
                op: BinaryOp::Eq, ..
            } => {}
            _ => panic!("expected Eq"),
        }
    }

    #[test]
    fn collect_columns_unique() {
        let e = LogicalExpr::col("a")
            .eq(LogicalExpr::lit(1))
            .and(LogicalExpr::col("b").lt(LogicalExpr::col("a")));
        let cols = e.collect_columns();
        assert_eq!(cols, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn alias_overrides_output_name() {
        let e = LogicalExpr::col("a").alias("aa");
        assert_eq!(e.output_name(), "aa");
    }

    #[test]
    fn op_classifications() {
        assert!(BinaryOp::Eq.is_comparison());
        assert!(BinaryOp::Plus.is_arithmetic());
        assert!(BinaryOp::And.is_logical());
        assert!(!BinaryOp::Or.is_arithmetic());
    }

    #[test]
    fn cast_alias_round_trip() {
        let e = LogicalExpr::col("a")
            .cast_to(crate::schema::DataType::Float64)
            .alias("af");
        assert_eq!(e.output_name(), "af");
    }
}
