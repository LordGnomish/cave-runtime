// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query optimizer — basic rewrites.

use crate::sql::ast::*;

pub struct Optimizer;

impl Optimizer {
    /// Constant folding: evaluate const expressions at plan time.
    pub fn fold_constants(expr: &Expr) -> Expr {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                let left = Self::fold_constants(left);
                let right = Self::fold_constants(right);
                if let (Expr::Literal(l), Expr::Literal(r)) = (&left, &right) {
                    if let Some(result) = Self::eval_binop(l, *op, r) {
                        return Expr::Literal(result);
                    }
                }
                Expr::BinaryOp {
                    left: Box::new(left),
                    op: *op,
                    right: Box::new(right),
                }
            }
            Expr::UnaryOp { op, operand } => {
                let operand = Self::fold_constants(operand);
                if let Expr::Literal(lit) = &operand {
                    if let Some(result) = Self::eval_unop(*op, lit) {
                        return Expr::Literal(result);
                    }
                }
                Expr::UnaryOp {
                    op: *op,
                    operand: Box::new(operand),
                }
            }
            _ => expr.clone(),
        }
    }

    fn eval_binop(left: &Literal, op: BinaryOp, right: &Literal) -> Option<Literal> {
        match (left, op, right) {
            (Literal::Integer(a), BinaryOp::Add, Literal::Integer(b)) => Some(Literal::Integer(a + b)),
            (Literal::Integer(a), BinaryOp::Sub, Literal::Integer(b)) => Some(Literal::Integer(a - b)),
            (Literal::Integer(a), BinaryOp::Mul, Literal::Integer(b)) => Some(Literal::Integer(a * b)),
            (Literal::Integer(a), BinaryOp::Div, Literal::Integer(b)) if *b != 0 => {
                Some(Literal::Integer(a / b))
            }
            (Literal::Float(a), BinaryOp::Add, Literal::Float(b)) => Some(Literal::Float(a + b)),
            (Literal::Float(a), BinaryOp::Sub, Literal::Float(b)) => Some(Literal::Float(a - b)),
            (Literal::Float(a), BinaryOp::Mul, Literal::Float(b)) => Some(Literal::Float(a * b)),
            (Literal::Float(a), BinaryOp::Div, Literal::Float(b)) if *b != 0.0 => {
                Some(Literal::Float(a / b))
            }
            _ => None,
        }
    }

    fn eval_unop(op: UnaryOp, lit: &Literal) -> Option<Literal> {
        match (op, lit) {
            (UnaryOp::Minus, Literal::Integer(n)) => Some(Literal::Integer(-n)),
            (UnaryOp::Minus, Literal::Float(f)) => Some(Literal::Float(-f)),
            (UnaryOp::Not, Literal::Boolean(b)) => Some(Literal::Boolean(!b)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_folding_add() {
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Literal(Literal::Integer(2))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Literal::Integer(3))),
        };
        let folded = Optimizer::fold_constants(&expr);
        assert!(matches!(folded, Expr::Literal(Literal::Integer(5))));
    }

    #[test]
    fn test_constant_folding_multiply() {
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Literal(Literal::Integer(4))),
            op: BinaryOp::Mul,
            right: Box::new(Expr::Literal(Literal::Integer(5))),
        };
        let folded = Optimizer::fold_constants(&expr);
        assert!(matches!(folded, Expr::Literal(Literal::Integer(20))));
    }

    #[test]
    fn test_unary_minus_folding() {
        let expr = Expr::UnaryOp {
            op: UnaryOp::Minus,
            operand: Box::new(Expr::Literal(Literal::Integer(10))),
        };
        let folded = Optimizer::fold_constants(&expr);
        assert!(matches!(folded, Expr::Literal(Literal::Integer(-10))));
    }
}
