//! SELECT statement execution.

use crate::sql::ast::{SelectStmt, SelectColumn, Expr, BinaryOp};
use crate::storage::schema::{Database, Row};
use crate::types::SqlValue;

pub struct SelectResult {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

pub fn execute_select(
    select: &SelectStmt,
    db: &Database,
) -> Result<SelectResult, String> {
    let schema = db.schemas.get("public").ok_or("no public schema")?;

    // Determine source table (stub version)
    let mut result_rows: Vec<Row> = Vec::new();

    // For simple SELECT *, return demo data
    if matches!(select.columns.first(), Some(SelectColumn::Star)) {
        // Try to find first table
        if let Some((_, table)) = schema.tables.iter().next() {
            result_rows = table.rows.clone();
        }
    }

    // Apply WHERE filter
    if let Some(where_clause) = &select.where_clause {
        result_rows.retain(|row| {
            eval_expr(where_clause, row, &schema.tables)
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });
    }

    // Apply ORDER BY
    if let Some(order_by) = &select.order_by {
        for order in order_by.iter().rev() {
            result_rows.sort_by(|a, b| {
                let a_val = eval_expr(&order.expr, a, &schema.tables).ok();
                let b_val = eval_expr(&order.expr, b, &schema.tables).ok();
                match (a_val, b_val) {
                    (Some(av), Some(bv)) => {
                        let cmp = av.compare(&bv).unwrap_or(std::cmp::Ordering::Equal);
                        if order.descending {
                            cmp.reverse()
                        } else {
                            cmp
                        }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });
        }
    }

    // Apply LIMIT and OFFSET
    let offset = select.offset.unwrap_or(0) as usize;
    let limit = select.limit.map(|l| l as usize);
    if offset > 0 && offset < result_rows.len() {
        result_rows = result_rows[offset..].to_vec();
    } else if offset >= result_rows.len() {
        result_rows.clear();
    }
    if let Some(l) = limit {
        result_rows.truncate(l);
    }

    let column_names = vec!["col".to_string()]; // TODO: extract from select.columns
    Ok(SelectResult {
        columns: column_names,
        rows: result_rows,
    })
}

fn eval_expr(expr: &Expr, _row: &Row, _tables: &std::collections::HashMap<String, crate::storage::schema::Table>) -> Result<SqlValue, String> {
    match expr {
        Expr::Literal(lit) => {
            use crate::sql::ast::Literal;
            Ok(match lit {
                Literal::Null => SqlValue::Null,
                Literal::Integer(n) => SqlValue::Int4(*n as i32),
                Literal::Float(f) => SqlValue::Numeric(*f),
                Literal::String(s) => SqlValue::Text(s.clone()),
                Literal::Boolean(b) => SqlValue::Bool(*b),
                Literal::Date(s) => SqlValue::Date(s.clone()),
                Literal::Timestamp(s) => SqlValue::Timestamp(s.clone()),
            })
        }
        Expr::BinaryOp { left, op, right } => {
            let lval = eval_expr(left, _row, _tables)?;
            let rval = eval_expr(right, _row, _tables)?;
            eval_binop(&lval, *op, &rval)
        }
        _ => Err("unsupported expression in WHERE".to_string()),
    }
}

fn eval_binop(left: &SqlValue, op: BinaryOp, right: &SqlValue) -> Result<SqlValue, String> {
    match (left, op, right) {
        (SqlValue::Int4(a), BinaryOp::Eq, SqlValue::Int4(b)) => Ok(SqlValue::Bool(a == b)),
        (SqlValue::Int4(a), BinaryOp::Ne, SqlValue::Int4(b)) => Ok(SqlValue::Bool(a != b)),
        (SqlValue::Int4(a), BinaryOp::Lt, SqlValue::Int4(b)) => Ok(SqlValue::Bool(a < b)),
        (SqlValue::Int4(a), BinaryOp::Gt, SqlValue::Int4(b)) => Ok(SqlValue::Bool(a > b)),
        (SqlValue::Int4(a), BinaryOp::Le, SqlValue::Int4(b)) => Ok(SqlValue::Bool(a <= b)),
        (SqlValue::Int4(a), BinaryOp::Ge, SqlValue::Int4(b)) => Ok(SqlValue::Bool(a >= b)),
        (SqlValue::Text(a), BinaryOp::Eq, SqlValue::Text(b)) => Ok(SqlValue::Bool(a == b)),
        (SqlValue::Text(a), BinaryOp::Like, SqlValue::Text(b)) => {
            let regex_pattern = b.replace("%", ".*").replace("_", ".");
            Ok(SqlValue::Bool(
                regex::Regex::new(&format!("^{}$", regex_pattern))
                    .map(|re| re.is_match(a))
                    .unwrap_or(false),
            ))
        }
        (a, BinaryOp::And, b) => {
            let ab = a.as_bool().unwrap_or(false);
            let bb = b.as_bool().unwrap_or(false);
            Ok(SqlValue::Bool(ab && bb))
        }
        (a, BinaryOp::Or, b) => {
            let ab = a.as_bool().unwrap_or(false);
            let bb = b.as_bool().unwrap_or(false);
            Ok(SqlValue::Bool(ab || bb))
        }
        _ => Err(format!("unsupported binop: {:?} {:?} {:?}", left, op, right)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_binop_int_eq() {
        let a = SqlValue::Int4(5);
        let b = SqlValue::Int4(5);
        let result = eval_binop(&a, BinaryOp::Eq, &b).unwrap();
        assert_eq!(result, SqlValue::Bool(true));
    }

    #[test]
    fn test_eval_binop_int_lt() {
        let a = SqlValue::Int4(3);
        let b = SqlValue::Int4(5);
        let result = eval_binop(&a, BinaryOp::Lt, &b).unwrap();
        assert_eq!(result, SqlValue::Bool(true));
    }

    #[test]
    fn test_eval_binop_and() {
        let a = SqlValue::Bool(true);
        let b = SqlValue::Bool(false);
        let result = eval_binop(&a, BinaryOp::And, &b).unwrap();
        assert_eq!(result, SqlValue::Bool(false));
    }
}
