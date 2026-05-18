// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cursor management for MongoDB find operations.

use crate::bson::Document;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Cursor {
    pub id: i64,
    pub ns: String,
    pub documents: Vec<Document>,
    pub batch_size: i32,
    pub index: usize,
}

impl Cursor {
    pub fn new(id: i64, ns: String, documents: Vec<Document>, batch_size: i32) -> Self {
        Self {
            id,
            ns,
            documents,
            batch_size,
            index: 0,
        }
    }

    pub fn next_batch(&mut self) -> Vec<Document> {
        let end = std::cmp::min(self.index + self.batch_size as usize, self.documents.len());
        let batch = self.documents[self.index..end].to_vec();
        self.index = end;
        batch
    }

    pub fn has_more(&self) -> bool {
        self.index < self.documents.len()
    }
}

pub struct CursorStore {
    cursors: Arc<RwLock<HashMap<i64, Cursor>>>,
    counter: AtomicI64,
}

impl CursorStore {
    pub fn new() -> Self {
        Self {
            cursors: Arc::new(RwLock::new(HashMap::new())),
            counter: AtomicI64::new(1),
        }
    }

    pub async fn create(&self, ns: String, documents: Vec<Document>, batch_size: i32) -> i64 {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let cursor = Cursor::new(id, ns, documents, batch_size);
        self.cursors.write().await.insert(id, cursor);
        id
    }

    pub async fn get_mut<F, T>(&self, id: i64, f: F) -> Option<T>
    where
        F: FnOnce(&mut Cursor) -> T,
    {
        let mut cursors = self.cursors.write().await;
        cursors.get_mut(&id).map(f)
    }

    pub async fn kill(&self, id: i64) -> bool {
        self.cursors.write().await.remove(&id).is_some()
    }
}

impl Default for CursorStore {
    fn default() -> Self {
        Self::new()
    }
}
