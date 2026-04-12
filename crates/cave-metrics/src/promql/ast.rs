//! PromQL abstract syntax tree.

#![allow(dead_code)]

use crate::model::LabelMatcher;

#[derive(Debug, Clone)]
pub enum Expr {
    NumberLiteral(f64),
    StringLiteral(String),
    VectorSelector {
        name: Option<String>,
        matchers: Vec<LabelMatcher>,
        offset: Option<i64>,
        at: Option<i64>,
    },
    MatrixSelector {
        selector: Box<Expr>,
        range_ms: i64,
    },
    Subquery {
        expr: Box<Expr>,
        range_ms: i64,
        step_ms: i64,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        matching: VectorMatching,
        return_bool: bool,
    },
    Aggregate {
        op: AggregateOp,
        expr: Box<Expr>,
        param: Option<Box<Expr>>,
        grouping: Grouping,
    },
    Call {
        func: String,
        args: Vec<Expr>,
    },
    Paren(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eql,
    Neq,
    Lss,
    Gtr,
    Lte,
    Gte,
    And,
    Or,
    Unless,
    Atan2,
}

impl BinaryOp {
    /// Precedence: higher = tighter binding.
    pub fn precedence(self) -> u8 {
        match self {
            BinaryOp::Or => 1,
            BinaryOp::And | BinaryOp::Unless => 2,
            BinaryOp::Eql
            | BinaryOp::Neq
            | BinaryOp::Lss
            | BinaryOp::Gtr
            | BinaryOp::Lte
            | BinaryOp::Gte => 3,
            BinaryOp::Add | BinaryOp::Sub => 4,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Atan2 => 5,
            BinaryOp::Pow => 6,
        }
    }

    pub fn is_right_assoc(self) -> bool {
        matches!(self, BinaryOp::Pow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateOp {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    Stddev,
    Stdvar,
    Quantile,
    Topk,
    Bottomk,
    CountValues,
}

/// by/without grouping clause for aggregations.
#[derive(Debug, Clone, Default)]
pub struct Grouping {
    pub by: bool, // true = "by", false = "without"
    pub labels: Vec<String>,
    /// Whether a grouping clause was explicitly specified.
    /// When false, no by/without was given → aggregate everything into one group.
    pub specified: bool,
}

/// Vector matching configuration for binary ops.
#[derive(Debug, Clone, Default)]
pub struct VectorMatching {
    pub card: MatchingCard,
    pub labels: Vec<String>,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchingCard {
    #[default]
    OneToOne,
    ManyToOne,
    OneToMany,
}
