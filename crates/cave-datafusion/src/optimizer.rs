//! Logical-plan optimizer rules — predicate pushdown, projection pushdown,
//! limit pushdown, constant folding, and a top-level pass loop.
//!
//! Mirrors apache/datafusion-optimizer crate's rule pattern:
//!
//! ```text
//! pub trait OptimizerRule { fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan; }
//! ```
//!
//! Each rule walks the tree and returns a (possibly transformed) plan. The
//! [`Optimizer`] runs all rules until a fixed point is reached or
//! `max_passes` is hit.

use crate::batch::Value;
use crate::expr::{BinaryOp, Expr};
use crate::logical_plan::LogicalPlan;

pub trait OptimizerRule: std::fmt::Debug {
    fn name(&self) -> &'static str;
    fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan;
}

#[derive(Debug)]
pub struct Optimizer {
    pub rules: Vec<Box<dyn OptimizerRule>>,
    pub max_passes: usize,
}

impl Default for Optimizer {
    fn default() -> Self {
        Self {
            rules: vec![
                Box::new(PredicatePushdown),
                Box::new(ProjectionPushdown),
                Box::new(LimitPushdown),
                Box::new(ConstantFolding),
                Box::new(EliminateRedundantProjection),
            ],
            max_passes: 5,
        }
    }
}

impl Optimizer {
    pub fn optimize(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = plan;
        for _ in 0..self.max_passes {
            let mut next = current.clone();
            for rule in &self.rules {
                next = rule.rewrite(next);
            }
            if next == current {
                return current;
            }
            current = next;
        }
        current
    }
}

// ── Rules ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct PredicatePushdown;

impl OptimizerRule for PredicatePushdown {
    fn name(&self) -> &'static str {
        "predicate_pushdown"
    }
    fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Filter { input, predicate } => {
                let inner = self.rewrite(*input);
                if let LogicalPlan::Projection { input, columns } = inner {
                    let pred_cols = predicate_columns(&predicate);
                    let all_present = pred_cols.iter().all(|c| columns.contains(c));
                    if all_present {
                        let new_filter = LogicalPlan::Filter {
                            input,
                            predicate,
                        };
                        return LogicalPlan::Projection {
                            input: Box::new(self.rewrite(new_filter)),
                            columns,
                        };
                    }
                    return LogicalPlan::Filter {
                        input: Box::new(LogicalPlan::Projection { input, columns }),
                        predicate,
                    };
                }
                LogicalPlan::Filter {
                    input: Box::new(inner),
                    predicate,
                }
            }
            other => map_children(other, self),
        }
    }
}

#[derive(Debug)]
pub struct ProjectionPushdown;

impl OptimizerRule for ProjectionPushdown {
    fn name(&self) -> &'static str {
        "projection_pushdown"
    }
    fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Projection { input, columns } => {
                let inner = self.rewrite(*input);
                match inner {
                    LogicalPlan::Scan {
                        table_name,
                        tenant_id,
                        ..
                    } => LogicalPlan::Scan {
                        table_name,
                        projection: Some(columns),
                        tenant_id,
                    },
                    LogicalPlan::Filter { input: filter_input, predicate } => {
                        // Push projection through filter when inner is a Scan,
                        // including any columns referenced by the predicate.
                        if let LogicalPlan::Scan {
                            table_name,
                            tenant_id,
                            ..
                        } = *filter_input
                        {
                            let mut scan_cols = columns.clone();
                            for c in predicate_columns(&predicate) {
                                if !scan_cols.contains(&c) {
                                    scan_cols.push(c);
                                }
                            }
                            return LogicalPlan::Projection {
                                input: Box::new(LogicalPlan::Filter {
                                    input: Box::new(LogicalPlan::Scan {
                                        table_name,
                                        projection: Some(scan_cols),
                                        tenant_id,
                                    }),
                                    predicate,
                                }),
                                columns,
                            };
                        }
                        LogicalPlan::Projection {
                            input: Box::new(LogicalPlan::Filter {
                                input: filter_input,
                                predicate,
                            }),
                            columns,
                        }
                    }
                    other => LogicalPlan::Projection {
                        input: Box::new(other),
                        columns,
                    },
                }
            }
            other => map_children(other, self),
        }
    }
}

#[derive(Debug)]
pub struct LimitPushdown;

impl OptimizerRule for LimitPushdown {
    fn name(&self) -> &'static str {
        "limit_pushdown"
    }
    fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Limit { input, skip, fetch } => {
                let inner = self.rewrite(*input);
                if let LogicalPlan::Projection { input, columns } = inner {
                    return LogicalPlan::Projection {
                        input: Box::new(LogicalPlan::Limit { input, skip, fetch }),
                        columns,
                    };
                }
                LogicalPlan::Limit {
                    input: Box::new(inner),
                    skip,
                    fetch,
                }
            }
            other => map_children(other, self),
        }
    }
}

#[derive(Debug)]
pub struct ConstantFolding;

impl OptimizerRule for ConstantFolding {
    fn name(&self) -> &'static str {
        "constant_folding"
    }
    fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Filter { input, predicate } => {
                let folded = fold_expr(predicate);
                LogicalPlan::Filter {
                    input: Box::new(self.rewrite(*input)),
                    predicate: folded,
                }
            }
            other => map_children(other, self),
        }
    }
}

#[derive(Debug)]
pub struct EliminateRedundantProjection;

impl OptimizerRule for EliminateRedundantProjection {
    fn name(&self) -> &'static str {
        "eliminate_redundant_projection"
    }
    fn rewrite(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Projection { input, columns } => {
                let inner = self.rewrite(*input);
                if let LogicalPlan::Projection {
                    input: inner_input,
                    columns: _inner_cols,
                } = inner
                {
                    return LogicalPlan::Projection {
                        input: inner_input,
                        columns,
                    };
                }
                LogicalPlan::Projection {
                    input: Box::new(inner),
                    columns,
                }
            }
            other => map_children(other, self),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn map_children<R: OptimizerRule + ?Sized>(plan: LogicalPlan, rule: &R) -> LogicalPlan {
    match plan {
        LogicalPlan::Scan { .. } => plan,
        LogicalPlan::Projection { input, columns } => LogicalPlan::Projection {
            input: Box::new(rule.rewrite(*input)),
            columns,
        },
        LogicalPlan::Filter { input, predicate } => LogicalPlan::Filter {
            input: Box::new(rule.rewrite(*input)),
            predicate,
        },
        LogicalPlan::Limit { input, skip, fetch } => LogicalPlan::Limit {
            input: Box::new(rule.rewrite(*input)),
            skip,
            fetch,
        },
        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => LogicalPlan::Aggregate {
            input: Box::new(rule.rewrite(*input)),
            group_by,
            aggregates,
        },
    }
}

fn predicate_columns(expr: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    walk_expr(expr, &mut |e| {
        if let Expr::Column(c) = e {
            out.push(c.clone());
        }
    });
    out
}

fn walk_expr<F: FnMut(&Expr)>(expr: &Expr, f: &mut F) {
    f(expr);
    if let Expr::Binary { left, right, .. } = expr {
        walk_expr(left, f);
        walk_expr(right, f);
    }
}

fn fold_expr(expr: Expr) -> Expr {
    match expr {
        Expr::Binary { op, left, right } => {
            let left = fold_expr(*left);
            let right = fold_expr(*right);
            match (&left, &right) {
                (Expr::Literal(Value::Int64(a)), Expr::Literal(Value::Int64(b))) => {
                    let folded = match op {
                        BinaryOp::Eq => Some(Value::Bool(a == b)),
                        BinaryOp::NotEq => Some(Value::Bool(a != b)),
                        BinaryOp::Lt => Some(Value::Bool(a < b)),
                        BinaryOp::LtEq => Some(Value::Bool(a <= b)),
                        BinaryOp::Gt => Some(Value::Bool(a > b)),
                        BinaryOp::GtEq => Some(Value::Bool(a >= b)),
                        _ => None,
                    };
                    if let Some(v) = folded {
                        return Expr::Literal(v);
                    }
                }
                (Expr::Literal(Value::Bool(a)), Expr::Literal(Value::Bool(b))) => {
                    let folded = match op {
                        BinaryOp::And => Some(Value::Bool(*a && *b)),
                        BinaryOp::Or => Some(Value::Bool(*a || *b)),
                        BinaryOp::Eq => Some(Value::Bool(a == b)),
                        BinaryOp::NotEq => Some(Value::Bool(a != b)),
                        _ => None,
                    };
                    if let Some(v) = folded {
                        return Expr::Literal(v);
                    }
                }
                _ => {}
            }
            Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> Expr {
        Expr::Column(name.into())
    }

    fn int_lit(n: i64) -> Expr {
        Expr::Literal(Value::Int64(n))
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::Literal(Value::Bool(b))
    }

    #[test]
    fn predicate_pushdown_through_projection() {
        let plan = LogicalPlan::scan("orders")
            .project(vec!["id".into(), "amount".into()])
            .filter(Expr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(col("amount")),
                right: Box::new(int_lit(100)),
            });
        let opt = PredicatePushdown.rewrite(plan);
        assert!(matches!(opt, LogicalPlan::Projection { .. }));
    }

    #[test]
    fn predicate_pushdown_blocked_when_column_dropped() {
        let plan = LogicalPlan::scan("orders")
            .project(vec!["id".into()])
            .filter(Expr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(col("amount")),
                right: Box::new(int_lit(100)),
            });
        let opt = PredicatePushdown.rewrite(plan);
        assert!(matches!(opt, LogicalPlan::Filter { .. }));
    }

    #[test]
    fn projection_pushdown_into_scan() {
        let plan = LogicalPlan::scan("orders").project(vec!["id".into(), "amount".into()]);
        let opt = ProjectionPushdown.rewrite(plan);
        if let LogicalPlan::Scan { projection, .. } = opt {
            assert_eq!(projection, Some(vec!["id".to_string(), "amount".to_string()]));
        } else {
            panic!("expected projection to fold into Scan");
        }
    }

    #[test]
    fn projection_pushdown_no_op_through_filter() {
        let plan = LogicalPlan::scan("orders")
            .filter(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(col("a")),
                right: Box::new(int_lit(1)),
            })
            .project(vec!["a".into()]);
        let opt = ProjectionPushdown.rewrite(plan);
        // The Filter sits between Projection and Scan, so projection cannot
        // fold into Scan; it stays as a Projection node.
        assert!(matches!(opt, LogicalPlan::Projection { .. }));
    }

    #[test]
    fn limit_pushdown_through_projection() {
        let plan = LogicalPlan::scan("orders")
            .project(vec!["id".into()])
            .limit(0, Some(10));
        let opt = LimitPushdown.rewrite(plan);
        if let LogicalPlan::Projection { input, .. } = opt {
            assert!(matches!(*input, LogicalPlan::Limit { .. }));
        } else {
            panic!("expected projection on top of limit");
        }
    }

    #[test]
    fn constant_folding_collapses_int_compare() {
        let pred = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(int_lit(1)),
            right: Box::new(int_lit(1)),
        };
        let plan = LogicalPlan::scan("t").filter(pred);
        let opt = ConstantFolding.rewrite(plan);
        if let LogicalPlan::Filter { predicate, .. } = opt {
            assert_eq!(predicate, bool_lit(true));
        }
    }

    #[test]
    fn constant_folding_int_lt() {
        let pred = Expr::Binary {
            op: BinaryOp::Lt,
            left: Box::new(int_lit(1)),
            right: Box::new(int_lit(2)),
        };
        let plan = LogicalPlan::scan("t").filter(pred);
        let opt = ConstantFolding.rewrite(plan);
        if let LogicalPlan::Filter { predicate, .. } = opt {
            assert_eq!(predicate, bool_lit(true));
        }
    }

    #[test]
    fn constant_folding_and_or() {
        let pred = Expr::Binary {
            op: BinaryOp::And,
            left: Box::new(bool_lit(true)),
            right: Box::new(bool_lit(false)),
        };
        let plan = LogicalPlan::scan("t").filter(pred);
        let opt = ConstantFolding.rewrite(plan);
        if let LogicalPlan::Filter { predicate, .. } = opt {
            assert_eq!(predicate, bool_lit(false));
        }
    }

    #[test]
    fn constant_folding_or_short_circuits() {
        let pred = Expr::Binary {
            op: BinaryOp::Or,
            left: Box::new(bool_lit(true)),
            right: Box::new(bool_lit(false)),
        };
        let plan = LogicalPlan::scan("t").filter(pred);
        let opt = ConstantFolding.rewrite(plan);
        if let LogicalPlan::Filter { predicate, .. } = opt {
            assert_eq!(predicate, bool_lit(true));
        }
    }

    #[test]
    fn eliminate_redundant_projection_collapses_two_layers() {
        let plan = LogicalPlan::Projection {
            input: Box::new(LogicalPlan::Projection {
                input: Box::new(LogicalPlan::scan("t")),
                columns: vec!["a".into(), "b".into()],
            }),
            columns: vec!["a".into()],
        };
        let opt = EliminateRedundantProjection.rewrite(plan);
        if let LogicalPlan::Projection { input, columns } = opt {
            assert_eq!(columns, vec!["a".to_string()]);
            assert!(matches!(*input, LogicalPlan::Scan { .. }));
        }
    }

    #[test]
    fn optimizer_runs_until_fixed_point() {
        let opt = Optimizer::default();
        let plan = LogicalPlan::scan("t")
            .project(vec!["a".into()])
            .filter(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(int_lit(1)),
                right: Box::new(int_lit(1)),
            });
        let result = opt.optimize(plan);
        let again = opt.optimize(result.clone());
        assert_eq!(result, again);
    }

    #[test]
    fn optimizer_combines_predicate_and_projection_pushdown() {
        let opt = Optimizer::default();
        let plan = LogicalPlan::scan("orders")
            .project(vec!["id".into(), "amount".into()])
            .filter(Expr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(col("amount")),
                right: Box::new(int_lit(100)),
            });
        let result = opt.optimize(plan);
        let mut found_scan_projection = false;
        walk_plan(&result, &mut |p| {
            if let LogicalPlan::Scan {
                projection: Some(_),
                ..
            } = p
            {
                found_scan_projection = true;
            }
        });
        assert!(found_scan_projection);
    }

    #[test]
    fn rule_names_unique() {
        let opt = Optimizer::default();
        let names: Vec<&str> = opt.rules.iter().map(|r| r.name()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn predicate_columns_extracts_referenced_columns() {
        let pred = Expr::Binary {
            op: BinaryOp::And,
            left: Box::new(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(col("a")),
                right: Box::new(int_lit(1)),
            }),
            right: Box::new(Expr::Binary {
                op: BinaryOp::Lt,
                left: Box::new(col("b")),
                right: Box::new(int_lit(10)),
            }),
        };
        let mut cols = predicate_columns(&pred);
        cols.sort();
        assert_eq!(cols, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn fold_unrelated_op_is_no_op() {
        // Binary with column and literal — cannot fold.
        let pred = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(col("a")),
            right: Box::new(int_lit(1)),
        };
        let folded = fold_expr(pred.clone());
        assert_eq!(folded, pred);
    }

    fn walk_plan<F: FnMut(&LogicalPlan)>(plan: &LogicalPlan, f: &mut F) {
        f(plan);
        match plan {
            LogicalPlan::Scan { .. } => {}
            LogicalPlan::Projection { input, .. }
            | LogicalPlan::Filter { input, .. }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Aggregate { input, .. } => walk_plan(input, f),
        }
    }
}
