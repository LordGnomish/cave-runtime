// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Schema and table definitions.

use crate::types::SqlValue;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Database {
    pub name: String,
    pub schemas: HashMap<String, Schema>,
}

impl Database {
    pub fn new(name: &str) -> Self {
        let mut schemas = HashMap::new();
        schemas.insert("public".to_string(), Schema::new("public"));
        schemas.insert("pg_catalog".to_string(), Schema::new("pg_catalog"));
        schemas.insert(
            "information_schema".to_string(),
            Schema::new("information_schema"),
        );
        Database {
            name: name.to_string(),
            schemas,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub tables: HashMap<String, Table>,
}

impl Schema {
    pub fn new(name: &str) -> Self {
        Schema {
            name: name.to_string(),
            tables: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub rows: Vec<Row>,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub type_name: String,
    pub not_null: bool,
    pub primary_key: bool,
}

pub type Row = Vec<SqlValue>;

impl Table {
    pub fn new(name: &str, columns: Vec<ColumnDef>) -> Self {
        Table {
            name: name.to_string(),
            columns,
            rows: Vec::new(),
            constraints: Vec::new(),
        }
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_creation() {
        let db = Database::new("postgres");
        assert_eq!(db.name, "postgres");
        assert!(db.schemas.contains_key("public"));
        assert!(db.schemas.contains_key("pg_catalog"));
    }

    #[test]
    fn test_schema_creation() {
        let schema = Schema::new("myschema");
        assert_eq!(schema.name, "myschema");
        assert!(schema.tables.is_empty());
    }

    #[test]
    fn test_table_creation() {
        let cols = vec![
            ColumnDef {
                name: "id".to_string(),
                type_name: "int".to_string(),
                not_null: true,
                primary_key: true,
            },
            ColumnDef {
                name: "name".to_string(),
                type_name: "text".to_string(),
                not_null: false,
                primary_key: false,
            },
        ];
        let table = Table::new("users", cols);
        assert_eq!(table.name, "users");
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.column_index("id"), Some(0));
        assert_eq!(table.column_index("name"), Some(1));
    }
}
