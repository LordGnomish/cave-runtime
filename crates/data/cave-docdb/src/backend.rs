// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pluggable storage backends for the hybrid document database.
//!
//! [`StorageBackend`] is the document-level CRUD surface the command/route
//! layers talk to. Two implementations realise the *hybrid* strategy:
//!
//! * [`MemoryBackend`] — the pure-Rust in-memory engine ([`crate::engine`]),
//!   the default and the one the unit suite exercises end to end.
//! * [`SqlBackend`] — translates each operation into parameterised SQL via
//!   [`crate::sql`] and runs it through a [`SqlExecutor`]. The executor is the
//!   only IO boundary, so the translation can be tested against a
//!   [`RecordingExecutor`] without any database, and driven against a real
//!   PostgreSQL (cave-pg / cave-rdbms wire) via the optional `pg` feature.

use crate::bson::Document;
use crate::engine::Database;
use crate::sql;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Mutex;

/// Result of executing one SQL statement against a [`SqlExecutor`].
#[derive(Debug, Clone, PartialEq)]
pub enum ExecOutcome {
    /// Rows from a `SELECT`; each entry is a `_jsonb` document.
    Rows(Vec<Value>),
    /// Number of rows affected by an `INSERT`/`UPDATE`/`DELETE`.
    Affected(u64),
}

/// The SQL IO boundary. Implementors run a [`sql::SqlQuery`] and return its
/// outcome — nothing above this trait performs IO or knows about a database.
#[async_trait]
pub trait SqlExecutor: Send + Sync {
    /// Execute a parameterised SQL statement.
    async fn execute(&self, query: &sql::SqlQuery) -> Result<ExecOutcome, String>;
}

/// Document-level CRUD surface, backend-agnostic.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Insert one document into a collection.
    async fn insert(&self, coll: &str, doc: Value) -> Result<(), String>;
    /// Find all documents matching `filter` (`{}` == all).
    async fn find(&self, coll: &str, filter: &Value) -> Result<Vec<Value>, String>;
    /// Apply `update` to every document matching `filter`; returns match count.
    async fn update(&self, coll: &str, filter: &Value, update: &Value) -> Result<u64, String>;
    /// Delete every document matching `filter`; returns deleted count.
    async fn delete(&self, coll: &str, filter: &Value) -> Result<u64, String>;
    /// Count documents matching `filter`.
    async fn count(&self, coll: &str, filter: &Value) -> Result<u64, String>;
    /// Backend label (`"memory"` / `"postgres"`) for diagnostics.
    fn kind(&self) -> &'static str;
}

// ── value/document conversions ───────────────────────────────────────────────

/// Convert a JSON object value into the engine's `Document` map.
fn obj_to_doc(v: &Value) -> Document {
    v.as_object()
        .map(|m| m.iter().map(|(k, val)| (k.clone(), val.clone())).collect())
        .unwrap_or_default()
}

/// Convert an engine `Document` back into a JSON object value.
fn doc_to_value(d: &Document) -> Value {
    Value::Object(d.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

// ── in-memory backend ────────────────────────────────────────────────────────

/// In-memory backend over [`crate::engine::Database`].
pub struct MemoryBackend {
    db: Database,
}

impl MemoryBackend {
    /// Create an empty in-memory backend.
    pub fn new() -> Self {
        Self {
            db: Database::default_db(),
        }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StorageBackend for MemoryBackend {
    async fn insert(&self, coll: &str, doc: Value) -> Result<(), String> {
        let col = self.db.get_or_create_collection(coll).await;
        col.insert_one(obj_to_doc(&doc)).await?;
        Ok(())
    }

    async fn find(&self, coll: &str, filter: &Value) -> Result<Vec<Value>, String> {
        let Some(col) = self.db.get_collection(coll).await else {
            return Ok(Vec::new());
        };
        let f = obj_to_doc(filter);
        let docs = col.find(Some(&f)).await?;
        Ok(docs.iter().map(doc_to_value).collect())
    }

    async fn update(&self, coll: &str, filter: &Value, update: &Value) -> Result<u64, String> {
        let col = self.db.get_or_create_collection(coll).await;
        col.update_many(Some(&obj_to_doc(filter)), &obj_to_doc(update))
            .await
    }

    async fn delete(&self, coll: &str, filter: &Value) -> Result<u64, String> {
        let col = self.db.get_or_create_collection(coll).await;
        col.delete_many(Some(&obj_to_doc(filter))).await
    }

    async fn count(&self, coll: &str, filter: &Value) -> Result<u64, String> {
        let Some(col) = self.db.get_collection(coll).await else {
            return Ok(0);
        };
        col.count(Some(&obj_to_doc(filter))).await
    }

    fn kind(&self) -> &'static str {
        "memory"
    }
}

// ── SQL backend ──────────────────────────────────────────────────────────────

/// SQL backend: translates operations to SQL and runs them via `E`.
pub struct SqlBackend<E: SqlExecutor> {
    exec: E,
}

impl<E: SqlExecutor> SqlBackend<E> {
    /// Wrap a [`SqlExecutor`] as a document backend.
    pub fn new(exec: E) -> Self {
        Self { exec }
    }

    /// Borrow the underlying executor (e.g. to inspect recorded SQL in tests).
    pub fn executor(&self) -> &E {
        &self.exec
    }

    /// SQL used by [`StorageBackend::count`]: a single-row `count(*)`.
    fn count_query(coll: &str, filter: &Value) -> sql::SqlQuery {
        // Reuse the $count pipeline shape for a count(*) projection.
        sql::pipeline_to_sql(coll, &[serde_json::json!({"$match": filter}), serde_json::json!({"$count": "n"})])
            .expect("count pipeline always translates")
    }
}

#[async_trait]
impl<E: SqlExecutor> StorageBackend for SqlBackend<E> {
    async fn insert(&self, coll: &str, doc: Value) -> Result<(), String> {
        let q = sql::insert_to_sql(coll, &doc);
        self.exec.execute(&q).await?;
        Ok(())
    }

    async fn find(&self, coll: &str, filter: &Value) -> Result<Vec<Value>, String> {
        let q = sql::find_to_sql(coll, filter, None, None, None, None);
        match self.exec.execute(&q).await? {
            ExecOutcome::Rows(rows) => Ok(rows),
            ExecOutcome::Affected(_) => Ok(Vec::new()),
        }
    }

    async fn update(&self, coll: &str, filter: &Value, update: &Value) -> Result<u64, String> {
        let q = sql::update_to_sql(coll, filter, update);
        match self.exec.execute(&q).await? {
            ExecOutcome::Affected(n) => Ok(n),
            ExecOutcome::Rows(r) => Ok(r.len() as u64),
        }
    }

    async fn delete(&self, coll: &str, filter: &Value) -> Result<u64, String> {
        let q = sql::delete_to_sql(coll, filter);
        match self.exec.execute(&q).await? {
            ExecOutcome::Affected(n) => Ok(n),
            ExecOutcome::Rows(r) => Ok(r.len() as u64),
        }
    }

    async fn count(&self, coll: &str, filter: &Value) -> Result<u64, String> {
        let q = Self::count_query(coll, filter);
        match self.exec.execute(&q).await? {
            // Count result arrives as a single {"n": <int>} document.
            ExecOutcome::Rows(rows) => Ok(rows
                .first()
                .and_then(|r| r.get("n"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0)),
            ExecOutcome::Affected(n) => Ok(n),
        }
    }

    fn kind(&self) -> &'static str {
        "postgres"
    }
}

// ── recording executor (test/diagnostic) ─────────────────────────────────────

/// A [`SqlExecutor`] that records every statement and replays canned outcomes.
///
/// Used to assert the SQL a backend operation emits, and to drive a backend
/// round-trip without a database.
pub struct RecordingExecutor {
    calls: Mutex<Vec<sql::SqlQuery>>,
    canned: Mutex<std::collections::VecDeque<ExecOutcome>>,
}

impl RecordingExecutor {
    /// New recorder; absent a canned outcome, writes report `Affected(1)`.
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            canned: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// Queue a canned outcome to return for the next `execute` call.
    pub fn push_outcome(&self, outcome: ExecOutcome) {
        self.canned.lock().unwrap().push_back(outcome);
    }

    /// Number of statements executed so far.
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// The most recently executed statement.
    pub fn last(&self) -> Option<sql::SqlQuery> {
        self.calls.lock().unwrap().last().cloned()
    }
}

impl Default for RecordingExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SqlExecutor for RecordingExecutor {
    async fn execute(&self, query: &sql::SqlQuery) -> Result<ExecOutcome, String> {
        self.calls.lock().unwrap().push(query.clone());
        Ok(self
            .canned
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ExecOutcome::Affected(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn memory_backend_roundtrip() {
        let be = MemoryBackend::new();
        be.insert("users", json!({"_id": "1", "name": "alice", "age": 30}))
            .await
            .unwrap();
        be.insert("users", json!({"_id": "2", "name": "bob", "age": 20}))
            .await
            .unwrap();

        let found = be.find("users", &json!({"age": {"$gt": 25}})).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["name"], json!("alice"));

        assert_eq!(be.count("users", &json!({})).await.unwrap(), 2);
        assert_eq!(be.kind(), "memory");
    }

    #[tokio::test]
    async fn memory_backend_update_and_delete() {
        let be = MemoryBackend::new();
        be.insert("c", json!({"_id": "1", "n": 1})).await.unwrap();
        be.insert("c", json!({"_id": "2", "n": 1})).await.unwrap();

        let modified = be
            .update("c", &json!({"n": 1}), &json!({"$set": {"n": 2}}))
            .await
            .unwrap();
        assert_eq!(modified, 2);
        assert_eq!(be.count("c", &json!({"n": 2})).await.unwrap(), 2);

        let deleted = be.delete("c", &json!({"n": 2})).await.unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(be.count("c", &json!({})).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn sql_backend_emits_insert_sql() {
        let be = SqlBackend::new(RecordingExecutor::new());
        be.insert("c", json!({"_id": "1", "x": 5})).await.unwrap();
        let q = be.executor().last().unwrap();
        assert_eq!(q.sql, "INSERT INTO \"c\" (_jsonb) VALUES ($1::jsonb)");
        assert_eq!(q.params, vec!["{\"_id\":\"1\",\"x\":5}"]);
        assert_eq!(be.kind(), "postgres");
    }

    #[tokio::test]
    async fn sql_backend_find_returns_canned_rows() {
        let exec = RecordingExecutor::new();
        exec.push_outcome(ExecOutcome::Rows(vec![json!({"_id": "1", "x": 9})]));
        let be = SqlBackend::new(exec);

        let rows = be.find("c", &json!({"x": {"$gt": 1}})).await.unwrap();
        assert_eq!(rows, vec![json!({"_id": "1", "x": 9})]);

        let q = be.executor().last().unwrap();
        assert_eq!(q.sql, "SELECT _jsonb FROM \"c\" WHERE _jsonb -> 'x' > $1::jsonb");
        assert_eq!(q.params, vec!["1"]);
    }

    #[tokio::test]
    async fn sql_backend_count_reads_n_field() {
        let exec = RecordingExecutor::new();
        exec.push_outcome(ExecOutcome::Rows(vec![json!({"n": 7})]));
        let be = SqlBackend::new(exec);

        assert_eq!(be.count("c", &json!({})).await.unwrap(), 7);
        let q = be.executor().last().unwrap();
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('n', count(*)) AS _jsonb FROM \"c\""
        );
    }

    #[tokio::test]
    async fn sql_backend_delete_reports_affected() {
        let exec = RecordingExecutor::new();
        exec.push_outcome(ExecOutcome::Affected(3));
        let be = SqlBackend::new(exec);
        assert_eq!(be.delete("c", &json!({"x": 1})).await.unwrap(), 3);
        assert_eq!(
            be.executor().last().unwrap().sql,
            "DELETE FROM \"c\" WHERE _jsonb -> 'x' = $1::jsonb"
        );
    }

    /// A backend round-trip written against the trait object, proving both
    /// backends satisfy the same dyn-compatible contract.
    #[tokio::test]
    async fn dyn_storage_backend_is_object_safe() {
        let backends: Vec<Box<dyn StorageBackend>> = vec![Box::new(MemoryBackend::new())];
        for be in &backends {
            be.insert("c", json!({"_id": "1", "v": 1})).await.unwrap();
            assert_eq!(be.count("c", &json!({})).await.unwrap(), 1);
        }
    }
}
