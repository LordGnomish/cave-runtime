//! INSERT statement execution.

use crate::sql::ast::{InsertStmt, Literal};
use crate::storage::schema::Database;
use crate::types::SqlValue;

pub fn execute_insert(insert: &InsertStmt, db: &mut Database) -> Result<u64, String> {
    let schema = db
        .schemas
        .get_mut("public")
        .ok_or("no public schema")?;
    let table = schema
        .tables
        .get_mut(&insert.table)
        .ok_or(format!("table {} not found", insert.table))?;

    let mut inserted = 0u64;
    for row_values in &insert.values {
        let mut row = Vec::new();
        for val_expr in row_values {
            use crate::sql::ast::Expr;
            let val = match val_expr {
                Expr::Literal(lit) => match lit {
                    Literal::Null => SqlValue::Null,
                    Literal::Integer(n) => SqlValue::Int4(*n as i32),
                    Literal::Float(f) => SqlValue::Numeric(*f),
                    Literal::String(s) => SqlValue::Text(s.clone()),
                    Literal::Boolean(b) => SqlValue::Bool(*b),
                    Literal::Date(s) => SqlValue::Date(s.clone()),
                    Literal::Timestamp(s) => SqlValue::Timestamp(s.clone()),
                },
                _ => return Err("complex expressions in INSERT not yet supported".to_string()),
            };
            row.push(val);
        }
        table.rows.push(row);
        inserted += 1;
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::ast::Expr;
    use crate::storage::schema::ColumnDef;

    #[test]
    fn test_insert_simple() {
        let mut db = Database::new("test");
        let schema = db.schemas.get_mut("public").unwrap();
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        let table = crate::storage::schema::Table::new("users", cols);
        schema.tables.insert("users".to_string(), table);

        let insert = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![Expr::Literal(Literal::Integer(1))]],
        };
        let result = execute_insert(&insert, &mut db);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }
}
