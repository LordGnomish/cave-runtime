//! SQL → LogicalPlan bridge using `sqlparser` (PostgreSQL dialect).
//!
//! Supported subset (extends the engine on demand):
//!
//! * `SELECT col, ... FROM table [WHERE pred] [ORDER BY] [LIMIT n] [OFFSET m]`
//! * `SELECT * FROM table`
//! * `SELECT COUNT(*), SUM(col), AVG(col), MIN(col), MAX(col) FROM table
//!    [GROUP BY col]`

use sqlparser::ast::{
    BinaryOperator, DuplicateTreatment, Expr as SqlExpr, FunctionArg, FunctionArgExpr,
    FunctionArguments, Query, SelectItem, SetExpr, Statement, TableFactor, Value as SqlValue,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::batch::Value;
use crate::error::{DataFusionError, DfResult};
use crate::expr::{BinaryOp, Expr};
use crate::logical_plan::{AggregateExpr, AggregateFunc, LogicalPlan, SortExpr};

pub fn parse_sql(sql: &str) -> DfResult<LogicalPlan> {
    let stmts = Parser::parse_sql(&PostgreSqlDialect {}, sql)
        .map_err(|e| DataFusionError::Plan(format!("parse error: {}", e)))?;
    let stmt = stmts
        .into_iter()
        .next()
        .ok_or_else(|| DataFusionError::Plan("empty SQL".into()))?;
    plan_statement(stmt)
}

fn plan_statement(stmt: Statement) -> DfResult<LogicalPlan> {
    let q = match stmt {
        Statement::Query(q) => *q,
        other => {
            return Err(DataFusionError::Plan(format!(
                "only SELECT supported, got {:?}",
                other
            )))
        }
    };
    plan_query(q)
}

fn plan_query(q: Query) -> DfResult<LogicalPlan> {
    let select = match *q.body {
        SetExpr::Select(s) => *s,
        other => {
            return Err(DataFusionError::Plan(format!(
                "only SELECT supported, got {:?}",
                other
            )))
        }
    };

    if select.from.len() != 1 {
        return Err(DataFusionError::Plan("only single-table FROM".into()));
    }
    let table_name = match &select.from[0].relation {
        TableFactor::Table { name, .. } => name
            .0
            .last()
            .map(|p| p.value.clone())
            .unwrap_or_default(),
        _ => return Err(DataFusionError::Plan("only literal table refs".into())),
    };

    // Aggregates?
    let mut aggregates = Vec::new();
    let mut projection_cols: Vec<String> = Vec::new();
    let mut has_star = false;
    for item in &select.projection {
        match item {
            SelectItem::Wildcard(_) => has_star = true,
            SelectItem::QualifiedWildcard(_, _) => has_star = true,
            SelectItem::UnnamedExpr(e) => {
                if let Some(agg) = parse_aggregate(e)? {
                    aggregates.push(agg);
                } else if let SqlExpr::Identifier(id) = e {
                    projection_cols.push(id.value.clone());
                } else {
                    return Err(DataFusionError::Plan(format!("unsupported select expr: {}", e)));
                }
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                if let Some(mut agg) = parse_aggregate(expr)? {
                    agg.output_name = alias.value.clone();
                    aggregates.push(agg);
                } else if let SqlExpr::Identifier(id) = expr {
                    projection_cols.push(id.value.clone());
                    let _ = alias;
                } else {
                    return Err(DataFusionError::Plan("unsupported aliased expr".into()));
                }
            }
        }
    }

    let group_by_cols: Vec<String> = match &select.group_by {
        sqlparser::ast::GroupByExpr::All(_) => Vec::new(),
        sqlparser::ast::GroupByExpr::Expressions(exprs, _) => exprs
            .iter()
            .filter_map(|e| match e {
                SqlExpr::Identifier(id) => Some(id.value.clone()),
                _ => None,
            })
            .collect(),
    };

    let mut plan = LogicalPlan::scan(table_name);

    if let Some(pred) = select.selection {
        let expr = plan_expr(pred)?;
        plan = plan.filter(expr);
    }

    if !aggregates.is_empty() || !group_by_cols.is_empty() {
        plan = plan.aggregate(group_by_cols, aggregates);
    } else if !has_star && !projection_cols.is_empty() {
        plan = plan.project(projection_cols);
    }

    // ORDER BY → Sort. Only column references with optional ASC/DESC are
    // supported; expressions in ORDER BY (e.g. `ORDER BY a + b`) error out.
    if let Some(order_by) = &q.order_by {
        if !order_by.exprs.is_empty() {
            let mut sort_exprs = Vec::with_capacity(order_by.exprs.len());
            for ob in &order_by.exprs {
                let column = match &ob.expr {
                    SqlExpr::Identifier(id) => id.value.clone(),
                    other => {
                        return Err(DataFusionError::Plan(format!(
                            "ORDER BY only supports column refs, got {:?}",
                            other
                        )))
                    }
                };
                let ascending = ob.asc.unwrap_or(true);
                sort_exprs.push(SortExpr { column, ascending });
            }
            plan = plan.sort(sort_exprs);
        }
    }

    if let Some(SqlExpr::Value(SqlValue::Number(n, _))) = q.limit {
        let n = n.parse::<usize>().unwrap_or(0);
        let off = q
            .offset
            .as_ref()
            .and_then(|o| match &o.value {
                SqlExpr::Value(SqlValue::Number(n, _)) => n.parse::<usize>().ok(),
                _ => None,
            })
            .unwrap_or(0);
        plan = plan.limit(off, Some(n));
    }

    Ok(plan)
}

fn parse_aggregate(expr: &SqlExpr) -> DfResult<Option<AggregateExpr>> {
    if let SqlExpr::Function(f) = expr {
        let name = f
            .name
            .0
            .last()
            .map(|p| p.value.to_uppercase())
            .unwrap_or_default();
        let mut func = match name.as_str() {
            "COUNT" => AggregateFunc::Count,
            "SUM" => AggregateFunc::Sum,
            "MIN" => AggregateFunc::Min,
            "MAX" => AggregateFunc::Max,
            "AVG" => AggregateFunc::Avg,
            _ => return Ok(None),
        };
        // Detect DISTINCT inside the call list (e.g. COUNT(DISTINCT col)).
        let is_distinct = match &f.args {
            FunctionArguments::List(list) => matches!(
                list.duplicate_treatment,
                Some(DuplicateTreatment::Distinct)
            ),
            _ => false,
        };
        if is_distinct {
            if func == AggregateFunc::Count {
                func = AggregateFunc::CountDistinct;
            } else {
                return Err(DataFusionError::Plan(format!(
                    "DISTINCT only supported on COUNT, got {}",
                    name
                )));
            }
        }
        let column = match &f.args {
            FunctionArguments::List(list) => list.args.first().and_then(|a| match a {
                FunctionArg::Unnamed(FunctionArgExpr::Wildcard) => None,
                FunctionArg::Unnamed(FunctionArgExpr::Expr(SqlExpr::Identifier(id))) => {
                    Some(id.value.clone())
                }
                _ => None,
            }),
            _ => None,
        };
        let output = if is_distinct {
            match &column {
                Some(col) => format!("count(distinct {})", col),
                None => "count(distinct *)".into(),
            }
        } else {
            match &column {
                Some(col) => format!("{}({})", name.to_lowercase(), col),
                None => format!("{}(*)", name.to_lowercase()),
            }
        };
        return Ok(Some(AggregateExpr {
            func,
            column,
            output_name: output,
        }));
    }
    Ok(None)
}

fn plan_expr(e: SqlExpr) -> DfResult<Expr> {
    match e {
        SqlExpr::Identifier(id) => Ok(Expr::Column(id.value)),
        SqlExpr::Value(v) => Ok(Expr::Literal(value_to_scalar(v)?)),
        SqlExpr::BinaryOp { left, op, right } => {
            let l = plan_expr(*left)?;
            let r = plan_expr(*right)?;
            let pop = match op {
                BinaryOperator::Eq => BinaryOp::Eq,
                BinaryOperator::NotEq => BinaryOp::NotEq,
                BinaryOperator::Lt => BinaryOp::Lt,
                BinaryOperator::LtEq => BinaryOp::LtEq,
                BinaryOperator::Gt => BinaryOp::Gt,
                BinaryOperator::GtEq => BinaryOp::GtEq,
                BinaryOperator::And => BinaryOp::And,
                BinaryOperator::Or => BinaryOp::Or,
                other => {
                    return Err(DataFusionError::Plan(format!(
                        "unsupported binary op {:?}",
                        other
                    )))
                }
            };
            Ok(Expr::Binary {
                left: Box::new(l),
                op: pop,
                right: Box::new(r),
            })
        }
        SqlExpr::Nested(e) => plan_expr(*e),
        other => Err(DataFusionError::Plan(format!("unsupported expr: {:?}", other))),
    }
}

fn value_to_scalar(v: SqlValue) -> DfResult<Value> {
    Ok(match v {
        SqlValue::Number(n, _) => {
            if let Ok(i) = n.parse::<i64>() {
                Value::Int64(i)
            } else if let Ok(f) = n.parse::<f64>() {
                Value::Float64(f)
            } else {
                return Err(DataFusionError::Plan(format!("bad number {}", n)));
            }
        }
        SqlValue::SingleQuotedString(s) | SqlValue::DoubleQuotedString(s) => Value::Utf8(s),
        SqlValue::Boolean(b) => Value::Bool(b),
        SqlValue::Null => Value::Null,
        other => return Err(DataFusionError::Plan(format!("bad literal {:?}", other))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_select_star() {
        let p = parse_sql("SELECT * FROM users").unwrap();
        match p {
            LogicalPlan::Scan { table_name, .. } => assert_eq!(table_name, "users"),
            other => panic!("expected Scan, got {:?}", other),
        }
    }

    #[test]
    fn parse_select_columns() {
        let p = parse_sql("SELECT id, name FROM users").unwrap();
        match p {
            LogicalPlan::Projection { columns, .. } => {
                assert_eq!(columns, vec!["id".to_string(), "name".to_string()])
            }
            other => panic!("expected Projection, got {:?}", other),
        }
    }

    #[test]
    fn parse_select_with_where() {
        let p = parse_sql("SELECT * FROM t WHERE id = 1").unwrap();
        assert!(matches!(p, LogicalPlan::Filter { .. }));
    }

    #[test]
    fn parse_select_with_limit_offset() {
        let p = parse_sql("SELECT * FROM t LIMIT 5 OFFSET 10").unwrap();
        match p {
            LogicalPlan::Limit { skip, fetch, .. } => {
                assert_eq!(skip, 10);
                assert_eq!(fetch, Some(5));
            }
            other => panic!("expected Limit, got {:?}", other),
        }
    }

    #[test]
    fn parse_count_star_aggregate() {
        let p = parse_sql("SELECT COUNT(*) FROM t").unwrap();
        match p {
            LogicalPlan::Aggregate { aggregates, .. } => {
                assert_eq!(aggregates[0].func, AggregateFunc::Count);
                assert_eq!(aggregates[0].column, None);
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_sum_aggregate_with_column() {
        let p = parse_sql("SELECT SUM(price) FROM orders").unwrap();
        match p {
            LogicalPlan::Aggregate { aggregates, .. } => {
                assert_eq!(aggregates[0].func, AggregateFunc::Sum);
                assert_eq!(aggregates[0].column.as_deref(), Some("price"));
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_avg_aggregate() {
        let p = parse_sql("SELECT AVG(score) FROM exams").unwrap();
        match p {
            LogicalPlan::Aggregate { aggregates, .. } => {
                assert_eq!(aggregates[0].func, AggregateFunc::Avg);
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_min_max() {
        let p = parse_sql("SELECT MIN(price), MAX(price) FROM orders").unwrap();
        match p {
            LogicalPlan::Aggregate { aggregates, .. } => {
                assert_eq!(aggregates.len(), 2);
                assert_eq!(aggregates[0].func, AggregateFunc::Min);
                assert_eq!(aggregates[1].func, AggregateFunc::Max);
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_group_by() {
        let p = parse_sql("SELECT COUNT(*) FROM t GROUP BY category").unwrap();
        match p {
            LogicalPlan::Aggregate { group_by, .. } => {
                assert_eq!(group_by, vec!["category".to_string()]);
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_aliased_aggregate_uses_alias() {
        let p = parse_sql("SELECT COUNT(*) AS total FROM t").unwrap();
        match p {
            LogicalPlan::Aggregate { aggregates, .. } => {
                assert_eq!(aggregates[0].output_name, "total");
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_unsupported_statement_errors() {
        let err = parse_sql("CREATE TABLE t (id INT)").unwrap_err();
        match err {
            DataFusionError::Plan(_) => {}
            other => panic!("expected Plan error, got {:?}", other),
        }
    }

    #[test]
    fn parse_invalid_sql_errors() {
        assert!(parse_sql("NOT VALID SQL HERE").is_err());
    }

    #[test]
    fn parse_string_literal_in_where() {
        let p = parse_sql("SELECT * FROM t WHERE name = 'alice'").unwrap();
        if let LogicalPlan::Filter { predicate, .. } = p {
            if let Expr::Binary { right, .. } = predicate {
                if let Expr::Literal(Value::Utf8(s)) = right.as_ref() {
                    assert_eq!(s, "alice");
                    return;
                }
            }
        }
        panic!("predicate not parsed correctly");
    }

    // ── ORDER BY ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_order_by_default_asc() {
        let p = parse_sql("SELECT * FROM t ORDER BY id").unwrap();
        match p {
            LogicalPlan::Sort { exprs, .. } => {
                assert_eq!(exprs.len(), 1);
                assert_eq!(exprs[0].column, "id");
                assert!(exprs[0].ascending);
            }
            other => panic!("expected Sort, got {:?}", other),
        }
    }

    #[test]
    fn parse_order_by_desc() {
        let p = parse_sql("SELECT * FROM t ORDER BY id DESC").unwrap();
        match p {
            LogicalPlan::Sort { exprs, .. } => {
                assert!(!exprs[0].ascending);
            }
            other => panic!("expected Sort, got {:?}", other),
        }
    }

    #[test]
    fn parse_order_by_multi_column() {
        let p = parse_sql("SELECT * FROM t ORDER BY a ASC, b DESC").unwrap();
        match p {
            LogicalPlan::Sort { exprs, .. } => {
                assert_eq!(exprs.len(), 2);
                assert_eq!(exprs[0].column, "a");
                assert!(exprs[0].ascending);
                assert_eq!(exprs[1].column, "b");
                assert!(!exprs[1].ascending);
            }
            other => panic!("expected Sort, got {:?}", other),
        }
    }

    #[test]
    fn parse_order_by_then_limit() {
        // ORDER BY must wrap the inner plan; LIMIT wraps the Sort.
        let p = parse_sql("SELECT * FROM t ORDER BY id LIMIT 5").unwrap();
        match p {
            LogicalPlan::Limit { input, fetch, .. } => {
                assert_eq!(fetch, Some(5));
                assert!(matches!(*input, LogicalPlan::Sort { .. }));
            }
            other => panic!("expected Limit, got {:?}", other),
        }
    }

    // ── COUNT(DISTINCT) ───────────────────────────────────────────────────────

    #[test]
    fn parse_count_distinct() {
        let p = parse_sql("SELECT COUNT(DISTINCT dept) FROM emp").unwrap();
        match p {
            LogicalPlan::Aggregate { aggregates, .. } => {
                assert_eq!(aggregates[0].func, AggregateFunc::CountDistinct);
                assert_eq!(aggregates[0].column.as_deref(), Some("dept"));
            }
            other => panic!("expected Aggregate, got {:?}", other),
        }
    }

    #[test]
    fn parse_distinct_on_sum_errors() {
        // DISTINCT only supported on COUNT in this subset.
        assert!(parse_sql("SELECT SUM(DISTINCT salary) FROM emp").is_err());
    }

    #[test]
    fn parse_complex_where_with_and() {
        let p = parse_sql("SELECT * FROM t WHERE a = 1 AND b > 5").unwrap();
        if let LogicalPlan::Filter { predicate, .. } = p {
            if let Expr::Binary { op, .. } = predicate {
                assert_eq!(op, BinaryOp::And);
                return;
            }
        }
        panic!("expected AND in predicate");
    }
}
