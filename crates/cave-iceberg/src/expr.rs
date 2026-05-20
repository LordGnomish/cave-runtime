// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg predicate expression AST.
//!
//! Upstream: `crates/iceberg/src/expr/predicate.rs` + `expr/term.rs`
//!
//! The MVP carries the predicate as a serializable AST. Evaluation
//! against concrete row values is implemented for the scalar `Equal /
//! NotEqual / Less / LessOrEqual / Greater / GreaterOrEqual` operators
//! over JSON values; manifest-time partition pruning uses the same
//! evaluator. Row-group-level Parquet pushdown is deferred — the
//! evaluator returns `Some(bool)` when the bound passes/fails and
//! `None` when undecidable.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// A column reference — Iceberg's `Reference` (field-id resolved at bind time).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Reference {
    pub name: String,
}

impl Reference {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Term {
    Reference(Reference),
    Literal(Json),
}

impl Term {
    pub fn ref_col(name: impl Into<String>) -> Self {
        Self::Reference(Reference::new(name))
    }

    pub fn lit(v: impl Into<Json>) -> Self {
        Self::Literal(v.into())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Predicate {
    True,
    False,
    Compare { op: CompareOp, left: Term, right: Term },
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    pub fn eq(left: Term, right: Term) -> Self {
        Self::Compare { op: CompareOp::Equal, left, right }
    }
    pub fn ne(left: Term, right: Term) -> Self {
        Self::Compare { op: CompareOp::NotEqual, left, right }
    }
    pub fn lt(left: Term, right: Term) -> Self {
        Self::Compare { op: CompareOp::Less, left, right }
    }
    pub fn le(left: Term, right: Term) -> Self {
        Self::Compare { op: CompareOp::LessOrEqual, left, right }
    }
    pub fn gt(left: Term, right: Term) -> Self {
        Self::Compare { op: CompareOp::Greater, left, right }
    }
    pub fn ge(left: Term, right: Term) -> Self {
        Self::Compare { op: CompareOp::GreaterOrEqual, left, right }
    }
    pub fn is_null(term: Term) -> Self {
        Self::Compare { op: CompareOp::IsNull, left: term, right: Term::Literal(Json::Null) }
    }
    pub fn is_not_null(term: Term) -> Self {
        Self::Compare { op: CompareOp::IsNotNull, left: term, right: Term::Literal(Json::Null) }
    }
    pub fn and(self, other: Self) -> Self {
        Self::And(Box::new(self), Box::new(other))
    }
    pub fn or(self, other: Self) -> Self {
        Self::Or(Box::new(self), Box::new(other))
    }
    pub fn not(self) -> Self {
        Self::Not(Box::new(self))
    }

    /// Evaluate against a row encoded as JSON. Returns `Some(true|false)`
    /// when fully decidable, `None` when a column is missing (treated
    /// as "undecidable" — manifest pruning skips the file).
    pub fn evaluate(&self, row: &Json) -> Option<bool> {
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            Self::Compare { op, left, right } => eval_compare(*op, left, right, row),
            Self::And(a, b) => match (a.evaluate(row), b.evaluate(row)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Self::Or(a, b) => match (a.evaluate(row), b.evaluate(row)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Self::Not(p) => p.evaluate(row).map(|v| !v),
        }
    }
}

fn resolve_term<'a>(t: &'a Term, row: &'a Json) -> Option<&'a Json> {
    match t {
        Term::Reference(r) => row.get(&r.name),
        Term::Literal(v) => Some(v),
    }
}

fn eval_compare(op: CompareOp, left: &Term, right: &Term, row: &Json) -> Option<bool> {
    if matches!(op, CompareOp::IsNull | CompareOp::IsNotNull) {
        let v = resolve_term(left, row);
        let is_null = match v {
            None => true,
            Some(j) => j.is_null(),
        };
        return Some(match op {
            CompareOp::IsNull => is_null,
            CompareOp::IsNotNull => !is_null,
            _ => unreachable!(),
        });
    }
    let l = resolve_term(left, row)?;
    let r = resolve_term(right, row)?;
    let ord = compare_json(l, r)?;
    Some(match op {
        CompareOp::Equal => ord == std::cmp::Ordering::Equal,
        CompareOp::NotEqual => ord != std::cmp::Ordering::Equal,
        CompareOp::Less => ord == std::cmp::Ordering::Less,
        CompareOp::LessOrEqual => ord != std::cmp::Ordering::Greater,
        CompareOp::Greater => ord == std::cmp::Ordering::Greater,
        CompareOp::GreaterOrEqual => ord != std::cmp::Ordering::Less,
        _ => unreachable!(),
    })
}

fn compare_json(a: &Json, b: &Json) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match (a, b) {
        (Json::Number(an), Json::Number(bn)) => {
            let af = an.as_f64()?;
            let bf = bn.as_f64()?;
            af.partial_cmp(&bf)
        }
        (Json::String(a), Json::String(b)) => Some(a.cmp(b)),
        (Json::Bool(a), Json::Bool(b)) => Some(a.cmp(b)),
        (Json::Null, Json::Null) => Some(Ordering::Equal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn eq_on_int_columns() {
        let p = Predicate::eq(Term::ref_col("a"), Term::lit(7));
        let row = json!({"a": 7});
        assert_eq!(p.evaluate(&row), Some(true));
        let row2 = json!({"a": 8});
        assert_eq!(p.evaluate(&row2), Some(false));
    }

    #[test]
    fn ord_compare_on_strings() {
        let p = Predicate::lt(Term::ref_col("k"), Term::lit("m"));
        let row = json!({"k": "a"});
        assert_eq!(p.evaluate(&row), Some(true));
    }

    #[test]
    fn missing_column_returns_undecidable() {
        let p = Predicate::eq(Term::ref_col("missing"), Term::lit(1));
        assert_eq!(p.evaluate(&json!({"a": 1})), None);
    }

    #[test]
    fn and_short_circuits_on_false() {
        let p = Predicate::eq(Term::ref_col("a"), Term::lit(1))
            .and(Predicate::eq(Term::ref_col("missing"), Term::lit(2)));
        let row = json!({"a": 2});
        // a != 1 → short-circuit false even though `missing` would be None.
        assert_eq!(p.evaluate(&row), Some(false));
    }

    #[test]
    fn or_short_circuits_on_true() {
        let p = Predicate::eq(Term::ref_col("a"), Term::lit(1))
            .or(Predicate::eq(Term::ref_col("missing"), Term::lit(2)));
        let row = json!({"a": 1});
        assert_eq!(p.evaluate(&row), Some(true));
    }

    #[test]
    fn is_null_detects_missing_and_null() {
        let p = Predicate::is_null(Term::ref_col("x"));
        assert_eq!(p.evaluate(&json!({"x": null})), Some(true));
        assert_eq!(p.evaluate(&json!({"x": 1})), Some(false));
        assert_eq!(p.evaluate(&json!({})), Some(true));
    }

    #[test]
    fn not_inverts_boolean() {
        let p = Predicate::eq(Term::ref_col("a"), Term::lit(1)).not();
        assert_eq!(p.evaluate(&json!({"a": 2})), Some(true));
        assert_eq!(p.evaluate(&json!({"a": 1})), Some(false));
    }

    #[test]
    fn predicate_json_round_trip() {
        let p = Predicate::eq(Term::ref_col("a"), Term::lit(1));
        let j = serde_json::to_string(&p).unwrap();
        let back: Predicate = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }
}
