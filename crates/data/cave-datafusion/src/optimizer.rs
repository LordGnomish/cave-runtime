// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Logical optimizer — rule-based `LogicalPlan` → `LogicalPlan` rewrites.
//!
//! Upstream: `apache/datafusion` `datafusion-optimizer/`.
//!
//! DataFusion's `Optimizer` owns an ordered list of `OptimizerRule`s and
//! applies them to a fixpoint (`Optimizer::optimize` loops the rule set
//! until the plan stops changing or `max_passes` is hit). This module
//! ports the core, dependency-free subset of those rules over the
//! cave-datafusion `LogicalPlan`/`LogicalExpr` AST:
//!
//!   * **Constant folding / expression simplification** — port of
//!     `simplify_expressions` (`ConstEvaluator` + `Simplifier`). A
//!     `BinaryOp` over two literals is evaluated once at plan time
//!     (re-using the physical arithmetic semantics so the folded value
//!     is bit-identical to what the executor would produce), and boolean
//!     identities collapse (`x AND true` → `x`, `x OR false` → `x`,
//!     `x AND false` → `false`, `x OR true` → `true`).
//!   * **Eliminate always-true filter** — port of `eliminate_filter`:
//!     `Filter(true, input)` → `input`.
//!   * **Merge consecutive filters** — the conjunction-combine step of
//!     `push_down_filter`: `Filter(p1, Filter(p2, x))` → `Filter(p1 ∧ p2, x)`.
//!   * **Predicate push-down through a pass-through projection** — port of
//!     `push_down_filter.rs`: `Filter(p, Projection(cols, x))` →
//!     `Projection(cols, Filter(p, x))` when the projection only re-emits
//!     bare input columns (so `p`'s column references resolve unchanged
//!     below the projection).
//!   * **Limit push-down through a projection** — port of
//!     `push_down_limit.rs`: a projection is 1:1 on rows, so the limit can
//!     run first.
//!   * **Identity-projection elimination** — port of
//!     `optimize_projections`/`eliminate_projection`: a projection that
//!     re-selects exactly the input's columns in order is removed.
//!
//! Each pass walks the tree bottom-up (children first), then applies the
//! node-local rewrites, and the whole thing iterates to a fixpoint. Rules
//! that need cross-node context (join reorder, full projection pruning,
//! filter push-down into the `TableProvider` scan) are deferred to
//! lakehouse-ray-2 alongside the vectorized executor.

use crate::logical_expr::{BinaryOp, LogicalExpr};
use crate::logical_plan::LogicalPlan;
use crate::physical_expr::{BinaryPhysicalOp, PhysicalExpr};
use crate::row::{Row, Value};

/// Maximum fixpoint passes before bailing out (a safety fuel, mirroring
/// upstream `Optimizer`'s `max_passes`).
const MAX_PASSES: usize = 16;

/// Rule-based logical optimizer.
#[derive(Debug, Clone, Default)]
pub struct Optimizer {
    max_passes: usize,
}

impl Optimizer {
    pub fn new() -> Self {
        Self {
            max_passes: MAX_PASSES,
        }
    }

    /// Override the fixpoint fuel.
    pub fn with_max_passes(mut self, n: usize) -> Self {
        self.max_passes = n.max(1);
        self
    }

    /// Optimize a plan to a fixpoint.
    pub fn optimize(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = plan;
        for _ in 0..self.max_passes {
            let next = optimize_node(current.clone());
            if next == current {
                return next;
            }
            current = next;
        }
        current
    }
}

/// Bottom-up rewrite of a single plan node: optimize children first, then
/// apply the node-local structural rules.
fn optimize_node(plan: LogicalPlan) -> LogicalPlan {
    let plan = match plan {
        LogicalPlan::Filter { predicate, input } => LogicalPlan::Filter {
            predicate: simplify_expr(predicate),
            input: Box::new(optimize_node(*input)),
        },
        LogicalPlan::Projection { expressions, input } => LogicalPlan::Projection {
            expressions: expressions.into_iter().map(simplify_expr).collect(),
            input: Box::new(optimize_node(*input)),
        },
        LogicalPlan::Aggregate {
            group_by,
            aggr,
            input,
        } => LogicalPlan::Aggregate {
            group_by: group_by.into_iter().map(simplify_expr).collect(),
            aggr: aggr.into_iter().map(simplify_expr).collect(),
            input: Box::new(optimize_node(*input)),
        },
        LogicalPlan::Sort { keys, input } => LogicalPlan::Sort {
            keys,
            input: Box::new(optimize_node(*input)),
        },
        LogicalPlan::Limit { skip, fetch, input } => LogicalPlan::Limit {
            skip,
            fetch,
            input: Box::new(optimize_node(*input)),
        },
        LogicalPlan::Join {
            kind,
            on,
            left,
            right,
        } => LogicalPlan::Join {
            kind,
            on,
            left: Box::new(optimize_node(*left)),
            right: Box::new(optimize_node(*right)),
        },
        LogicalPlan::Union { inputs } => LogicalPlan::Union {
            inputs: inputs.into_iter().map(optimize_node).collect(),
        },
        leaf @ (LogicalPlan::TableScan { .. } | LogicalPlan::EmptyRelation { .. }) => leaf,
    };
    apply_rules(plan)
}

/// Node-local structural rewrites (run after children are optimized).
fn apply_rules(plan: LogicalPlan) -> LogicalPlan {
    match plan {
        LogicalPlan::Filter { predicate, input } => {
            // eliminate_filter: drop an always-true predicate.
            if is_true_lit(&predicate) {
                return *input;
            }
            match *input {
                // merge consecutive filters into a single conjunction.
                LogicalPlan::Filter {
                    predicate: inner,
                    input: inner_input,
                } => LogicalPlan::Filter {
                    predicate: predicate.and(inner),
                    input: inner_input,
                },
                // push the predicate below a bare-column pass-through projection.
                LogicalPlan::Projection { expressions, input }
                    if is_column_passthrough(&expressions) =>
                {
                    LogicalPlan::Projection {
                        expressions,
                        input: Box::new(LogicalPlan::Filter {
                            predicate,
                            input,
                        }),
                    }
                }
                other => LogicalPlan::Filter {
                    predicate,
                    input: Box::new(other),
                },
            }
        }
        LogicalPlan::Limit { skip, fetch, input } => match *input {
            // push the limit below a projection (1:1 on rows).
            LogicalPlan::Projection { expressions, input } => LogicalPlan::Projection {
                expressions,
                input: Box::new(LogicalPlan::Limit {
                    skip,
                    fetch,
                    input,
                }),
            },
            other => LogicalPlan::Limit {
                skip,
                fetch,
                input: Box::new(other),
            },
        },
        LogicalPlan::Projection { expressions, input } => {
            if is_identity_projection(&expressions, &input) {
                *input
            } else {
                LogicalPlan::Projection { expressions, input }
            }
        }
        other => other,
    }
}

/// A projection is a bare-column pass-through when every output expression
/// is a plain `Column` (no alias, no computed expression). Filters can be
/// pushed below such a projection because the column references resolve
/// identically above and below it.
fn is_column_passthrough(exprs: &[LogicalExpr]) -> bool {
    !exprs.is_empty()
        && exprs
            .iter()
            .all(|e| matches!(e, LogicalExpr::Column { .. }))
}

/// A projection is an identity when it re-emits exactly the input's output
/// columns, in order, as bare column references.
fn is_identity_projection(exprs: &[LogicalExpr], input: &LogicalPlan) -> bool {
    if !is_column_passthrough(exprs) {
        return false;
    }
    let proj_names: Vec<&str> = exprs
        .iter()
        .map(|e| match e {
            LogicalExpr::Column { name } => name.as_str(),
            _ => unreachable!("guarded by is_column_passthrough"),
        })
        .collect();
    output_columns(input)
        .map(|cols| cols == proj_names)
        .unwrap_or(false)
}

/// The ordered output column names of a plan node, when resolvable. Used by
/// the identity-projection rule.
fn output_columns(plan: &LogicalPlan) -> Option<Vec<&str>> {
    match plan {
        LogicalPlan::TableScan { schema, .. } | LogicalPlan::EmptyRelation { schema } => {
            Some(schema.fields.iter().map(|f| f.name.as_str()).collect())
        }
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Limit { input, .. } => output_columns(input),
        _ => None,
    }
}

fn is_true_lit(e: &LogicalExpr) -> bool {
    matches!(
        e,
        LogicalExpr::Literal {
            value: Value::Bool(true)
        }
    )
}

fn is_false_lit(e: &LogicalExpr) -> bool {
    matches!(
        e,
        LogicalExpr::Literal {
            value: Value::Bool(false)
        }
    )
}

/// Bottom-up expression simplifier: constant folding + boolean identities.
fn simplify_expr(expr: LogicalExpr) -> LogicalExpr {
    match expr {
        LogicalExpr::BinaryOp { op, left, right } => {
            let left = simplify_expr(*left);
            let right = simplify_expr(*right);

            // Constant folding: both operands are literals.
            if let (LogicalExpr::Literal { value: l }, LogicalExpr::Literal { value: r }) =
                (&left, &right)
            {
                if let Some(folded) = fold_binary(op, l, r) {
                    return LogicalExpr::Literal { value: folded };
                }
            }

            // Boolean short-circuit identities.
            match op {
                BinaryOp::And => {
                    if is_false_lit(&left) || is_false_lit(&right) {
                        return LogicalExpr::lit(false);
                    }
                    if is_true_lit(&left) {
                        return right;
                    }
                    if is_true_lit(&right) {
                        return left;
                    }
                }
                BinaryOp::Or => {
                    if is_true_lit(&left) || is_true_lit(&right) {
                        return LogicalExpr::lit(true);
                    }
                    if is_false_lit(&left) {
                        return right;
                    }
                    if is_false_lit(&right) {
                        return left;
                    }
                }
                _ => {}
            }

            LogicalExpr::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }
        LogicalExpr::Not { expr } => {
            let inner = simplify_expr(*expr);
            // Fold NOT over a boolean literal; collapse double negation.
            match inner {
                LogicalExpr::Literal {
                    value: Value::Bool(b),
                } => LogicalExpr::lit(!b),
                LogicalExpr::Not { expr } => *expr,
                other => LogicalExpr::Not {
                    expr: Box::new(other),
                },
            }
        }
        LogicalExpr::IsNull { expr } => LogicalExpr::IsNull {
            expr: Box::new(simplify_expr(*expr)),
        },
        LogicalExpr::IsNotNull { expr } => LogicalExpr::IsNotNull {
            expr: Box::new(simplify_expr(*expr)),
        },
        LogicalExpr::Cast { expr, to } => LogicalExpr::Cast {
            expr: Box::new(simplify_expr(*expr)),
            to,
        },
        LogicalExpr::Alias { expr, alias } => LogicalExpr::Alias {
            expr: Box::new(simplify_expr(*expr)),
            alias,
        },
        LogicalExpr::Function { name, args } => LogicalExpr::Function {
            name,
            args: args.into_iter().map(simplify_expr).collect(),
        },
        leaf @ (LogicalExpr::Column { .. } | LogicalExpr::Literal { .. }) => leaf,
    }
}

/// Evaluate a binary op over two literal values, re-using the exact
/// physical-executor arithmetic/comparison semantics so a folded constant
/// is bit-identical to the runtime result. Returns `None` (leave the
/// expression unfolded) on any evaluation error — e.g. divide-by-zero or a
/// type mismatch — matching upstream's `ConstEvaluator`, which declines to
/// fold expressions that would error.
fn fold_binary(op: BinaryOp, l: &Value, r: &Value) -> Option<Value> {
    let phys = PhysicalExpr::Binary {
        op: to_physical_op(op),
        left: Box::new(PhysicalExpr::Literal { value: l.clone() }),
        right: Box::new(PhysicalExpr::Literal { value: r.clone() }),
    };
    phys.evaluate(&Row::new(vec![])).ok()
}

fn to_physical_op(op: BinaryOp) -> BinaryPhysicalOp {
    match op {
        BinaryOp::Plus => BinaryPhysicalOp::Plus,
        BinaryOp::Minus => BinaryPhysicalOp::Minus,
        BinaryOp::Multiply => BinaryPhysicalOp::Multiply,
        BinaryOp::Divide => BinaryPhysicalOp::Divide,
        BinaryOp::Modulo => BinaryPhysicalOp::Modulo,
        BinaryOp::Eq => BinaryPhysicalOp::Eq,
        BinaryOp::NotEq => BinaryPhysicalOp::NotEq,
        BinaryOp::Lt => BinaryPhysicalOp::Lt,
        BinaryOp::LtEq => BinaryPhysicalOp::LtEq,
        BinaryOp::Gt => BinaryPhysicalOp::Gt,
        BinaryOp::GtEq => BinaryPhysicalOp::GtEq,
        BinaryOp::And => BinaryPhysicalOp::And,
        BinaryOp::Or => BinaryPhysicalOp::Or,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_declines_divide_by_zero() {
        // 1 / 0 must stay unfolded (physical eval errors).
        let r = fold_binary(BinaryOp::Divide, &Value::Int64(1), &Value::Int64(0));
        assert!(r.is_none());
    }

    #[test]
    fn simplify_double_negation() {
        let e = LogicalExpr::col("a")
            .gt(LogicalExpr::lit(1));
        let nn = LogicalExpr::Not {
            expr: Box::new(LogicalExpr::Not {
                expr: Box::new(e.clone()),
            }),
        };
        assert_eq!(simplify_expr(nn), e);
    }

    #[test]
    fn simplify_and_false_is_false() {
        let e = LogicalExpr::col("a")
            .gt(LogicalExpr::lit(1))
            .and(LogicalExpr::lit(false));
        assert_eq!(simplify_expr(e), LogicalExpr::lit(false));
    }
}
