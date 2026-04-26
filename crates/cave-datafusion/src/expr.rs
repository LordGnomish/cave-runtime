//! Logical expressions — column refs, literals, binary ops.
//!
//! Mirrors apache/datafusion datafusion-expr/src/expr.rs `Expr` enum (subset).

use crate::batch::{RecordBatch, Value};
use crate::error::{DataFusionError, DfResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BinaryOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

impl BinaryOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Eq => "=",
            BinaryOp::NotEq => "<>",
            BinaryOp::Lt => "<",
            BinaryOp::LtEq => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::GtEq => ">=",
            BinaryOp::And => "AND",
            BinaryOp::Or => "OR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Column(String),
    Literal(Value),
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
}

impl Expr {
    pub fn col(name: impl Into<String>) -> Self {
        Expr::Column(name.into())
    }

    pub fn lit(v: Value) -> Self {
        Expr::Literal(v)
    }

    pub fn eq(self, other: Expr) -> Self {
        Expr::Binary {
            left: Box::new(self),
            op: BinaryOp::Eq,
            right: Box::new(other),
        }
    }

    pub fn lt(self, other: Expr) -> Self {
        Expr::Binary {
            left: Box::new(self),
            op: BinaryOp::Lt,
            right: Box::new(other),
        }
    }

    pub fn gt(self, other: Expr) -> Self {
        Expr::Binary {
            left: Box::new(self),
            op: BinaryOp::Gt,
            right: Box::new(other),
        }
    }

    pub fn and(self, other: Expr) -> Self {
        Expr::Binary {
            left: Box::new(self),
            op: BinaryOp::And,
            right: Box::new(other),
        }
    }

    /// Evaluate this expression against one row of `batch`.
    /// Returns Value::Null when any operand is null (SQL nullability).
    pub fn evaluate(&self, batch: &RecordBatch, row: &[Value]) -> DfResult<Value> {
        match self {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Column(name) => {
                let idx = batch.column_index(name)?;
                Ok(row[idx].clone())
            }
            Expr::Binary { left, op, right } => {
                let l = left.evaluate(batch, row)?;
                let r = right.evaluate(batch, row)?;
                eval_binary(&l, *op, &r)
            }
        }
    }
}

fn eval_binary(l: &Value, op: BinaryOp, r: &Value) -> DfResult<Value> {
    if l.is_null() || r.is_null() {
        return Ok(Value::Null);
    }
    match op {
        BinaryOp::Eq => Ok(Value::Bool(values_equal(l, r))),
        BinaryOp::NotEq => Ok(Value::Bool(!values_equal(l, r))),
        BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
            let cmp = compare(l, r)?;
            let b = match op {
                BinaryOp::Lt => cmp == std::cmp::Ordering::Less,
                BinaryOp::LtEq => cmp != std::cmp::Ordering::Greater,
                BinaryOp::Gt => cmp == std::cmp::Ordering::Greater,
                BinaryOp::GtEq => cmp != std::cmp::Ordering::Less,
                _ => unreachable!(),
            };
            Ok(Value::Bool(b))
        }
        BinaryOp::And => {
            let lb = l.as_bool().ok_or_else(|| {
                DataFusionError::TypeMismatch(format!(
                    "AND expects bool, got {}",
                    l.type_name()
                ))
            })?;
            let rb = r.as_bool().ok_or_else(|| {
                DataFusionError::TypeMismatch(format!(
                    "AND expects bool, got {}",
                    r.type_name()
                ))
            })?;
            Ok(Value::Bool(lb && rb))
        }
        BinaryOp::Or => {
            let lb = l.as_bool().ok_or_else(|| {
                DataFusionError::TypeMismatch(format!(
                    "OR expects bool, got {}",
                    l.type_name()
                ))
            })?;
            let rb = r.as_bool().ok_or_else(|| {
                DataFusionError::TypeMismatch(format!(
                    "OR expects bool, got {}",
                    r.type_name()
                ))
            })?;
            Ok(Value::Bool(lb || rb))
        }
    }
}

fn values_equal(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int64(a), Value::Int64(b)) => a == b,
        (Value::Float64(a), Value::Float64(b)) => a == b,
        (Value::Utf8(a), Value::Utf8(b)) => a == b,
        _ => false,
    }
}

fn compare(l: &Value, r: &Value) -> DfResult<std::cmp::Ordering> {
    match (l, r) {
        (Value::Int64(a), Value::Int64(b)) => Ok(a.cmp(b)),
        (Value::Float64(a), Value::Float64(b)) => a
            .partial_cmp(b)
            .ok_or_else(|| DataFusionError::TypeMismatch("NaN comparison".into())),
        (Value::Utf8(a), Value::Utf8(b)) => Ok(a.cmp(b)),
        _ => Err(DataFusionError::TypeMismatch(format!(
            "cannot compare {} with {}",
            l.type_name(),
            r.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> RecordBatch {
        RecordBatch::new(
            vec!["id".into(), "age".into()],
            vec![
                vec![Value::Int64(1), Value::Int64(25)],
                vec![Value::Int64(2), Value::Int64(35)],
            ],
        )
        .unwrap()
    }

    // ── BinaryOp ──────────────────────────────────────────────────────────────

    #[test]
    fn binary_op_symbols() {
        // citation: SQL standard binary operator notations
        assert_eq!(BinaryOp::Eq.symbol(), "=");
        assert_eq!(BinaryOp::NotEq.symbol(), "<>");
        assert_eq!(BinaryOp::Lt.symbol(), "<");
        assert_eq!(BinaryOp::LtEq.symbol(), "<=");
        assert_eq!(BinaryOp::Gt.symbol(), ">");
        assert_eq!(BinaryOp::GtEq.symbol(), ">=");
        assert_eq!(BinaryOp::And.symbol(), "AND");
        assert_eq!(BinaryOp::Or.symbol(), "OR");
    }

    // ── Expr constructors ─────────────────────────────────────────────────────

    #[test]
    fn col_constructor() {
        assert_eq!(Expr::col("x"), Expr::Column("x".into()));
    }

    #[test]
    fn lit_constructor() {
        assert_eq!(Expr::lit(Value::Int64(5)), Expr::Literal(Value::Int64(5)));
    }

    #[test]
    fn fluent_eq() {
        let e = Expr::col("a").eq(Expr::lit(Value::Int64(1)));
        assert!(matches!(e, Expr::Binary { op: BinaryOp::Eq, .. }));
    }

    #[test]
    fn fluent_chain_and() {
        let e = Expr::col("a")
            .gt(Expr::lit(Value::Int64(0)))
            .and(Expr::col("a").lt(Expr::lit(Value::Int64(10))));
        if let Expr::Binary { op, .. } = e {
            assert_eq!(op, BinaryOp::And);
        } else {
            panic!("expected Binary AND");
        }
    }

    // ── evaluate — column / literal ───────────────────────────────────────────

    #[test]
    fn eval_literal() {
        let v = Expr::lit(Value::Int64(42)).evaluate(&batch(), &[]).unwrap();
        assert_eq!(v, Value::Int64(42));
    }

    #[test]
    fn eval_column_first_row() {
        let b = batch();
        let v = Expr::col("age").evaluate(&b, &b.rows[0]).unwrap();
        assert_eq!(v, Value::Int64(25));
    }

    #[test]
    fn eval_column_unknown_err() {
        let b = batch();
        let e = Expr::col("missing").evaluate(&b, &b.rows[0]).unwrap_err();
        assert!(matches!(e, DataFusionError::ColumnNotFound(_)));
    }

    // ── evaluate — comparisons ────────────────────────────────────────────────

    #[test]
    fn eval_eq_int_true() {
        let b = batch();
        let e = Expr::col("age").eq(Expr::lit(Value::Int64(25)));
        assert_eq!(e.evaluate(&b, &b.rows[0]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn eval_eq_int_false() {
        let b = batch();
        let e = Expr::col("age").eq(Expr::lit(Value::Int64(99)));
        assert_eq!(e.evaluate(&b, &b.rows[0]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn eval_lt_int() {
        let b = batch();
        let e = Expr::col("age").lt(Expr::lit(Value::Int64(30)));
        assert_eq!(e.evaluate(&b, &b.rows[0]).unwrap(), Value::Bool(true));
        assert_eq!(e.evaluate(&b, &b.rows[1]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn eval_gt_int() {
        let b = batch();
        let e = Expr::col("age").gt(Expr::lit(Value::Int64(30)));
        assert_eq!(e.evaluate(&b, &b.rows[1]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn eval_eq_string() {
        let b = RecordBatch::new(
            vec!["name".into()],
            vec![vec![Value::Utf8("alice".into())]],
        )
        .unwrap();
        let e = Expr::col("name").eq(Expr::lit(Value::Utf8("alice".into())));
        assert_eq!(e.evaluate(&b, &b.rows[0]).unwrap(), Value::Bool(true));
    }

    // ── evaluate — null propagation (SQL three-valued logic) ──────────────────

    #[test]
    fn eval_null_propagates_through_binary() {
        let b = RecordBatch::new(vec!["x".into()], vec![vec![Value::Null]]).unwrap();
        let e = Expr::col("x").eq(Expr::lit(Value::Int64(1)));
        // citation: SQL three-valued logic — null op anything = null
        assert_eq!(e.evaluate(&b, &b.rows[0]).unwrap(), Value::Null);
    }

    // ── evaluate — AND / OR ───────────────────────────────────────────────────

    #[test]
    fn eval_and_true_true() {
        let e = Expr::lit(Value::Bool(true)).and(Expr::lit(Value::Bool(true)));
        assert_eq!(e.evaluate(&batch(), &[]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn eval_and_true_false() {
        let e = Expr::lit(Value::Bool(true)).and(Expr::lit(Value::Bool(false)));
        assert_eq!(e.evaluate(&batch(), &[]).unwrap(), Value::Bool(false));
    }

    #[test]
    fn eval_and_type_mismatch_err() {
        let e = Expr::lit(Value::Int64(1)).and(Expr::lit(Value::Bool(true)));
        let err = e.evaluate(&batch(), &[]).unwrap_err();
        assert!(matches!(err, DataFusionError::TypeMismatch(_)));
    }

    // ── evaluate — type errors ────────────────────────────────────────────────

    #[test]
    fn eval_compare_mixed_types_err() {
        let e = Expr::lit(Value::Int64(1)).lt(Expr::lit(Value::Utf8("a".into())));
        assert!(matches!(
            e.evaluate(&batch(), &[]).unwrap_err(),
            DataFusionError::TypeMismatch(_)
        ));
    }

    // ── eq across same value ──────────────────────────────────────────────────

    #[test]
    fn eval_eq_bool_bool() {
        let e = Expr::lit(Value::Bool(true)).eq(Expr::lit(Value::Bool(true)));
        assert_eq!(e.evaluate(&batch(), &[]).unwrap(), Value::Bool(true));
    }

    #[test]
    fn eval_eq_float_float() {
        let e = Expr::lit(Value::Float64(1.5)).eq(Expr::lit(Value::Float64(1.5)));
        assert_eq!(e.evaluate(&batch(), &[]).unwrap(), Value::Bool(true));
    }

    // ── serde ─────────────────────────────────────────────────────────────────

    #[test]
    fn expr_serde_roundtrip() {
        let e = Expr::col("a").eq(Expr::lit(Value::Int64(1)));
        let j = serde_json::to_string(&e).unwrap();
        let back: Expr = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }
}
