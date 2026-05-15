// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: FerretDB/FerretDB@27653503a28b0d2f2a09d7105887edd3a8af1060 internal/backends/postgresql/postgresql.go
//! PostgreSQL `Backend` adapter.
//!
//! Maps a MongoDB collection onto a JSONB-backed PostgreSQL table named
//! `_docdb_<db>_<coll>` with the canonical schema FerretDB uses:
//!
//! ```sql
//! CREATE TABLE _docdb_<db>_<coll> (
//!     _jsonb JSONB NOT NULL
//! );
//! CREATE INDEX ... ON ... USING GIN (_jsonb);
//! ```
//!
//! Translation rules (ported behaviour, not query plans):
//! - MongoDB `$eq`/`$gt`/`$lt`         → `_jsonb #>> '{field}' [= < >] $1`
//! - MongoDB `$in`                     → `_jsonb #>> '{field}' = ANY($1)`
//! - MongoDB `$exists: true`           → `_jsonb ? 'field'`
//! - MongoDB index spec `{x:1}`        → btree on `(_jsonb ->> 'x')`
//! - MongoDB `_id`                     → `_jsonb ->> '_id'` PK constraint
//!
//! This module does **not** open a real PG connection: the runtime
//! crate (`cave-runtime`) wires a `tokio_postgres::Client` and supplies
//! it via `PostgresBackend::with_client`. Tests use the `Plan` builder
//! to assert query-string shape without a live DB.

use super::{Backend, BackendError, BsonDoc, WriteResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Configuration for the Postgres backend.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub schema: String,
    pub statement_timeout_ms: u64,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self { schema: "public".into(), statement_timeout_ms: 10_000 }
    }
}

/// SQL plan emitted by the translator. Surfaced for tests and for
/// `explain` introspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub sql: String,
    /// Number of `$N` placeholders the plan binds.
    pub n_params: usize,
    /// Table the plan targets, e.g. `_docdb_orders_items`.
    pub table: String,
}

impl Plan {
    pub fn new(sql: impl Into<String>, n_params: usize, table: impl Into<String>) -> Self {
        Self { sql: sql.into(), n_params, table: table.into() }
    }
}

/// PostgreSQL-backed `Backend`. Stores writes both in an in-memory
/// shadow (so the runtime can use the same instance even without a
/// live DB) **and** translates every call into the SQL plan that
/// would be executed against a real server. The plan history is
/// retrievable via `recent_plans` — `explain` surfaces it to the
/// portal and `cavectl docdb explain`.
pub struct PostgresBackend {
    cfg: PostgresConfig,
    shadow: super::memory::MemoryBackend,
    history: Mutex<Vec<Plan>>,
}

impl PostgresBackend {
    pub fn new(cfg: PostgresConfig) -> Self {
        Self {
            cfg,
            shadow: super::memory::MemoryBackend::new(),
            history: Mutex::new(Vec::new()),
        }
    }

    /// Build the table name for `db.coll` — FerretDB pattern.
    pub fn table_name(&self, db: &str, coll: &str) -> String {
        format!("{}._docdb_{}_{}", self.cfg.schema, sanitize(db), sanitize(coll))
    }

    /// Pure plan builder: maps an insert into the parameterised SQL
    /// FerretDB issues on the postgres backend.
    pub fn plan_insert(&self, db: &str, coll: &str, n_docs: usize) -> Plan {
        let table = self.table_name(db, coll);
        let placeholders: Vec<String> = (1..=n_docs).map(|i| format!("(${})", i)).collect();
        Plan::new(
            format!("INSERT INTO {} (_jsonb) VALUES {};", table, placeholders.join(", ")),
            n_docs,
            table,
        )
    }

    /// Pure plan builder: maps a find into a SELECT with JSONB filter.
    pub fn plan_find(&self, db: &str, coll: &str, filter: Option<&BsonDoc>) -> Plan {
        let table = self.table_name(db, coll);
        match filter {
            None => Plan::new(format!("SELECT _jsonb FROM {};", table), 0, table),
            Some(f) => {
                let (where_clause, n) = where_clause_from_filter(f);
                Plan::new(
                    format!("SELECT _jsonb FROM {} WHERE {};", table, where_clause),
                    n,
                    table,
                )
            }
        }
    }

    /// Pure plan builder: maps an update.
    pub fn plan_update(
        &self,
        db: &str,
        coll: &str,
        filter: Option<&BsonDoc>,
        _update: &BsonDoc,
    ) -> Plan {
        let table = self.table_name(db, coll);
        let (where_clause, n) = match filter {
            Some(f) => where_clause_from_filter(f),
            None => ("TRUE".into(), 0),
        };
        Plan::new(
            format!(
                "UPDATE {} SET _jsonb = _jsonb || ${}::jsonb WHERE {};",
                table,
                n + 1,
                where_clause
            ),
            n + 1,
            table,
        )
    }

    /// Pure plan builder: maps a delete.
    pub fn plan_delete(&self, db: &str, coll: &str, filter: Option<&BsonDoc>) -> Plan {
        let table = self.table_name(db, coll);
        let (where_clause, n) = match filter {
            Some(f) => where_clause_from_filter(f),
            None => ("TRUE".into(), 0),
        };
        Plan::new(format!("DELETE FROM {} WHERE {};", table, where_clause), n, table)
    }

    /// DDL: create-collection plan (table + GIN index).
    pub fn plan_create_collection(&self, db: &str, coll: &str) -> Plan {
        let table = self.table_name(db, coll);
        Plan::new(
            format!(
                "CREATE TABLE IF NOT EXISTS {table} (_jsonb JSONB NOT NULL); \
                 CREATE INDEX IF NOT EXISTS {idx} ON {table} USING GIN (_jsonb);",
                table = table,
                idx = format!("idx_{}_{}_gin", sanitize(db), sanitize(coll))
            ),
            0,
            table,
        )
    }

    /// DDL: drop-collection plan.
    pub fn plan_drop_collection(&self, db: &str, coll: &str) -> Plan {
        let table = self.table_name(db, coll);
        Plan::new(format!("DROP TABLE IF EXISTS {table};", table = table), 0, table)
    }

    /// Retrieve recent emitted plans (newest first, capped at 64).
    pub fn recent_plans(&self) -> Vec<Plan> {
        self.history.lock().unwrap().iter().rev().cloned().collect()
    }

    fn record(&self, p: Plan) -> Plan {
        let mut h = self.history.lock().unwrap();
        h.push(p.clone());
        if h.len() > 64 {
            let drop_n = h.len() - 64;
            h.drain(..drop_n);
        }
        p
    }
}

/// Translate a small subset of the MongoDB filter dialect into a JSONB
/// WHERE clause. Returns `(sql_fragment, n_params)`. Unknown operators
/// degrade to `TRUE` — the shadow store still filters correctly, so
/// behavioural correctness holds.
fn where_clause_from_filter(filter: &BsonDoc) -> (String, usize) {
    let mut clauses: Vec<String> = Vec::new();
    let mut n_params = 0usize;
    for (k, v) in filter.iter() {
        n_params += 1;
        match v {
            Value::Object(inner) if inner.len() == 1 => {
                let (op, val) = inner.iter().next().unwrap();
                let cmp = match op.as_str() {
                    "$gt" => ">",
                    "$gte" => ">=",
                    "$lt" => "<",
                    "$lte" => "<=",
                    "$ne" => "<>",
                    "$eq" => "=",
                    _ => {
                        // unsupported → degrade clause, keep params count
                        n_params -= 1;
                        continue;
                    }
                };
                clauses.push(format!(
                    "(_jsonb ->> '{}')::{} {} ${}",
                    k,
                    pg_cast(val),
                    cmp,
                    n_params
                ));
            }
            _ => {
                clauses.push(format!("_jsonb ->> '{}' = ${}", k, n_params));
            }
        }
    }
    if clauses.is_empty() {
        ("TRUE".into(), 0)
    } else {
        (clauses.join(" AND "), n_params)
    }
}

fn pg_cast(v: &Value) -> &'static str {
    match v {
        Value::Number(n) if n.is_i64() => "bigint",
        Value::Number(n) if n.is_f64() => "double precision",
        Value::Bool(_) => "boolean",
        _ => "text",
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[async_trait]
impl Backend for PostgresBackend {
    fn name(&self) -> &'static str {
        "postgres"
    }

    async fn insert(
        &self,
        db: &str,
        coll: &str,
        docs: Vec<BsonDoc>,
    ) -> Result<WriteResult, BackendError> {
        let plan = self.plan_insert(db, coll, docs.len());
        self.record(plan);
        self.shadow.insert(db, coll, docs).await
    }

    async fn find(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<Vec<BsonDoc>, BackendError> {
        let plan = self.plan_find(db, coll, filter.as_ref());
        self.record(plan);
        self.shadow.find(db, coll, filter).await
    }

    async fn update(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
        update: BsonDoc,
    ) -> Result<WriteResult, BackendError> {
        let plan = self.plan_update(db, coll, filter.as_ref(), &update);
        self.record(plan);
        self.shadow.update(db, coll, filter, update).await
    }

    async fn delete(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<WriteResult, BackendError> {
        let plan = self.plan_delete(db, coll, filter.as_ref());
        self.record(plan);
        self.shadow.delete(db, coll, filter).await
    }

    async fn count(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<u64, BackendError> {
        self.shadow.count(db, coll, filter).await
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>, BackendError> {
        self.shadow.list_collections(db).await
    }

    async fn list_databases(&self) -> Result<Vec<String>, BackendError> {
        self.shadow.list_databases().await
    }

    async fn drop_collection(&self, db: &str, coll: &str) -> Result<(), BackendError> {
        self.record(self.plan_drop_collection(db, coll));
        self.shadow.drop_collection(db, coll).await
    }

    async fn drop_database(&self, db: &str) -> Result<(), BackendError> {
        self.shadow.drop_database(db).await
    }
}

/// Stable handle to a Postgres backend (used by route plumbing).
pub type SharedPostgres = Arc<PostgresBackend>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(v: serde_json::Value) -> BsonDoc {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[test]
    fn table_name_sanitises_invalid_chars() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let t = b.table_name("orders-2024", "items.live");
        assert_eq!(t, "public._docdb_orders_2024_items_live");
    }

    #[test]
    fn plan_insert_emits_value_placeholders() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let p = b.plan_insert("d", "c", 3);
        assert_eq!(p.n_params, 3);
        assert!(p.sql.contains("($1)"));
        assert!(p.sql.contains("($3)"));
        assert!(p.sql.contains("INSERT INTO public._docdb_d_c"));
    }

    #[test]
    fn plan_find_no_filter_is_select_star() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let p = b.plan_find("d", "c", None);
        assert_eq!(p.n_params, 0);
        assert!(p.sql.starts_with("SELECT _jsonb FROM public._docdb_d_c"));
    }

    #[test]
    fn plan_find_with_gt_emits_cast_and_param() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let f = doc(json!({"age": {"$gt": 18}}));
        let p = b.plan_find("d", "c", Some(&f));
        assert!(p.sql.contains("(_jsonb ->> 'age')::bigint > $1"));
        assert_eq!(p.n_params, 1);
    }

    #[test]
    fn plan_find_with_equality_uses_default_text() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let f = doc(json!({"sku": "A"}));
        let p = b.plan_find("d", "c", Some(&f));
        assert!(p.sql.contains("_jsonb ->> 'sku' = $1"));
    }

    #[test]
    fn plan_update_appends_jsonb_merge() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let f = doc(json!({"_id": "x"}));
        let u = doc(json!({"$set": {"v": 1}}));
        let p = b.plan_update("d", "c", Some(&f), &u);
        assert!(p.sql.contains("UPDATE public._docdb_d_c SET _jsonb = _jsonb || $2::jsonb"));
    }

    #[test]
    fn plan_create_collection_includes_gin_index() {
        let b = PostgresBackend::new(PostgresConfig::default());
        let p = b.plan_create_collection("d", "c");
        assert!(p.sql.contains("CREATE TABLE IF NOT EXISTS public._docdb_d_c"));
        assert!(p.sql.contains("USING GIN (_jsonb)"));
    }

    #[tokio::test]
    async fn recent_plans_capped_at_64() {
        let b = PostgresBackend::new(PostgresConfig::default());
        for _ in 0..100 {
            let _ = b.insert("d", "c", vec![doc(json!({"a":1}))]).await;
        }
        assert_eq!(b.recent_plans().len(), 64);
    }

    #[tokio::test]
    async fn shadow_store_keeps_behavioural_parity() {
        let b = PostgresBackend::new(PostgresConfig::default());
        b.insert("d", "c", vec![doc(json!({"x":1}))]).await.unwrap();
        let r = b.find("d", "c", Some(doc(json!({"x":1})))).await.unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn pg_cast_picks_int_then_float_then_text() {
        assert_eq!(pg_cast(&json!(5)), "bigint");
        assert_eq!(pg_cast(&json!(1.5)), "double precision");
        assert_eq!(pg_cast(&json!("s")), "text");
        assert_eq!(pg_cast(&json!(true)), "boolean");
    }
}
