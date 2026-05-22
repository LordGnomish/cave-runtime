// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SELECT statement execution.

use crate::sql::ast::{BinaryOp, Expr, FromClause, Literal, SelectColumn, SelectStmt};
use crate::storage::schema::{Database, Row, Table};
use crate::types::SqlValue;

pub struct SelectResult {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

pub fn execute_select(select: &SelectStmt, db: &Database) -> Result<SelectResult, String> {
    let schema = db.schemas.get("public").ok_or("no public schema")?;

    // Resolve the source table from the FROM clause
    let (table_name, table): (Option<String>, Option<&Table>) = match &select.from {
        None => (None, None),
        Some(from) => match from.as_ref() {
            FromClause::Table(name, _alias) => {
                let t = schema
                    .tables
                    .get(name)
                    .ok_or(format!("table {} not found", name))?;
                (Some(name.clone()), Some(t))
            }
            FromClause::Join { .. } => (None, None), // join not yet supported in eval
        },
    };

    let col_names: Vec<String> = table
        .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default();

    let mut result_rows: Vec<Row> = table.map(|t| t.rows.clone()).unwrap_or_default();

    // Apply WHERE filter
    if let Some(where_expr) = &select.where_clause {
        result_rows.retain(|row| {
            eval_expr(where_expr, row, &col_names)
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });
    }

    // Apply ORDER BY
    if let Some(order_by) = &select.order_by {
        for order in order_by.iter().rev() {
            result_rows.sort_by(|a, b| {
                let av = eval_expr(&order.expr, a, &col_names).ok();
                let bv = eval_expr(&order.expr, b, &col_names).ok();
                match (av, bv) {
                    (Some(av), Some(bv)) => {
                        let cmp = av.compare(&bv).unwrap_or(std::cmp::Ordering::Equal);
                        if order.descending { cmp.reverse() } else { cmp }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });
        }
    }

    // Apply OFFSET and LIMIT
    let offset = select.offset.unwrap_or(0) as usize;
    if offset >= result_rows.len() {
        result_rows.clear();
    } else if offset > 0 {
        result_rows = result_rows[offset..].to_vec();
    }
    if let Some(limit) = select.limit {
        result_rows.truncate(limit as usize);
    }

    // Determine output column names and project rows
    let (output_cols, output_rows) = project(select, &col_names, result_rows, table);

    // Ignore unused variable warning for table_name
    let _ = table_name;

    Ok(SelectResult {
        columns: output_cols,
        rows: output_rows,
    })
}

fn project(
    select: &SelectStmt,
    col_names: &[String],
    rows: Vec<Row>,
    table: Option<&Table>,
) -> (Vec<String>, Vec<Row>) {
    let is_star = matches!(select.columns.first(), Some(SelectColumn::Star));

    if is_star {
        return (col_names.to_vec(), rows);
    }

    // Named column projection
    let output_cols: Vec<String> = select
        .columns
        .iter()
        .map(|sc| match sc {
            SelectColumn::Star => "*".to_string(),
            SelectColumn::TableStar(t) => format!("{}.*", t),
            SelectColumn::Expr(_, Some(alias)) => alias.clone(),
            SelectColumn::Expr(Expr::Identifier(name), None) => name.clone(),
            SelectColumn::Expr(Expr::QualifiedIdentifier(_, col), None) => col.clone(),
            SelectColumn::Expr(_, None) => "?column?".to_string(),
        })
        .collect();

    let output_rows: Vec<Row> = rows
        .into_iter()
        .map(|row| {
            select
                .columns
                .iter()
                .map(|sc| match sc {
                    SelectColumn::Expr(Expr::Identifier(name), _) => col_names
                        .iter()
                        .position(|c| c == name)
                        .and_then(|i| row.get(i))
                        .cloned()
                        .unwrap_or(SqlValue::Null),
                    SelectColumn::Expr(Expr::QualifiedIdentifier(_, col), _) => col_names
                        .iter()
                        .position(|c| c == col)
                        .and_then(|i| row.get(i))
                        .cloned()
                        .unwrap_or(SqlValue::Null),
                    SelectColumn::Expr(expr, _) => {
                        eval_expr(expr, &row, col_names).unwrap_or(SqlValue::Null)
                    }
                    _ => SqlValue::Null,
                })
                .collect()
        })
        .collect();

    let _ = table; // may be used for type metadata in future
    (output_cols, output_rows)
}

fn eval_expr(expr: &Expr, row: &Row, col_names: &[String]) -> Result<SqlValue, String> {
    match expr {
        Expr::Literal(lit) => Ok(match lit {
            Literal::Null => SqlValue::Null,
            Literal::Integer(n) => SqlValue::Int4(*n as i32),
            Literal::Float(f) => SqlValue::Numeric(*f),
            Literal::String(s) => SqlValue::Text(s.clone()),
            Literal::Boolean(b) => SqlValue::Bool(*b),
            Literal::Date(s) => SqlValue::Date(s.clone()),
            Literal::Timestamp(s) => SqlValue::Timestamp(s.clone()),
        }),
        Expr::Identifier(name) => {
            let idx = col_names
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("column '{}' not found", name))?;
            Ok(row.get(idx).cloned().unwrap_or(SqlValue::Null))
        }
        Expr::QualifiedIdentifier(_, col) => {
            let idx = col_names
                .iter()
                .position(|c| c.eq_ignore_ascii_case(col))
                .ok_or_else(|| format!("column '{}' not found", col))?;
            Ok(row.get(idx).cloned().unwrap_or(SqlValue::Null))
        }
        Expr::BinaryOp { left, op, right } => {
            let lval = eval_expr(left, row, col_names)?;
            let rval = eval_expr(right, row, col_names)?;
            eval_binop(&lval, *op, &rval)
        }
        Expr::IsNull { expr, not } => {
            let val = eval_expr(expr, row, col_names)?;
            let is_null = matches!(val, SqlValue::Null);
            Ok(SqlValue::Bool(if *not { !is_null } else { is_null }))
        }
        _ => Err(format!("unsupported expression type in eval")),
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
        (SqlValue::Text(a), BinaryOp::Ne, SqlValue::Text(b)) => Ok(SqlValue::Bool(a != b)),
        (SqlValue::Text(a), BinaryOp::Like, SqlValue::Text(b)) => {
            let pattern = b.replace('%', ".*").replace('_', ".");
            Ok(SqlValue::Bool(
                regex::Regex::new(&format!("^{}$", pattern))
                    .map(|re| re.is_match(a))
                    .unwrap_or(false),
            ))
        }
        (a, BinaryOp::And, b) => Ok(SqlValue::Bool(
            a.as_bool().unwrap_or(false) && b.as_bool().unwrap_or(false),
        )),
        (a, BinaryOp::Or, b) => Ok(SqlValue::Bool(
            a.as_bool().unwrap_or(false) || b.as_bool().unwrap_or(false),
        )),
        _ => Err(format!(
            "unsupported binop: {:?} {:?} {:?}",
            left, op, right
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::ast::{FromClause, SelectStmt};
    use crate::storage::schema::ColumnDef;

    fn make_db_with_users() -> Database {
        let mut db = Database::new("test");
        let schema = db.schemas.get_mut("public").unwrap();
        let cols = vec![
            ColumnDef {
                name: "id".into(),
                type_name: "int".into(),
                not_null: true,
                primary_key: true,
            },
            ColumnDef {
                name: "name".into(),
                type_name: "text".into(),
                not_null: false,
                primary_key: false,
            },
            ColumnDef {
                name: "age".into(),
                type_name: "int".into(),
                not_null: false,
                primary_key: false,
            },
        ];
        let mut table = Table::new("users", cols);
        table.rows.push(vec![
            SqlValue::Int4(1),
            SqlValue::Text("alice".into()),
            SqlValue::Int4(30),
        ]);
        table.rows.push(vec![
            SqlValue::Int4(2),
            SqlValue::Text("bob".into()),
            SqlValue::Int4(25),
        ]);
        table.rows.push(vec![
            SqlValue::Int4(3),
            SqlValue::Text("carol".into()),
            SqlValue::Int4(35),
        ]);
        schema.tables.insert("users".into(), table);
        db
    }

    fn select_star(table: &str) -> SelectStmt {
        SelectStmt {
            distinct: false,
            columns: vec![SelectColumn::Star],
            from: Some(Box::new(FromClause::Table(table.into(), None))),
            where_clause: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            offset: None,
        }
    }

    #[test]
    fn test_select_star_returns_all_rows() {
        let db = make_db_with_users();
        let stmt = select_star("users");
        let result = execute_select(&stmt, &db).unwrap();
        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.columns, vec!["id", "name", "age"]);
    }

    #[test]
    fn test_select_with_where_identifier() {
        let db = make_db_with_users();
        let mut stmt = select_star("users");
        stmt.where_clause = Some(Box::new(Expr::BinaryOp {
            left: Box::new(Expr::Identifier("age".into())),
            op: BinaryOp::Gt,
            right: Box::new(Expr::Literal(Literal::Integer(28))),
        }));
        let result = execute_select(&stmt, &db).unwrap();
        assert_eq!(result.rows.len(), 2); // alice (30) and carol (35)
    }

    #[test]
    fn test_select_named_columns() {
        let db = make_db_with_users();
        let stmt = SelectStmt {
            distinct: false,
            columns: vec![
                SelectColumn::Expr(Expr::Identifier("id".into()), None),
                SelectColumn::Expr(Expr::Identifier("name".into()), None),
            ],
            from: Some(Box::new(FromClause::Table("users".into(), None))),
            where_clause: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            offset: None,
        };
        let result = execute_select(&stmt, &db).unwrap();
        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.rows[0].len(), 2);
    }

    #[test]
    fn test_select_with_limit() {
        let db = make_db_with_users();
        let mut stmt = select_star("users");
        stmt.limit = Some(2);
        let result = execute_select(&stmt, &db).unwrap();
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn test_select_with_offset() {
        let db = make_db_with_users();
        let mut stmt = select_star("users");
        stmt.offset = Some(1);
        let result = execute_select(&stmt, &db).unwrap();
        assert_eq!(result.rows.len(), 2); // rows 2 and 3
    }

    #[test]
    fn test_eval_binop_int_eq() {
        let a = SqlValue::Int4(5);
        let b = SqlValue::Int4(5);
        assert_eq!(
            eval_binop(&a, BinaryOp::Eq, &b).unwrap(),
            SqlValue::Bool(true)
        );
    }

    #[test]
    fn test_eval_binop_int_lt() {
        let a = SqlValue::Int4(3);
        let b = SqlValue::Int4(5);
        assert_eq!(
            eval_binop(&a, BinaryOp::Lt, &b).unwrap(),
            SqlValue::Bool(true)
        );
    }

    #[test]
    fn test_eval_binop_and() {
        assert_eq!(
            eval_binop(&SqlValue::Bool(true), BinaryOp::And, &SqlValue::Bool(false)).unwrap(),
            SqlValue::Bool(false)
        );
    }
}
