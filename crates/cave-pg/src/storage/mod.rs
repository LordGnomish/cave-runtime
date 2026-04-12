//! Storage engine — heap tables, MVCC, WAL, indexes, sequences, and schemas.

pub mod heap;
pub mod mvcc;

use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use crate::error::{Error, PgError, Result, SqlState};
use crate::storage::heap::{ColumnDef, Constraint, FkAction, Index, IndexMethod, Schema, Sequence, Table, View};
use crate::storage::mvcc::{CommitLog, IsolationLevel, TransactionManager, TransactionState};
use crate::types::{oid, ColumnDesc, Oid, PgValue};

// ─────────────────────────────────────────────────────────────────────────────
// OID allocator
// ─────────────────────────────────────────────────────────────────────────────

/// Global OID allocator — starts above the PostgreSQL built-in range.
static NEXT_OID: AtomicU32 = AtomicU32::new(oid::USER_DEFINED_START);

pub fn alloc_oid() -> Oid {
    NEXT_OID.fetch_add(1, Ordering::SeqCst)
}

// ─────────────────────────────────────────────────────────────────────────────
// Database
// ─────────────────────────────────────────────────────────────────────────────

/// A single database — contains schemas and a shared transaction manager.
pub struct Database {
    pub name: String,
    pub oid: Oid,
    pub owner: String,
    pub encoding: String,
    pub collation: String,
    pub schemas: DashMap<String, Arc<RwLock<Schema>>>,
    pub txn_manager: Arc<TransactionManager>,
}

impl Database {
    pub fn new(name: impl Into<String>, oid: Oid, owner: impl Into<String>) -> Self {
        let mut db = Self {
            name: name.into(),
            oid,
            owner: owner.into(),
            encoding: "UTF8".to_string(),
            collation: "en_US.UTF-8".to_string(),
            schemas: DashMap::new(),
            txn_manager: Arc::new(TransactionManager::new()),
        };

        // Create the default schemas
        db.create_schema("public", "postgres");
        db.create_schema("pg_catalog", "postgres");
        db.create_schema("information_schema", "postgres");
        db.create_schema("pg_toast", "postgres");
        db
    }

    pub fn create_schema(&self, name: &str, owner: &str) {
        let oid = alloc_oid();
        let schema = Schema::new(name, oid, owner);
        self.schemas.insert(name.to_lowercase(), Arc::new(RwLock::new(schema)));
    }

    pub fn schema(&self, name: &str) -> Option<Arc<RwLock<Schema>>> {
        self.schemas.get(&name.to_lowercase()).map(|s| s.clone())
    }

    pub fn public_schema(&self) -> Arc<RwLock<Schema>> {
        self.schema("public").expect("public schema must exist")
    }

    /// Resolve a table, searching the given search_path.
    pub fn resolve_table(
        &self,
        table_name: &str,
        search_path: &[&str],
    ) -> Option<(Arc<RwLock<Schema>>, String)> {
        // Check if table_name is qualified (schema.table)
        if let Some((schema_name, tbl)) = table_name.split_once('.') {
            if let Some(schema) = self.schema(schema_name) {
                if schema.read().table(tbl).is_some() {
                    return Some((schema, tbl.to_string()));
                }
            }
            return None;
        }

        // Search the search path
        for schema_name in search_path {
            if let Some(schema) = self.schema(schema_name) {
                if schema.read().table(table_name).is_some() {
                    return Some((schema, table_name.to_string()));
                }
            }
        }
        None
    }

    /// Resolve a view, searching the given search_path.
    pub fn resolve_view(
        &self,
        view_name: &str,
        search_path: &[&str],
    ) -> Option<(Arc<RwLock<Schema>>, String)> {
        if let Some((schema_name, v)) = view_name.split_once('.') {
            if let Some(schema) = self.schema(schema_name) {
                if schema.read().view(v).is_some() {
                    return Some((schema, v.to_string()));
                }
            }
            return None;
        }
        for schema_name in search_path {
            if let Some(schema) = self.schema(schema_name) {
                if schema.read().view(view_name).is_some() {
                    return Some((schema, view_name.to_string()));
                }
            }
        }
        None
    }

    /// Resolve a sequence, searching the given search_path.
    pub fn resolve_sequence(
        &self,
        seq_name: &str,
        search_path: &[&str],
    ) -> Option<(Arc<RwLock<Schema>>, String)> {
        if let Some((schema_name, s)) = seq_name.split_once('.') {
            if let Some(schema) = self.schema(schema_name) {
                if schema.read().sequence(s).is_some() {
                    return Some((schema, s.to_string()));
                }
            }
            return None;
        }
        for schema_name in search_path {
            if let Some(schema) = self.schema(schema_name) {
                if schema.read().sequence(seq_name).is_some() {
                    return Some((schema, seq_name.to_string()));
                }
            }
        }
        None
    }

    pub fn schema_names(&self) -> Vec<String> {
        self.schemas.iter().map(|e| e.key().clone()).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Engine — top-level store for all databases
// ─────────────────────────────────────────────────────────────────────────────

/// The top-level storage engine — manages multiple databases.
pub struct Engine {
    pub databases: DashMap<String, Arc<Database>>,
}

impl Engine {
    pub fn new() -> Self {
        let engine = Self {
            databases: DashMap::new(),
        };
        // Create default databases
        let default_db = Arc::new(Database::new("postgres", alloc_oid(), "postgres"));
        engine.databases.insert("postgres".to_string(), default_db.clone());
        // "template1" — standard template
        let template1 = Arc::new(Database::new("template1", alloc_oid(), "postgres"));
        engine.databases.insert("template1".to_string(), template1);
        engine
    }

    pub fn database(&self, name: &str) -> Option<Arc<Database>> {
        self.databases.get(&name.to_lowercase()).map(|d| d.clone())
    }

    pub fn create_database(&self, name: &str, owner: &str) -> Result<()> {
        let name_lower = name.to_lowercase();
        if self.databases.contains_key(&name_lower) {
            return Err(Error::Pg(PgError::error(
                SqlState::DUPLICATE_DATABASE,
                format!("database \"{name}\" already exists"),
            )));
        }
        let oid = alloc_oid();
        let db = Arc::new(Database::new(name, oid, owner));
        self.databases.insert(name_lower, db);
        Ok(())
    }

    pub fn drop_database(&self, name: &str) -> Result<()> {
        if name == "postgres" || name == "template1" {
            return Err(Error::Pg(PgError::error(
                SqlState::FEATURE_NOT_SUPPORTED,
                format!("cannot drop database \"{name}\""),
            )));
        }
        if self.databases.remove(&name.to_lowercase()).is_none() {
            return Err(Error::Pg(PgError::error(
                SqlState::UNDEFINED_DATABASE,
                format!("database \"{name}\" does not exist"),
            )));
        }
        Ok(())
    }

    pub fn database_names(&self) -> Vec<String> {
        self.databases.iter().map(|e| e.key().clone()).collect()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
