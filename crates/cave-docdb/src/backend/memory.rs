// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: FerretDB/FerretDB@27653503a28b0d2f2a09d7105887edd3a8af1060 internal/backends/memory/memory.go
//! In-memory `Backend` — the reference implementation.
//!
//! Wraps the existing `crate::engine::Engine` (an `Arc`-tree of
//! `BTreeMap<doc_id, Document>`) and adapts its async API to the
//! `Backend` trait shape. No persistence — for tests + dev use.

use super::{Backend, BackendError, BsonDoc, WriteResult};
use crate::engine::Engine;
use crate::query::matches_query;
use crate::update::apply_update;
use async_trait::async_trait;
use serde_json::Value;

/// In-memory backend.
#[derive(Default)]
pub struct MemoryBackend {
    engine: Engine,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self { engine: Engine::new() }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    fn doc_to_bson(doc: BsonDoc) -> BsonDoc {
        doc
    }
}

#[async_trait]
impl Backend for MemoryBackend {
    fn name(&self) -> &'static str {
        "memory"
    }

    async fn insert(
        &self,
        db: &str,
        coll: &str,
        docs: Vec<BsonDoc>,
    ) -> Result<WriteResult, BackendError> {
        let database = self.engine.get_or_create_database(db).await;
        let collection = database.get_or_create_collection(coll).await;
        let mut affected = 0u64;
        for d in docs {
            collection
                .insert_one(Self::doc_to_bson(d))
                .await
                .map_err(BackendError::InvalidDocument)?;
            affected += 1;
        }
        Ok(WriteResult { affected })
    }

    async fn find(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<Vec<BsonDoc>, BackendError> {
        let dbs = self.engine.databases.read().await;
        let Some(database) = dbs.get(db) else { return Ok(vec![]) };
        let Some(collection) = database.get_collection(coll).await else { return Ok(vec![]) };
        let docs = collection.documents.read().await;
        Ok(docs
            .values()
            .filter(|d| match &filter {
                Some(f) => matches_query(d, f),
                None => true,
            })
            .cloned()
            .collect())
    }

    async fn update(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
        update: BsonDoc,
    ) -> Result<WriteResult, BackendError> {
        let database = self.engine.get_or_create_database(db).await;
        let collection = database.get_or_create_collection(coll).await;
        let mut docs = collection.documents.write().await;
        let mut affected = 0u64;
        let ids: Vec<String> = docs
            .iter()
            .filter_map(|(id, d)| {
                let m = match &filter {
                    Some(f) => matches_query(d, f),
                    None => true,
                };
                if m {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in ids {
            if let Some(d) = docs.get_mut(&id) {
                apply_update(d, &update).map_err(BackendError::InvalidDocument)?;
                affected += 1;
            }
        }
        Ok(WriteResult { affected })
    }

    async fn delete(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<WriteResult, BackendError> {
        let dbs = self.engine.databases.read().await;
        let Some(database) = dbs.get(db) else { return Ok(WriteResult { affected: 0 }) };
        let Some(collection) = database.get_collection(coll).await else {
            return Ok(WriteResult { affected: 0 });
        };
        let mut docs = collection.documents.write().await;
        let ids: Vec<String> = docs
            .iter()
            .filter_map(|(id, d)| {
                let m = match &filter {
                    Some(f) => matches_query(d, f),
                    None => true,
                };
                if m {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        let n = ids.len() as u64;
        for id in ids {
            docs.remove(&id);
        }
        Ok(WriteResult { affected: n })
    }

    async fn count(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<u64, BackendError> {
        let rows = self.find(db, coll, filter).await?;
        Ok(rows.len() as u64)
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>, BackendError> {
        let dbs = self.engine.databases.read().await;
        let Some(database) = dbs.get(db) else { return Ok(vec![]) };
        let cols = database.collections.read().await;
        Ok(cols.keys().cloned().collect())
    }

    async fn list_databases(&self) -> Result<Vec<String>, BackendError> {
        Ok(self.engine.databases.read().await.keys().cloned().collect())
    }

    async fn drop_collection(&self, db: &str, coll: &str) -> Result<(), BackendError> {
        let dbs = self.engine.databases.read().await;
        let Some(database) = dbs.get(db) else { return Ok(()) };
        database.drop_collection(coll).await.map_err(BackendError::Io)
    }

    async fn drop_database(&self, db: &str) -> Result<(), BackendError> {
        self.engine.databases.write().await.remove(db);
        Ok(())
    }
}

// Helper to make `Value` arrays usable in update operators.
#[allow(dead_code)]
fn coerce_array(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        _ => vec![v.clone()],
    }
}

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

    #[tokio::test]
    async fn name_is_memory() {
        assert_eq!(MemoryBackend::new().name(), "memory");
    }

    #[tokio::test]
    async fn find_on_missing_db_is_empty() {
        let b = MemoryBackend::new();
        let r = b.find("nope", "nope", None).await.unwrap();
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn delete_on_missing_db_is_zero() {
        let b = MemoryBackend::new();
        let r = b.delete("nope", "nope", None).await.unwrap();
        assert_eq!(r.affected, 0);
    }

    #[tokio::test]
    async fn drop_db_idempotent() {
        let b = MemoryBackend::new();
        b.drop_database("never_existed").await.unwrap();
        b.insert("d", "c", vec![doc(json!({"a":1}))]).await.unwrap();
        b.drop_database("d").await.unwrap();
        b.drop_database("d").await.unwrap();
    }
}
