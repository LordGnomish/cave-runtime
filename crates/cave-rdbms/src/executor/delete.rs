// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DELETE statement execution.

use crate::sql::ast::DeleteStmt;
use crate::storage::schema::Database;

pub fn execute_delete(delete: &DeleteStmt, db: &mut Database) -> Result<u64, String> {
    let schema = db.schemas.get_mut("public").ok_or("no public schema")?;
    let table = schema
        .tables
        .get_mut(&delete.table)
        .ok_or(format!("table {} not found", delete.table))?;

    let initial_count = table.rows.len();
    if delete.where_clause.is_none() {
        table.rows.clear();
    }
    let deleted = (initial_count - table.rows.len()) as u64;
    Ok(deleted)
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
        };
        let result = execute_delete(&delete, &mut db);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }
}
