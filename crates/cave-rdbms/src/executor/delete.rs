// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DELETE statement execution.

use crate::executor::update::project_returning_row;
use crate::sql::ast::DeleteStmt;
use crate::storage::schema::{Database, Row};

pub fn execute_delete(delete: &DeleteStmt, db: &mut Database) -> Result<u64, String> {
    let (n, _) = execute_delete_inner(delete, db, false)?;
    Ok(n)
}

/// DELETE … RETURNING — Postgres-compat extension. Returns (affected, rows).
pub fn execute_delete_returning(
    delete: &DeleteStmt,
    db: &mut Database,
) -> Result<(u64, Vec<Row>), String> {
    execute_delete_inner(delete, db, true)
}

fn execute_delete_inner(
    delete: &DeleteStmt,
    db: &mut Database,
    capture_returning: bool,
) -> Result<(u64, Vec<Row>), String> {
    let schema = db.schemas.get_mut("public").ok_or("no public schema")?;
    let table = schema
        .tables
        .get_mut(&delete.table)
        .ok_or(format!("table {} not found", delete.table))?;

    let col_lookup: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
    let mut returning_rows: Vec<Row> = Vec::new();
    let initial_count = table.rows.len();
    if delete.where_clause.is_none() {
        if capture_returning {
            for row in &table.rows {
                returning_rows.push(project_returning_row(
                    delete.returning.as_deref(),
                    &col_lookup,
                    row,
                )?);
            }
        }
        table.rows.clear();
    }
    let deleted = (initial_count - table.rows.len()) as u64;
    Ok((deleted, returning_rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::ColumnDef;
    use crate::types::SqlValue;

    #[test]
    fn test_delete_simple() {
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

        let delete = DeleteStmt {
            table: "users".to_string(),
            where_clause: None,
            returning: None,
        };
        let result = execute_delete(&delete, &mut db);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_delete_returning_yields_deleted_rows() {
        use crate::sql::ast::{Expr, SelectColumn};
        let mut db = Database::new("test");
        let schema = db.schemas.get_mut("public").unwrap();
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        let mut table = crate::storage::schema::Table::new("nums", cols);
        table.rows.push(vec![SqlValue::Int4(10)]);
        table.rows.push(vec![SqlValue::Int4(20)]);
        schema.tables.insert("nums".to_string(), table);

        let delete = DeleteStmt {
            table: "nums".to_string(),
            where_clause: None,
            returning: Some(vec![SelectColumn::Expr(
                Expr::Identifier("id".to_string()),
                None,
            )]),
        };
        let (n, rows) = execute_delete_returning(&delete, &mut db).unwrap();
        assert_eq!(n, 2);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0][0], SqlValue::Int4(10)));
        assert!(matches!(rows[1][0], SqlValue::Int4(20)));
    }
}
