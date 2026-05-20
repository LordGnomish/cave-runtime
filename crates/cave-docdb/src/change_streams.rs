// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Change streams — parity with `src/mongo/db/pipeline/change_stream.cpp`
//! (MongoDB r7.0.0 spec) and FerretDB's change-stream handler.
//!
//! A change stream is a tail-cursor over the oplog filtered to events
//! affecting a specific collection / database / cluster. Events are
//! emitted with a `_id` (resume token), an `operationType` (insert /
//! update / replace / delete / drop / invalidate), the affected
//! `documentKey`, and either `fullDocument` (insert / update with
//! `fullDocument="updateLookup"`) or `updateDescription`.

use crate::bson::Document;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationType {
    Insert,
    Update,
    Replace,
    Delete,
    Drop,
    Invalidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeToken {
    pub timestamp_ms: i64,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateDescription {
    pub updated_fields: Document,
    pub removed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChangeEvent {
    pub id: ResumeToken,
    pub operation_type: OperationType,
    pub database: String,
    pub collection: String,
    pub document_key: Document,
    pub full_document: Option<Document>,
    pub update_description: Option<UpdateDescription>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamScope {
    /// All databases.
    Cluster,
    /// Single database, all its collections.
    Database(String),
    /// Single collection.
    Collection { database: String, collection: String },
}

#[derive(Debug, Default)]
pub struct ChangeStreamBus {
    events: Mutex<Vec<ChangeEvent>>,
    next_seq: Mutex<u64>,
}

impl ChangeStreamBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Publish an event onto the oplog. Sets the resume-token seq.
    pub fn publish(&self, mut ev: ChangeEvent) -> ResumeToken {
        let mut next = self.next_seq.lock().unwrap();
        ev.id.seq = *next;
        *next += 1;
        let token = ev.id.clone();
        self.events.lock().unwrap().push(ev);
        token
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.lock().unwrap().is_empty()
    }
}

/// Server-side cursor view over the bus filtered to a scope.
#[derive(Debug)]
pub struct ChangeStreamCursor {
    bus: Arc<ChangeStreamBus>,
    scope: StreamScope,
    next_index: usize,
    resume_after: Option<ResumeToken>,
}

impl ChangeStreamCursor {
    pub fn open(bus: Arc<ChangeStreamBus>, scope: StreamScope) -> Self {
        let next_index = bus.events.lock().unwrap().len();
        Self {
            bus,
            scope,
            next_index,
            resume_after: None,
        }
    }

    pub fn resume_after(mut self, token: ResumeToken) -> Self {
        self.resume_after = Some(token);
        self.next_index = 0;
        self
    }

    /// Pull the next batch of events. Returns the events that match
    /// the cursor's scope + resume position, in oplog order.
    pub fn next_batch(&mut self) -> Vec<ChangeEvent> {
        let events = self.bus.events.lock().unwrap();
        let mut out = Vec::new();
        let mut idx = self.next_index;
        while idx < events.len() {
            let ev = &events[idx];
            idx += 1;
            if let Some(ref tok) = self.resume_after {
                if (ev.id.timestamp_ms, ev.id.seq) <= (tok.timestamp_ms, tok.seq) {
                    continue;
                }
            }
            if !scope_matches(&self.scope, ev) {
                continue;
            }
            out.push(ev.clone());
        }
        // Once we've delivered up to resume_after, drop it for subsequent pulls.
        self.resume_after = None;
        self.next_index = idx;
        out
    }
}

fn scope_matches(scope: &StreamScope, ev: &ChangeEvent) -> bool {
    match scope {
        StreamScope::Cluster => true,
        StreamScope::Database(db) => &ev.database == db,
        StreamScope::Collection { database, collection } => {
            &ev.database == database && &ev.collection == collection
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(db: &str, coll: &str, op: OperationType) -> ChangeEvent {
        let mut key = Document::new();
        key.insert("_id".into(), json!(1));
        ChangeEvent {
            id: ResumeToken {
                timestamp_ms: 0,
                seq: 0,
            },
            operation_type: op,
            database: db.into(),
            collection: coll.into(),
            document_key: key,
            full_document: None,
            update_description: None,
        }
    }

    #[test]
    fn publish_assigns_monotonic_seq() {
        let bus = ChangeStreamBus::new();
        let t1 = bus.publish(ev("test", "users", OperationType::Insert));
        let t2 = bus.publish(ev("test", "users", OperationType::Insert));
        assert!(t2.seq > t1.seq);
        assert_eq!(bus.len(), 2);
    }

    #[test]
    fn cursor_at_collection_scope_filters() {
        let bus = ChangeStreamBus::new();
        bus.publish(ev("a", "users", OperationType::Insert));
        bus.publish(ev("a", "orders", OperationType::Insert));
        let mut c = ChangeStreamCursor::open(
            Arc::clone(&bus),
            StreamScope::Collection {
                database: "a".into(),
                collection: "orders".into(),
            },
        );
        // open after publish so cursor's initial index is past existing events;
        // publish a new event matching the scope and one not matching.
        bus.publish(ev("a", "users", OperationType::Insert));
        bus.publish(ev("a", "orders", OperationType::Update));
        let batch = c.next_batch();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].collection, "orders");
        assert_eq!(batch[0].operation_type, OperationType::Update);
    }

    #[test]
    fn cursor_at_database_scope_returns_all_collections() {
        let bus = ChangeStreamBus::new();
        let mut c = ChangeStreamCursor::open(Arc::clone(&bus), StreamScope::Database("a".into()));
        bus.publish(ev("a", "users", OperationType::Insert));
        bus.publish(ev("a", "orders", OperationType::Insert));
        bus.publish(ev("b", "users", OperationType::Insert));
        let batch = c.next_batch();
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn cursor_at_cluster_scope_returns_everything() {
        let bus = ChangeStreamBus::new();
        let mut c = ChangeStreamCursor::open(Arc::clone(&bus), StreamScope::Cluster);
        bus.publish(ev("a", "users", OperationType::Insert));
        bus.publish(ev("b", "orders", OperationType::Insert));
        let batch = c.next_batch();
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn cursor_resumes_after_token() {
        let bus = ChangeStreamBus::new();
        let _t1 = bus.publish(ev("a", "users", OperationType::Insert));
        let t2 = bus.publish(ev("a", "users", OperationType::Update));
        let _t3 = bus.publish(ev("a", "users", OperationType::Delete));
        let mut c = ChangeStreamCursor::open(Arc::clone(&bus), StreamScope::Cluster).resume_after(t2);
        let batch = c.next_batch();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].operation_type, OperationType::Delete);
    }

    #[test]
    fn empty_cursor_returns_empty_batch() {
        let bus = ChangeStreamBus::new();
        let mut c = ChangeStreamCursor::open(Arc::clone(&bus), StreamScope::Cluster);
        let batch = c.next_batch();
        assert!(batch.is_empty());
    }

    #[test]
    fn drop_and_invalidate_emitted() {
        let bus = ChangeStreamBus::new();
        let mut c = ChangeStreamCursor::open(Arc::clone(&bus), StreamScope::Cluster);
        bus.publish(ev("a", "users", OperationType::Drop));
        bus.publish(ev("a", "users", OperationType::Invalidate));
        let batch = c.next_batch();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].operation_type, OperationType::Drop);
        assert_eq!(batch[1].operation_type, OperationType::Invalidate);
    }

    #[test]
    fn second_pull_returns_only_new_events() {
        let bus = ChangeStreamBus::new();
        let mut c = ChangeStreamCursor::open(Arc::clone(&bus), StreamScope::Cluster);
        bus.publish(ev("a", "users", OperationType::Insert));
        assert_eq!(c.next_batch().len(), 1);
        bus.publish(ev("a", "users", OperationType::Update));
        assert_eq!(c.next_batch().len(), 1);
        assert_eq!(c.next_batch().len(), 0);
    }
}
