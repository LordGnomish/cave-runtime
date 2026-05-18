// SPDX-License-Identifier: AGPL-3.0-or-later
//! PostgreSQL system catalog.

use crate::storage::schema::Database;

pub struct SystemCatalog;

impl SystemCatalog {
    /// Returns rows for pg_tables query.
    pub fn pg_tables(db: &Database) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        for (schema_name, schema) in &db.schemas {
            for (table_name, _table) in &schema.tables {
                rows.push(vec![
                    schema_name.clone(),
                    table_name.clone(),
                    "postgres".to_string(), // table_owner
                    "".to_string(),         // storage
                    "f".to_string(),        // has_index_on_toast_table
                ]);
            }
        }
        rows
    }

    /// Returns rows for information_schema.tables.
    pub fn information_schema_tables(db: &Database) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        for (schema_name, schema) in &db.schemas {
            for (table_name, _table) in &schema.tables {
                rows.push(vec![
                    "def".to_string(),         // table_catalog
                    schema_name.clone(),       // table_schema
                    table_name.clone(),        // table_name
                    "BASE TABLE".to_string(),  // table_type
                ]);
            }
        }
        rows
    }

    /// Returns rows for information_schema.columns.
    pub fn information_schema_columns(db: &Database) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        for (schema_name, schema) in &db.schemas {
            for (table_name, table) in &schema.tables {
                for (ordinal, col) in table.columns.iter().enumerate() {
                    rows.push(vec![
                        "def".to_string(),           // table_catalog
                        schema_name.clone(),         // table_schema
                        table_name.clone(),          // table_name
                        col.name.clone(),            // column_name
                        (ordinal + 1).to_string(),   // ordinal_position
                        "".to_string(),              // column_default
                        if col.not_null {
                            "NO"
                        } else {
                            "YES"
                        }
                        .to_string(), // is_nullable
                        col.type_name.clone(), // data_type
                    ]);
                }
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::{ColumnDef, Schema, Table};

    #[test]
    fn test_pg_tables_catalog() {
        let mut db = Database::new("testdb");
        let mut schema = Schema::new("public");
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        schema.tables.insert("users".to_string(), Table::new("users", cols));
        db.schemas.insert("public".to_string(), schema);

        let rows = SystemCatalog::pg_tables(&db);
        assert!(rows.len() > 0);
        let found = rows.iter().any(|row| row[1] == "users");
        assert!(found);
    }

    #[test]
    fn test_information_schema_tables() {
        let mut db = Database::new("testdb");
        let mut schema = Schema::new("public");
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        schema.tables.insert("users".to_string(), Table::new("users", cols));
        db.schemas.insert("public".to_string(), schema);

        let rows = SystemCatalog::information_schema_tables(&db);
        assert!(rows.len() > 0);
    }

    #[test]
    fn test_information_schema_columns() {
        let mut db = Database::new("testdb");
        let mut schema = Schema::new("public");
        let cols = vec![ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            primary_key: true,
        }];
        schema.tables.insert("users".to_string(), Table::new("users", cols));
        db.schemas.insert("public".to_string(), schema);

        let rows = SystemCatalog::information_schema_columns(&db);
        assert!(rows.len() > 0);
    }
}
