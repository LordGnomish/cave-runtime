// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PromQL Abstract Syntax Tree.

use crate::model::LabelMatcher;

/// All possible PromQL expressions.
#[derive(Debug, Clone)]
pub enum Expr {
    NumberLiteral(f64),
    StringLiteral(String),

    /// `metric_name{label="value"}` — instant vector selector.
    VectorSelector(VectorSelector),

    /// `metric_name{...}[5m]` — range vector selector.
    MatrixSelector(MatrixSelector),

    /// Subquery: `expr[5m:1m]`.
    Subquery(Subquery),

    /// Unary minus: `-expr`.
    Unary(UnaryExpr),

    /// Binary operation: `lhs op rhs`.
    Binary(BinaryExpr),

    /// Aggregation: `sum by(job) (expr)`.
    Aggregate(AggregateExpr),

    /// Function call: `rate(expr[5m])`.
    Call(CallExpr),
}

// ─── Vector selector ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VectorSelector {
    pub name: Option<String>,          // metric name shorthand
    pub matchers: Vec<LabelMatcher>,
    pub offset: Option<i64>,           // milliseconds
    pub at: Option<i64>,               // unix ms, from @ modifier
}

// ─── Matrix selector ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MatrixSelector {
    pub selector: VectorSelector,
    pub range_ms: i64,
    pub offset: Option<i64>,
    pub at: Option<i64>,
}

// ─── Subquery ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Subquery {
    pub expr: Box<Expr>,
    pub range_ms: i64,
    pub step_ms: i64,
    pub offset: Option<i64>,
    pub at: Option<i64>,
}

// ─── Unary ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct UnaryExpr {
    pub expr: Box<Expr>,
}

// ─── Binary ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum BinaryOp {
    Add, Sub, Mul, Div, Mod, Pow,
    Eql, Neq, Lss, Lte, Gtr, Gte,
    And, Or, Unless, Atan2,
}

impl BinaryOp {
    pub fn apply(self, l: f64, r: f64) -> f64 {
        match self {
            Self::Add   => l + r,
            Self::Sub   => l - r,
            Self::Mul   => l * r,
            Self::Div   => l / r,
            Self::Mod   => l % r,
            Self::Pow   => l.powf(r),
            Self::Eql   => if l == r { 1.0 } else { 0.0 },
            Self::Neq   => if l != r { 1.0 } else { 0.0 },
            Self::Lss   => if l <  r { 1.0 } else { 0.0 },
            Self::Lte   => if l <= r { 1.0 } else { 0.0 },
            Self::Gtr   => if l >  r { 1.0 } else { 0.0 },
            Self::Gte   => if l >= r { 1.0 } else { 0.0 },
            Self::And | Self::Or | Self::Unless | Self::Atan2 => l.atan2(r),
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, Self::Eql | Self::Neq | Self::Lss | Self::Lte | Self::Gtr | Self::Gte)
    }

    pub fn is_set_op(self) -> bool {
        matches!(self, Self::And | Self::Or | Self::Unless)
    }
}

#[derive(Debug, Clone)]
pub struct BinaryExpr {
    pub op: BinaryOp,
    pub lhs: Box<Expr>,
    pub rhs: Box<Expr>,
    pub matching: Option<VectorMatching>,
    pub return_bool: bool,
}

#[derive(Debug, Clone)]
pub struct VectorMatching {
    pub card: MatchCardinality,
    pub on: bool,        // true = on(...), false = ignoring(...)
    pub labels: Vec<String>,
    pub include: Vec<String>, // group_left/group_right include labels
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchCardinality {
    OneToOne,
    ManyToOne,
    OneToMany,
}

// ─── Aggregate ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggregateOp {
    Sum, Min, Max, Avg, Count, Stddev, Stdvar,
    Quantile, Topk, Bottomk, CountValues, Group,
}

#[derive(Debug, Clone)]
pub struct AggregateExpr {
    pub op: AggregateOp,
    pub expr: Box<Expr>,
    pub param: Option<Box<Expr>>, // quantile Q, topk K, bottomk K, count_values label
    pub grouping: Grouping,
}

#[derive(Debug, Clone, Default)]
pub struct Grouping {
    pub without: bool,
    pub labels: Vec<String>,
}

// ─── Function call ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CallExpr {
    pub func: String,
    pub args: Vec<Expr>,
}
