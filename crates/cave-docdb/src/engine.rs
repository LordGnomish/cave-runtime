// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory document storage engine.

use crate::bson::Document;
use crate::index::Index;
use crate::query::matches_query;
use crate::update::apply_update;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Collection {
    pub documents: Arc<RwLock<BTreeMap<String, Document>>>,
    pub indexes: Arc<RwLock<Vec<Index>>>,
    pub doc_counter: Arc<tokio::sync::Mutex<u64>>,
}

impl Collection {
    fn new() -> Self {
        Self {
            documents: Arc::new(RwLock::new(BTreeMap::new())),
            indexes: Arc::new(RwLock::new(Vec::new())),
            doc_counter: Arc::new(tokio::sync::Mutex::new(0)),
        }
    }

    pub async fn insert_one(&self, mut doc: Document) -> Result<String, String> {
        // Auto-generate _id if missing
        if !doc.contains_key("_id") {
            let mut counter = self.doc_counter.lock().await;
            *counter += 1;
            let id = format!("{:024x}", *counter);
            doc.insert("_id".to_string(), Value::String(id));
        }

        let id = doc
            .get("_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                doc.get("_id")
                    .and_then(|v| v.as_i64().map(|n| n.to_string()))
            })
            .ok_or_else(|| "missing _id".to_string())?;

        let mut docs = self.documents.write().await;
        docs.insert(id.clone(), doc);
        Ok(id)
    }

    pub async fn insert_many(&self, docs: Vec<Document>) -> Result<Vec<String>, String> {
        let mut ids = Vec::new();
        for doc in docs {
            let id = self.insert_one(doc).await?;
            ids.push(id);
        }
        Ok(ids)
    }

    pub async fn find(&self, filter: Option<&Document>) -> Result<Vec<Document>, String> {
        let docs = self.documents.read().await;
        let mut results = Vec::new();

        for (_id, doc) in docs.iter() {
            if let Some(f) = filter {
                if matches_query(doc, f) {
                    results.push(doc.clone());
                }
            } else {
                results.push(doc.clone());
            }
        }

        Ok(results)
    }

    pub async fn find_one(&self, filter: Option<&Document>) -> Result<Option<Document>, String> {
        let docs = self.documents.read().await;

        for (_id, doc) in docs.iter() {
            if let Some(f) = filter {
                if matches_query(doc, f) {
                    return Ok(Some(doc.clone()));
                }
            } else {
                return Ok(Some(doc.clone()));
            }
        }

        Ok(None)
    }

    pub async fn update_many(
        &self,
        filter: Option<&Document>,
        update: &Document,
    ) -> Result<u64, String> {
        let mut docs = self.documents.write().await;
        let mut count = 0;

        let ids_to_update: Vec<String> = docs
            .iter()
            .filter_map(|(id, doc)| {
                let matches = if let Some(f) = filter {
                    matches_query(doc, f)
                } else {
                    true
                };
                if matches { Some(id.clone()) } else { None }
            })
            .collect();

        for id in ids_to_update {
            if let Some(doc) = docs.get_mut(&id) {
                apply_update(doc, update)?;
                count += 1;
            }
        }

        Ok(count)
    }

    pub async fn delete_many(&self, filter: Option<&Document>) -> Result<u64, String> {
        let mut docs = self.documents.write().await;
        let initial_len = docs.len();

        let ids_to_delete: Vec<String> = docs
            .iter()
            .filter_map(|(id, doc)| {
                let matches = if let Some(f) = filter {
                    matches_query(doc, f)
                } else {
                    true
                };
                if matches { Some(id.clone()) } else { None }
            })
            .collect();

        for id in ids_to_delete {
            docs.remove(&id);
        }

        Ok((initial_len - docs.len()) as u64)
    }

    pub async fn count(&self, filter: Option<&Document>) -> Result<u64, String> {
        let docs = self.documents.read().await;
        let count = docs
            .iter()
            .filter(|(_id, doc)| {
                if let Some(f) = filter {
                    matches_query(doc, f)
                } else {
                    true
                }
            })
            .count();
        Ok(count as u64)
    }

    pub async fn drop(&self) -> Result<(), String> {
        self.documents.write().await.clear();
        self.indexes.write().await.clear();
        Ok(())
    }

    pub async fn add_index(&self, index: Index) -> Result<(), String> {
        let mut indexes = self.indexes.write().await;
        indexes.push(index);
        Ok(())
    }

    pub async fn list_indexes(&self) -> Result<Vec<Index>, String> {
        Ok(self.indexes.read().await.clone())
    }

    pub async fn drop_index(&self, name: &str) -> Result<(), String> {
        let mut indexes = self.indexes.write().await;
        indexes.retain(|idx| idx.name != name);
        Ok(())
    }

    pub async fn stats(&self) -> Result<CollectionStats, String> {
        let docs = self.documents.read().await;
        let indexes = self.indexes.read().await;
        Ok(CollectionStats {
            document_count: docs.len() as u64,
            index_count: indexes.len() as u64,
        })
    }
}

#[derive(Clone)]
pub struct Database {
    pub collections: Arc<RwLock<HashMap<String, Collection>>>,
}

impl Database {
    fn new() -> Self {
        Self {
            collections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_create_collection(&self, name: &str) -> Collection {
        let mut cols = self.collections.write().await;
        cols.entry(name.to_string())
            .or_insert_with(Collection::new)
            .clone()
    }

    pub async fn get_collection(&self, name: &str) -> Option<Collection> {
        self.collections.read().await.get(name).cloned()
    }

    pub async fn list_collections(&self) -> Result<Vec<String>, String> {
        Ok(self.collections.read().await.keys().cloned().collect())
    }

    pub async fn drop_collection(&self, name: &str) -> Result<(), String> {
        self.collections.write().await.remove(name);
        Ok(())
    }

    pub async fn drop(&self) -> Result<(), String> {
        self.collections.write().await.clear();
        Ok(())
    }
}

pub struct Engine {
    pub databases: Arc<RwLock<HashMap<String, Database>>>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            databases: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_create_database(&self, name: &str) -> Database {
        let mut dbs = self.databases.write().await;
        dbs.entry(name.to_string())
            .or_insert_with(Database::new)
            .clone()
    }

    pub async fn get_database(&self, name: &str) -> Option<Database> {
        self.databases.read().await.get(name).cloned()
    }

    pub async fn list_databases(&self) -> Result<Vec<String>, String> {
        Ok(self.databases.read().await.keys().cloned().collect())
    }

    pub async fn drop_database(&self, name: &str) -> Result<(), String> {
        self.databases.write().await.remove(name);
        Ok(())
    }

    pub async fn stats(&self) -> Result<EngineStats, String> {
        let databases = self.databases.read().await;
        let mut db_count = 0;
        let mut col_count = 0;
        let mut doc_count = 0;

        for db in databases.values() {
            db_count += 1;
            let cols = db.collections.read().await;
            for col in cols.values() {
                col_count += 1;
                doc_count += col.documents.read().await.len();
            }
        }

        Ok(EngineStats {
            database_count: db_count,
            collection_count: col_count,
            document_count: doc_count as u64,
        })
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionStats {
    pub document_count: u64,
    pub index_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStats {
    pub database_count: u64,
    pub collection_count: u64,
    pub document_count: u64,
}

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_insert_and_find() {
        let col = Collection::new();
        let mut doc = Document::new();
        doc.insert("name".to_string(), Value::String("alice".to_string()));
        col.insert_one(doc).await.unwrap();

        let found = col.find(None).await.unwrap();
        assert_eq!(found.len(), 1);
    }

    #[tokio::test]
    async fn test_find_with_filter() {
        let col = Collection::new();
        let mut doc1 = Document::new();
        doc1.insert("status".to_string(), Value::String("active".to_string()));
        col.insert_one(doc1).await.unwrap();

        let mut doc2 = Document::new();
        doc2.insert("status".to_string(), Value::String("inactive".to_string()));
        col.insert_one(doc2).await.unwrap();

        let mut filter = Document::new();
        filter.insert("status".to_string(), Value::String("active".to_string()));
        let found = col.find(Some(&filter)).await.unwrap();
        assert_eq!(found.len(), 1);
    }

    #[tokio::test]
    async fn test_delete_many() {
        let col = Collection::new();
        for i in 0..5 {
            let mut doc = Document::new();
            doc.insert("value".to_string(), Value::Number(i.into()));
            col.insert_one(doc).await.unwrap();
        }

        let deleted = col.delete_many(None).await.unwrap();
        assert_eq!(deleted, 5);

        let remaining = col.find(None).await.unwrap();
        assert_eq!(remaining.len(), 0);
    }

    #[tokio::test]
    async fn test_count() {
        let col = Collection::new();
        for i in 0..3 {
            let mut doc = Document::new();
            doc.insert("idx".to_string(), Value::Number(i.into()));
            col.insert_one(doc).await.unwrap();
        }

        let count = col.count(None).await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_database_operations() {
        let db = Database::new();
        let col = db.get_or_create_collection("test").await;

        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::String("value".to_string()));
        col.insert_one(doc).await.unwrap();

        let collections = db.list_collections().await.unwrap();
        assert!(collections.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn test_engine_operations() {
        let engine = Engine::new();
        let db = engine.get_or_create_database("testdb").await;
        let col = db.get_or_create_collection("testcol").await;

        let mut doc = Document::new();
        doc.insert("x".to_string(), Value::Number(1.into()));
        col.insert_one(doc).await.unwrap();

        let databases = engine.list_databases().await.unwrap();
        assert!(databases.contains(&"testdb".to_string()));
    }
}
