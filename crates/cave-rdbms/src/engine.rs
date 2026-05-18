// SPDX-License-Identifier: AGPL-3.0-or-later
//! RDBMS execution engine.

use crate::storage::schema::{ColumnDef, Database, Table};
use crate::types::SqlValue;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Engine {
    pub storage: Arc<RwLock<Database>>,
}

impl Engine {
    pub fn new() -> Self {
        let mut db = Database::new("postgres");

        // Create demo 'users' table
        let user_cols = vec![
            ColumnDef {
                name: "id".to_string(),
                type_name: "int".to_string(),
                not_null: true,
                primary_key: true,
            },
            ColumnDef {
                name: "username".to_string(),
                type_name: "text".to_string(),
                not_null: false,
                primary_key: false,
            },
            ColumnDef {
                name: "email".to_string(),
                type_name: "text".to_string(),
                not_null: false,
                primary_key: false,
            },
        ];
        let mut users_table = Table::new("users", user_cols);
        users_table.rows.push(vec![
            SqlValue::Int4(1),
            SqlValue::Text("alice".to_string()),
            SqlValue::Text("alice@example.com".to_string()),
        ]);
        users_table.rows.push(vec![
            SqlValue::Int4(2),
            SqlValue::Text("bob".to_string()),
            SqlValue::Text("bob@example.com".to_string()),
        ]);

        let public_schema = db.schemas.get_mut("public").unwrap();
        public_schema
            .tables
            .insert("users".to_string(), users_table);

        // Create demo 'orders' table
        let order_cols = vec![
            ColumnDef {
                name: "id".to_string(),
                type_name: "int".to_string(),
                not_null: true,
                primary_key: true,
            },
            ColumnDef {
                name: "user_id".to_string(),
                type_name: "int".to_string(),
                not_null: false,
                primary_key: false,
            },
            ColumnDef {
                name: "amount".to_string(),
                type_name: "numeric".to_string(),
                not_null: false,
                primary_key: false,
            },
        ];
        let mut orders_table = Table::new("orders", order_cols);
        orders_table.rows.push(vec![
            SqlValue::Int4(100),
            SqlValue::Int4(1),
            SqlValue::Numeric(99.99),
        ]);
        orders_table.rows.push(vec![
            SqlValue::Int4(101),
            SqlValue::Int4(2),
            SqlValue::Numeric(49.50),
        ]);

        let public_schema = db.schemas.get_mut("public").unwrap();
        public_schema
            .tables
            .insert("orders".to_string(), orders_table);

        Engine {
            storage: Arc::new(RwLock::new(db)),
        }
    }

    pub async fn get_database(&self) -> Database {
        self.storage.read().await.clone()
    }

    pub async fn execute_ddl(&self, statement: &str) -> Result<String, String> {
        let _db = self.storage.write().await;
        if statement.to_uppercase().contains("CREATE TABLE") {
            Ok("TABLE CREATED".to_string())
        } else if statement.to_uppercase().contains("DROP TABLE") {
            Ok("TABLE DROPPED".to_string())
        } else {
            Ok("OK".to_string())
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_creation() {
        let engine = Engine::new();
        let db = engine.get_database().await;
        assert_eq!(db.name, "postgres");
    }

    #[tokio::test]
    async fn test_engine_has_demo_tables() {
        let engine = Engine::new();
        let db = engine.get_database().await;
        let public = db.schemas.get("public").unwrap();
        assert!(public.tables.contains_key("users"));
        assert!(public.tables.contains_key("orders"));
    }
}
