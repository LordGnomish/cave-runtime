// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! UPDATE statement execution.

use crate::sql::ast::{Expr, Literal, SelectColumn, UpdateStmt};
use crate::storage::schema::{Database, Row};
use crate::types::SqlValue;

pub fn execute_update(update: &UpdateStmt, db: &mut Database) -> Result<u64, String> {
    let (n, _) = execute_update_inner(update, db, false)?;
    Ok(n)
}

/// UPDATE … RETURNING — Postgres-compat extension. Returns (affected, rows).
pub fn execute_update_returning(
    update: &UpdateStmt,
    db: &mut Database,
) -> Result<(u64, Vec<Row>), String> {
    execute_update_inner(update, db, true)
}

fn execute_update_inner(
    update: &UpdateStmt,
    db: &mut Database,
    capture_returning: bool,
) -> Result<(u64, Vec<Row>), String> {
    let schema = db.schemas.get_mut("public").ok_or("no public schema")?;
    let table = schema
        .tables
        .get_mut(&update.table)
        .ok_or(format!("table {} not found", update.table))?;

    let col_indices: Vec<(usize, SqlValue)> = update
        .assignments
        .iter()
        .filter_map(|(col_name, expr)| {
            table
                .column_index(col_name)
                .and_then(|idx| expr_to_value(expr).ok().map(|val| (idx, val)))
        })
        .collect();

    let mut updated = 0u64;
    let mut returning_rows: Vec<Row> = Vec::new();
    // Snapshot column-name lookup so we can project RETURNING without
    // re-borrowing the table while it's mutably iterated.
    let col_lookup: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
    if update.where_clause.is_none() {
        for row in &mut table.rows {
            for (idx, val) in &col_indices {
                if *idx < row.len() {
                    row[*idx] = val.clone();
                }
            }
            updated += 1;
            if capture_returning {
                returning_rows.push(project_returning_row(
                    update.returning.as_deref(),
                    &col_lookup,
                    row,
                )?);
            }
        }
    }
    Ok((updated, returning_rows))
}

pub(crate) fn project_returning_row(
    returning: Option<&[SelectColumn]>,
    col_lookup: &[String],
    row: &Row,
) -> Result<Row, String> {
    let Some(cols) = returning else {
        return Ok(row.clone());
    };
    let mut out = Vec::with_capacity(cols.len());
    for col in cols {
        match col {
            SelectColumn::Star | SelectColumn::TableStar(_) => return Ok(row.clone()),
            SelectColumn::Expr(Expr::Identifier(name), _) => {
                let idx = col_lookup
                    .iter()
                    .position(|n| n == name)
                    .ok_or_else(|| format!("RETURNING column {} not found", name))?;
                out.push(row.get(idx).cloned().unwrap_or(SqlValue::Null));
            }
            _ => return Err("only column references allowed in RETURNING".to_string()),
        }
    }
    Ok(out)
}

fn expr_to_value(expr: &crate::sql::ast::Expr) -> Result<SqlValue, String> {
    use crate::sql::ast::Expr;
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
        _ => Err("complex expressions in UPDATE not yet supported".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::ast::Expr;
    use crate::storage::schema::ColumnDef;

    #[test]
    fn test_update_simple() {
        let mut db = Database::new("test");
        let schema = db.schemas.get_mut("public").unwrap();
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        let mut table = crate::storage::schema::Table::new("users", cols);
        table.rows.push(vec![SqlValue::Int4(1)]);
        schema.tables.insert("users".to_string(), table);

        let update = UpdateStmt {
            table: "users".to_string(),
            assignments: vec![("id".to_string(), Expr::Literal(Literal::Integer(2)))],
            where_clause: None,
            returning: None,
        };
        let result = execute_update(&update, &mut db);
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_returning_projects_specific_column() {
        let mut db = Database::new("test");
        let schema = db.schemas.get_mut("public").unwrap();
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        let mut table = crate::storage::schema::Table::new("nums", cols);
        table.rows.push(vec![SqlValue::Int4(1)]);
        table.rows.push(vec![SqlValue::Int4(2)]);
        schema.tables.insert("nums".to_string(), table);

        let update = UpdateStmt {
            table: "nums".to_string(),
            assignments: vec![("id".to_string(), Expr::Literal(Literal::Integer(99)))],
            where_clause: None,
            returning: Some(vec![SelectColumn::Expr(
                Expr::Identifier("id".to_string()),
                None,
            )]),
        };
        let (n, rows) = execute_update_returning(&update, &mut db).unwrap();
        assert_eq!(n, 2);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0][0], SqlValue::Int4(99)));
        assert!(matches!(rows[1][0], SqlValue::Int4(99)));
    }
}
