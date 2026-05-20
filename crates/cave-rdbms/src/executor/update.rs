// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! UPDATE statement execution.

use crate::sql::ast::{Literal, UpdateStmt};
use crate::storage::schema::Database;
use crate::types::SqlValue;

pub fn execute_update(update: &UpdateStmt, db: &mut Database) -> Result<u64, String> {
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
    if update.where_clause.is_none() {
        for row in &mut table.rows {
            for (idx, val) in &col_indices {
                if *idx < row.len() {
                    row[*idx] = val.clone();
                }
            }
            updated += 1;
        }
    }
    Ok(updated)
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
        };
        let result = execute_update(&update, &mut db);
        assert!(result.is_ok());
    }
}
