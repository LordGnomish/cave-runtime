// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Persistence layer for CAVE modules.
//!
//! Provides a `Storage` trait with three implementations:
//! - `MemoryStorage`  — in-process HashMap, for tests and local dev
//! - `DiskStorage`    — SQLite via rusqlite (bundled), for single-node deployments
//! - `PostgresStorage`— deadpool-postgres, schema-per-module, for production
//!
//! The object-safe `Storage` trait exposes raw `String`/JSON operations.
//! `StorageExt`, implemented for `Arc<dyn Storage>`, provides ergonomic typed
//! `get<T>` / `list<T>` / `put<T>` / `query<T>` helpers.

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::CavePool;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Spawn-blocking panic")]
    Join(#[from] tokio::task::JoinError),
}

pub type StorageResult<T> = Result<T, StorageError>;

// ── Filter ────────────────────────────────────────────────────────────────────

/// A simple equality/membership filter for `query`.
#[derive(Debug, Clone)]
pub struct Filter {
    /// Top-level JSON field name.
    pub field: String,
    pub op: FilterOp,
    /// The value to compare against (as a `serde_json::Value`).
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum FilterOp {
    Eq,
    Ne,
    /// String contains (case-sensitive).
    Contains,
}

fn filter_matches(json: &str, filter: &Filter) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    let field_val = &v[&filter.field];
    match &filter.op {
        FilterOp::Eq => field_val == &filter.value,
        FilterOp::Ne => field_val != &filter.value,
        FilterOp::Contains => {
            if let (Some(fv), Some(sv)) = (field_val.as_str(), filter.value.as_str()) {
                fv.contains(sv)
            } else {
                false
            }
        }
    }
}

// ── Storage trait (object-safe) ───────────────────────────────────────────────

/// Core storage trait. All methods operate on raw JSON strings so that the
/// trait is object-safe and can be used as `Arc<dyn Storage>`.
///
/// For typed access use the [`StorageExt`] extension trait which is blanket-
/// implemented for `Arc<dyn Storage>`.
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    /// Fetch a single item. Returns `None` when absent.
    async fn get_raw(&self, collection: &str, id: &str) -> StorageResult<Option<String>>;

    /// List all items in a collection as raw JSON strings.
    async fn list_raw(&self, collection: &str) -> StorageResult<Vec<String>>;

    /// Upsert an item.
    async fn put_raw(&self, collection: &str, id: &str, data: &str) -> StorageResult<()>;

    /// Delete an item. Returns `true` when an item was actually removed.
    async fn delete(&self, collection: &str, id: &str) -> StorageResult<bool>;

    /// Return items in `collection` that match `filter`.
    async fn query_raw(&self, collection: &str, filter: &Filter) -> StorageResult<Vec<String>>;
}

// ── StorageExt — typed wrapper over Arc<dyn Storage> ─────────────────────────

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = StorageResult<T>> + Send + 'a>>;

/// Extension trait that adds generic `get<T>`, `list<T>`, `put<T>`, `query<T>`
/// on top of the object-safe `Storage` interface.
///
/// Implemented for `Arc<dyn Storage>`. Bring this into scope with
/// `use cave_db::persistence::StorageExt` to unlock the typed helpers.
pub trait StorageExt {
    fn get<T>(&self, collection: &str, id: &str) -> BoxFut<'_, Option<T>>
    where
        T: DeserializeOwned + Send + 'static;

    fn list<T>(&self, collection: &str) -> BoxFut<'_, Vec<T>>
    where
        T: DeserializeOwned + Send + 'static;

    fn put<T>(&self, collection: &str, id: &str, value: &T) -> BoxFut<'_, ()>
    where
        T: Serialize + Send + Sync + 'static;

    fn query<T>(&self, collection: &str, filter: &Filter) -> BoxFut<'_, Vec<T>>
    where
        T: DeserializeOwned + Send + 'static;
}

impl StorageExt for Arc<dyn Storage> {
    fn get<T>(&self, collection: &str, id: &str) -> BoxFut<'_, Option<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let s = Arc::clone(self);
        let col = collection.to_owned();
        let id = id.to_owned();
        Box::pin(async move {
            match s.get_raw(&col, &id).await? {
                Some(raw) => Ok(Some(serde_json::from_str::<T>(&raw)?)),
                None => Ok(None),
            }
        })
    }

    fn list<T>(&self, collection: &str) -> BoxFut<'_, Vec<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let s = Arc::clone(self);
        let col = collection.to_owned();
        Box::pin(async move {
            let raws = s.list_raw(&col).await?;
            raws.iter()
                .map(|raw| serde_json::from_str::<T>(raw).map_err(StorageError::from))
                .collect()
        })
    }

    fn put<T>(&self, collection: &str, id: &str, value: &T) -> BoxFut<'_, ()>
    where
        T: Serialize + Send + Sync + 'static,
    {
        let s = Arc::clone(self);
        let col = collection.to_owned();
        let id = id.to_owned();
        let json = match serde_json::to_string(value) {
            Ok(j) => j,
            Err(e) => return Box::pin(std::future::ready(Err(StorageError::from(e)))),
        };
        Box::pin(async move { s.put_raw(&col, &id, &json).await })
    }

    fn query<T>(&self, collection: &str, filter: &Filter) -> BoxFut<'_, Vec<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let s = Arc::clone(self);
        let col = collection.to_owned();
        let filter = filter.clone();
        Box::pin(async move {
            let raws = s.query_raw(&col, &filter).await?;
            raws.iter()
                .map(|raw| serde_json::from_str::<T>(raw).map_err(StorageError::from))
                .collect()
        })
    }
}

// ── MemoryStorage ─────────────────────────────────────────────────────────────

/// In-memory storage — suitable for tests and ephemeral local dev sessions.
///
/// State is held in a `tokio::sync::Mutex<HashMap<collection, HashMap<id, json>>>`.
pub struct MemoryStorage {
    data: Mutex<HashMap<String, HashMap<String, String>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn get_raw(&self, collection: &str, id: &str) -> StorageResult<Option<String>> {
        let guard = self.data.lock().await;
        Ok(guard.get(collection).and_then(|c| c.get(id)).cloned())
    }

    async fn list_raw(&self, collection: &str) -> StorageResult<Vec<String>> {
        let guard = self.data.lock().await;
        Ok(guard
            .get(collection)
            .map(|c| c.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn put_raw(&self, collection: &str, id: &str, data: &str) -> StorageResult<()> {
        let mut guard = self.data.lock().await;
        guard
            .entry(collection.to_owned())
            .or_default()
            .insert(id.to_owned(), data.to_owned());
        Ok(())
    }

    async fn delete(&self, collection: &str, id: &str) -> StorageResult<bool> {
        let mut guard = self.data.lock().await;
        let removed = guard
            .get_mut(collection)
            .and_then(|c| c.remove(id))
            .is_some();
        Ok(removed)
    }

    async fn query_raw(&self, collection: &str, filter: &Filter) -> StorageResult<Vec<String>> {
        let guard = self.data.lock().await;
        let items = guard
            .get(collection)
            .map(|c| c.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(items
            .into_iter()
            .filter(|s| filter_matches(s, filter))
            .collect())
    }
}

// ── DiskStorage (SQLite) ──────────────────────────────────────────────────────

/// SQLite-backed storage using the bundled `rusqlite` library.
///
/// Schema:
/// ```sql
/// CREATE TABLE kv (
///     collection  TEXT NOT NULL,
///     id          TEXT NOT NULL,
///     data        TEXT NOT NULL,
///     created_at  TEXT NOT NULL DEFAULT (datetime('now')),
///     updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
///     PRIMARY KEY (collection, id)
/// );
/// ```
///
/// Each async method runs its SQLite work inside `tokio::task::spawn_blocking`
/// to keep the async executor free.
pub struct DiskStorage {
    path: PathBuf,
}

impl DiskStorage {
    /// Open (or create) a SQLite database at `path` and ensure the `kv` table
    /// exists. Returns an error if the file cannot be opened.
    pub fn new(path: impl Into<PathBuf>) -> StorageResult<Self> {
        let path = path.into();
        let conn = rusqlite::Connection::open(&path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (
                collection  TEXT NOT NULL,
                id          TEXT NOT NULL,
                data        TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (collection, id)
            );
            CREATE INDEX IF NOT EXISTS idx_kv_collection ON kv(collection);",
        )?;
        Ok(Self { path })
    }
}

#[async_trait]
impl Storage for DiskStorage {
    async fn get_raw(&self, collection: &str, id: &str) -> StorageResult<Option<String>> {
        let path = self.path.clone();
        let collection = collection.to_owned();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || -> StorageResult<Option<String>> {
            let conn = rusqlite::Connection::open(&path)?;
            let result = conn.query_row(
                "SELECT data FROM kv WHERE collection = ?1 AND id = ?2",
                rusqlite::params![collection, id],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(data) => Ok(Some(data)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(StorageError::Sqlite(e)),
            }
        })
        .await?
    }

    async fn list_raw(&self, collection: &str) -> StorageResult<Vec<String>> {
        let path = self.path.clone();
        let collection = collection.to_owned();
        tokio::task::spawn_blocking(move || -> StorageResult<Vec<String>> {
            let conn = rusqlite::Connection::open(&path)?;
            let mut stmt = conn.prepare("SELECT data FROM kv WHERE collection = ?1")?;
            let rows = stmt
                .query_map(rusqlite::params![collection], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?
    }

    async fn put_raw(&self, collection: &str, id: &str, data: &str) -> StorageResult<()> {
        let path = self.path.clone();
        let collection = collection.to_owned();
        let id = id.to_owned();
        let data = data.to_owned();
        tokio::task::spawn_blocking(move || -> StorageResult<()> {
            let conn = rusqlite::Connection::open(&path)?;
            conn.execute(
                "INSERT INTO kv (collection, id, data, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(collection, id)
                 DO UPDATE SET data = excluded.data, updated_at = datetime('now')",
                rusqlite::params![collection, id, data],
            )?;
            Ok(())
        })
        .await?
    }

    async fn delete(&self, collection: &str, id: &str) -> StorageResult<bool> {
        let path = self.path.clone();
        let collection = collection.to_owned();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || -> StorageResult<bool> {
            let conn = rusqlite::Connection::open(&path)?;
            let n = conn.execute(
                "DELETE FROM kv WHERE collection = ?1 AND id = ?2",
                rusqlite::params![collection, id],
            )?;
            Ok(n > 0)
        })
        .await?
    }

    async fn query_raw(&self, collection: &str, filter: &Filter) -> StorageResult<Vec<String>> {
        // Fetch all, filter in Rust — avoids complex SQLite JSON path handling.
        let all = self.list_raw(collection).await?;
        Ok(all
            .into_iter()
            .filter(|s| filter_matches(s, filter))
            .collect())
    }
}

// ── PostgresStorage ───────────────────────────────────────────────────────────

/// PostgreSQL-backed storage using the shared `CavePool`.
///
/// Each module gets its own schema (`cave_{module}`) with a single `kv` table:
/// ```sql
/// CREATE TABLE cave_{module}.kv (
///     collection  TEXT NOT NULL,
///     id          TEXT NOT NULL,
///     data        TEXT NOT NULL,
///     created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
///     updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
///     PRIMARY KEY (collection, id)
/// );
/// ```
pub struct PostgresStorage {
    pool: Arc<CavePool>,
    schema: String,
}

impl PostgresStorage {
    /// Create a `PostgresStorage` for the given `module` name, ensuring the
    /// schema and `kv` table exist. Mirrors the naming convention used by
    /// `CavePool::ensure_schema` (`cave_{module}`).
    pub async fn new(pool: Arc<CavePool>, module: &str) -> StorageResult<Self> {
        pool.ensure_schema(module)
            .await
            .map_err(StorageError::Database)?;

        let schema = format!("cave_{module}");
        let client = pool
            .get()
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        client
            .execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS \"{schema}\".kv (
                        collection  TEXT        NOT NULL,
                        id          TEXT        NOT NULL,
                        data        TEXT        NOT NULL,
                        created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                        PRIMARY KEY (collection, id)
                    )"
                ),
                &[],
            )
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        client
            .execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS idx_kv_collection \
                     ON \"{schema}\".kv(collection)"
                ),
                &[],
            )
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(Self { pool, schema })
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn get_raw(&self, collection: &str, id: &str) -> StorageResult<Option<String>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                &format!(
                    "SELECT data FROM \"{}\".kv WHERE collection = $1 AND id = $2",
                    self.schema
                ),
                &[&collection, &id],
            )
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    async fn list_raw(&self, collection: &str) -> StorageResult<Vec<String>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = client
            .query(
                &format!(
                    "SELECT data FROM \"{}\".kv WHERE collection = $1",
                    self.schema
                ),
                &[&collection],
            )
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.get::<_, String>(0)).collect())
    }

    async fn put_raw(&self, collection: &str, id: &str, data: &str) -> StorageResult<()> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        client
            .execute(
                &format!(
                    "INSERT INTO \"{}\".kv (collection, id, data)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (collection, id)
                     DO UPDATE SET data = EXCLUDED.data, updated_at = NOW()",
                    self.schema
                ),
                &[&collection, &id, &data],
            )
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(())
    }

    async fn delete(&self, collection: &str, id: &str) -> StorageResult<bool> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let n = client
            .execute(
                &format!(
                    "DELETE FROM \"{}\".kv WHERE collection = $1 AND id = $2",
                    self.schema
                ),
                &[&collection, &id],
            )
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(n > 0)
    }

    async fn query_raw(&self, collection: &str, filter: &Filter) -> StorageResult<Vec<String>> {
        // Fetch all items for the collection, then filter in Rust.
        // For high-cardinality use cases, push the filter down to SQL via
        // JSONB operators; this in-memory path is intentionally simple.
        let all = self.list_raw(collection).await?;
        Ok(all
            .into_iter()
            .filter(|s| filter_matches(s, filter))
            .collect())
    }
}
