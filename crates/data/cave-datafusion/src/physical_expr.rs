// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! PhysicalExpr — DataFusion physical expression AST.
//!
//! Upstream: `crates/datafusion-physical-expr/src/expressions/`
//!
//! The PhysicalExpr is the lowered form of LogicalExpr — columns are
//! resolved to integer indices (avoiding a HashMap lookup per row), and
//! aggregates are split into their state-update + result-collect halves.
//! The MVP row-at-a-time `evaluate(&Row) -> Value` is enough to power
//! filter / project / sort / aggregate execution.

use crate::error::{Error, Result};
use crate::functions::ScalarFnHandle;
use crate::row::{Row, Value};
use crate::schema::DataType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BinaryPhysicalOp {
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PhysicalExpr {
    Column {
        index: usize,
    },
    Literal {
        value: Value,
    },
    Binary {
        op: BinaryPhysicalOp,
        left: Box<PhysicalExpr>,
        right: Box<PhysicalExpr>,
    },
    Not {
        expr: Box<PhysicalExpr>,
    },
    IsNull {
        expr: Box<PhysicalExpr>,
    },
    IsNotNull {
        expr: Box<PhysicalExpr>,
    },
    Cast {
        expr: Box<PhysicalExpr>,
        to: DataType,
    },
    /// Row-level scalar function invocation — the lowered form of a
    /// `LogicalExpr::Function` whose arguments are not all literals.
    ///
    /// Upstream: `datafusion-physical-expr/src/scalar_function.rs`
    /// (`ScalarFunctionExpr`). Each argument is evaluated against the row,
    /// then the function is invoked over the resulting values. The closure
    /// is `#[serde(skip)]`-ed (a function pointer can't round-trip through
    /// JSON); a deserialized `Call` carries the `Default` stub and must be
    /// re-bound against a `FunctionRegistry` before execution.
    Call {
        name: String,
        #[serde(skip)]
        fun: ScalarFnHandle,
        args: Vec<PhysicalExpr>,
    },
}

impl PhysicalExpr {
    pub fn evaluate(&self, row: &Row) -> Result<Value> {
        match self {
            Self::Column { index } => row
                .get(*index)
                .cloned()
                .ok_or_else(|| Error::Execution(format!("column index {} out of bounds", index))),
            Self::Literal { value } => Ok(value.clone()),
            Self::Binary { op, left, right } => {
                let l = left.evaluate(row)?;
                let r = right.evaluate(row)?;
                eval_binary(*op, &l, &r)
            }
            Self::Not { expr } => {
                let v = expr.evaluate(row)?;
                Ok(match v {
                    Value::Bool(b) => Value::Bool(!b),
                    Value::Null => Value::Null,
                    _ => return Err(Error::TypeMismatch("NOT requires boolean".into())),
                })
            }
            Self::IsNull { expr } => Ok(Value::Bool(expr.evaluate(row)?.is_null())),
            Self::IsNotNull { expr } => Ok(Value::Bool(!expr.evaluate(row)?.is_null())),
            Self::Cast { expr, to } => {
                let v = expr.evaluate(row)?;
                cast(&v, *to)
            }
            Self::Call { fun, args, .. } => {
                let vals: Vec<Value> = args
                    .iter()
                    .map(|a| a.evaluate(row))
                    .collect::<Result<_>>()?;
                fun.call(&vals)
            }
        }
    }
}

fn eval_binary(op: BinaryPhysicalOp, l: &Value, r: &Value) -> Result<Value> {
    use BinaryPhysicalOp::*;

    if l.is_null() || r.is_null() {
        return Ok(Value::Null);
    }

    match op {
        Plus | Minus | Multiply | Divide | Modulo => {
            let lf = l
                .as_f64()
                .ok_or_else(|| Error::TypeMismatch("arithmetic on non-numeric".into()))?;
            let rf = r
                .as_f64()
                .ok_or_else(|| Error::TypeMismatch("arithmetic on non-numeric".into()))?;
            let out = match op {
                Plus => lf + rf,
                Minus => lf - rf,
                Multiply => lf * rf,
                Divide => {
                    if rf == 0.0 {
                        return Err(Error::Execution("divide by zero".into()));
                    }
                    lf / rf
                }
                Modulo => {
                    if rf == 0.0 {
                        return Err(Error::Execution("modulo by zero".into()));
                    }
                    lf % rf
                }
                _ => unreachable!(),
            };
            // Promote to f64 unless both sides are integer.
            if matches!(l, Value::Int32(_) | Value::Int64(_))
                && matches!(r, Value::Int32(_) | Value::Int64(_))
                && matches!(op, Plus | Minus | Multiply | Modulo)
            {
                Ok(Value::Int64(out as i64))
            } else {
                Ok(Value::Float64(out))
            }
        }
        Eq => Ok(Value::Bool(l.cmp_nulls_first(r).is_eq())),
        NotEq => Ok(Value::Bool(!l.cmp_nulls_first(r).is_eq())),
        Lt => Ok(Value::Bool(l.cmp_nulls_first(r).is_lt())),
        LtEq => Ok(Value::Bool(l.cmp_nulls_first(r).is_le())),
        Gt => Ok(Value::Bool(l.cmp_nulls_first(r).is_gt())),
        GtEq => Ok(Value::Bool(l.cmp_nulls_first(r).is_ge())),
        And => Ok(Value::Bool(
            l.as_bool().unwrap_or(false) && r.as_bool().unwrap_or(false),
        )),
        Or => Ok(Value::Bool(
            l.as_bool().unwrap_or(false) || r.as_bool().unwrap_or(false),
        )),
    }
}

fn cast(v: &Value, to: DataType) -> Result<Value> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    let out = match to {
        DataType::Int32 => Value::Int32(
            v.as_i64()
                .ok_or_else(|| Error::TypeMismatch(format!("cast {:?}→Int32", v.data_type())))?
                as i32,
        ),
        DataType::Int64 => Value::Int64(
            v.as_i64()
                .ok_or_else(|| Error::TypeMismatch(format!("cast {:?}→Int64", v.data_type())))?,
        ),
        DataType::Float64 => Value::Float64(
            v.as_f64()
                .ok_or_else(|| Error::TypeMismatch(format!("cast {:?}→Float64", v.data_type())))?,
        ),
        DataType::Float32 => Value::Float64(
            v.as_f64()
                .ok_or_else(|| Error::TypeMismatch(format!("cast {:?}→Float32", v.data_type())))?,
        ),
        DataType::Boolean => Value::Bool(match v {
            Value::Bool(b) => *b,
            Value::Int32(0) | Value::Int64(0) => false,
            Value::Int32(_) | Value::Int64(_) => true,
            _ => return Err(Error::TypeMismatch("cast to Bool unsupported".into())),
        }),
        DataType::Utf8 => Value::Utf8(match v {
            Value::Utf8(s) => s.clone(),
            Value::Int32(n) => n.to_string(),
            Value::Int64(n) => n.to_string(),
            Value::Float64(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return Err(Error::TypeMismatch("cast to Utf8 unsupported".into())),
        }),
        _ => return Err(Error::TypeMismatch(format!("cast to {:?} unsupported", to))),
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: Vec<Value>) -> Row {
        Row::new(values)
    }

    #[test]
    fn column_evaluates_by_index() {
        let r = row(vec![Value::Int64(1), Value::Utf8("x".into())]);
        let e = PhysicalExpr::Column { index: 1 };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Utf8("x".into()));
    }

    #[test]
    fn arithmetic_promotes_to_float_when_one_side_is_float() {
        let r = row(vec![Value::Int64(3), Value::Float64(2.5)]);
        let e = PhysicalExpr::Binary {
            op: BinaryPhysicalOp::Plus,
            left: Box::new(PhysicalExpr::Column { index: 0 }),
            right: Box::new(PhysicalExpr::Column { index: 1 }),
        };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Float64(5.5));
    }

    #[test]
    fn arithmetic_keeps_int_when_both_int() {
        let r = row(vec![Value::Int64(3), Value::Int64(4)]);
        let e = PhysicalExpr::Binary {
            op: BinaryPhysicalOp::Multiply,
            left: Box::new(PhysicalExpr::Column { index: 0 }),
            right: Box::new(PhysicalExpr::Column { index: 1 }),
        };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Int64(12));
    }

    #[test]
    fn divide_by_zero_errors() {
        let r = row(vec![Value::Int64(3), Value::Int64(0)]);
        let e = PhysicalExpr::Binary {
            op: BinaryPhysicalOp::Divide,
            left: Box::new(PhysicalExpr::Column { index: 0 }),
            right: Box::new(PhysicalExpr::Column { index: 1 }),
        };
        assert!(matches!(e.evaluate(&r), Err(Error::Execution(_))));
    }

    #[test]
    fn comparisons_emit_bool() {
        let r = row(vec![Value::Int64(3)]);
        let e = PhysicalExpr::Binary {
            op: BinaryPhysicalOp::Gt,
            left: Box::new(PhysicalExpr::Column { index: 0 }),
            right: Box::new(PhysicalExpr::Literal {
                value: Value::Int64(2),
            }),
        };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Bool(true));
    }

    #[test]
    fn null_propagates_through_arithmetic() {
        let r = row(vec![Value::Null]);
        let e = PhysicalExpr::Binary {
            op: BinaryPhysicalOp::Plus,
            left: Box::new(PhysicalExpr::Column { index: 0 }),
            right: Box::new(PhysicalExpr::Literal {
                value: Value::Int64(1),
            }),
        };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Null);
    }

    #[test]
    fn cast_int_to_utf8() {
        let r = row(vec![Value::Int64(42)]);
        let e = PhysicalExpr::Cast {
            expr: Box::new(PhysicalExpr::Column { index: 0 }),
            to: DataType::Utf8,
        };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Utf8("42".to_string()));
    }

    #[test]
    fn is_null_returns_bool() {
        let r = row(vec![Value::Null, Value::Int64(1)]);
        let e = PhysicalExpr::IsNull {
            expr: Box::new(PhysicalExpr::Column { index: 0 }),
        };
        assert_eq!(e.evaluate(&r).unwrap(), Value::Bool(true));
        let e2 = PhysicalExpr::IsNotNull {
            expr: Box::new(PhysicalExpr::Column { index: 1 }),
        };
        assert_eq!(e2.evaluate(&r).unwrap(), Value::Bool(true));
    }
}
