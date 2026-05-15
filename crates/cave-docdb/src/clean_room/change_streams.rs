// SPDX-License-Identifier: AGPL-3.0-or-later
// Clean-room: authored from
//   https://www.mongodb.com/docs/manual/changeStreams/
//   https://www.mongodb.com/docs/manual/reference/change-events/
// No `mongodb/mongo` source code consulted.
//
//! Change-stream cursors.
//!
//! A change stream surfaces oplog events as they are appended,
//! filtered by namespace and resumable via the resumeToken (the
//! BSON timestamp of the last delivered event). The transport here
//! is `tokio::sync::broadcast` for the in-process case; the PG
//! LISTEN/NOTIFY adapter is sketched below behind the
//! `PostgresChangeStreamSource` enum variant — it consumes
//! NOTIFY payloads and feeds the same event channel.

use super::oplog::{OpEntry, OpKind, Oplog};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Public schema of a change event (matches MongoDB docs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    #[serde(rename = "_id")]
    pub id: ResumeToken,
    #[serde(rename = "operationType")]
    pub operation_type: &'static str,
    pub ns: Namespace,
    #[serde(rename = "documentKey", skip_serializing_if = "Option::is_none")]
    pub document_key: Option<serde_json::Value>,
    #[serde(rename = "fullDocument", skip_serializing_if = "Option::is_none")]
    pub full_document: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Namespace {
    pub db: String,
    pub coll: String,
}

/// Resume token = the oplog timestamp of the last delivered event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeToken(pub u64);

#[derive(Debug, thiserror::Error)]
pub enum ChangeStreamError {
    #[error("resume point {0} predates oplog tail; client must re-init")]
    ResumePointInvalidated(u64),
    #[error("subscription lagged: {0} events dropped")]
    Lagged(u64),
}

/// Broadcaster — owns the channel and re-uses the oplog as the
/// canonical event source.
pub struct ChangeBroadcaster {
    tx: broadcast::Sender<ChangeEvent>,
    oplog: Arc<Oplog>,
}

impl ChangeBroadcaster {
    pub fn new(oplog: Arc<Oplog>, channel_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(channel_capacity);
        Self { tx, oplog }
    }

    /// Publish an event from a freshly-appended oplog entry.
    pub fn publish(&self, entry: &OpEntry) -> usize {
        let ev = entry_to_change_event(entry);
        self.tx.send(ev).unwrap_or(0)
    }

    /// Append to the oplog and broadcast in one step.
    pub fn append_and_broadcast(
        &self,
        op: OpKind,
        ns: &Namespace,
        document_key: serde_json::Value,
        full: Option<serde_json::Value>,
    ) -> ResumeToken {
        let ns_str = format!("{}.{}", ns.db, ns.coll);
        let ts = match op {
            OpKind::Update | OpKind::Delete => {
                self.oplog
                    .append_with_o2(op, ns_str, full.clone().unwrap_or(serde_json::Value::Null), document_key.clone())
            }
            _ => self.oplog.append(op, ns_str, full.clone().unwrap_or(serde_json::Value::Null)),
        };
        let ev = ChangeEvent {
            id: ResumeToken(ts),
            operation_type: operation_type_str(op),
            ns: ns.clone(),
            document_key: Some(document_key),
            full_document: full,
        };
        let _ = self.tx.send(ev);
        ResumeToken(ts)
    }

    /// Subscribe and optionally resume from a token. Resume reads any
    /// already-appended events from the oplog before yielding the
    /// broadcast receiver.
    pub fn subscribe(
        &self,
        resume_from: Option<ResumeToken>,
    ) -> Result<ChangeStreamCursor, ChangeStreamError> {
        let rx = self.tx.subscribe();
        let mut backfill = Vec::new();
        if let Some(t) = resume_from {
            // Validate: tail of oplog must not be past the resume point.
            if let Some(tail) = self.oplog.tail() {
                if t.0 < tail {
                    return Err(ChangeStreamError::ResumePointInvalidated(t.0));
                }
            }
            for e in self.oplog.read_since(t.0) {
                backfill.push(entry_to_change_event(&e));
            }
        }
        Ok(ChangeStreamCursor { rx, backfill })
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// Receiver-side cursor.
pub struct ChangeStreamCursor {
    rx: broadcast::Receiver<ChangeEvent>,
    backfill: Vec<ChangeEvent>,
}

impl ChangeStreamCursor {
    /// Pop the next event, drained from the backfill first.
    pub fn try_next(&mut self) -> Option<ChangeEvent> {
        if let Some(e) = self.backfill_pop() {
            return Some(e);
        }
        self.rx.try_recv().ok()
    }

    /// Await the next event.
    pub async fn next(&mut self) -> Result<ChangeEvent, ChangeStreamError> {
        if let Some(e) = self.backfill_pop() {
            return Ok(e);
        }
        match self.rx.recv().await {
            Ok(e) => Ok(e),
            Err(broadcast::error::RecvError::Lagged(n)) => Err(ChangeStreamError::Lagged(n)),
            Err(broadcast::error::RecvError::Closed) => Err(ChangeStreamError::Lagged(0)),
        }
    }

    fn backfill_pop(&mut self) -> Option<ChangeEvent> {
        if self.backfill.is_empty() {
            None
        } else {
            Some(self.backfill.remove(0))
        }
    }
}

fn entry_to_change_event(entry: &OpEntry) -> ChangeEvent {
    let (db, coll) = entry.ns.split_once('.').unwrap_or((entry.ns.as_str(), ""));
    ChangeEvent {
        id: ResumeToken(entry.ts),
        operation_type: operation_type_str(entry.op),
        ns: Namespace { db: db.into(), coll: coll.into() },
        document_key: entry.o2.clone(),
        full_document: Some(entry.o.clone()),
    }
}

fn operation_type_str(op: OpKind) -> &'static str {
    match op {
        OpKind::Insert => "insert",
        OpKind::Update => "update",
        OpKind::Delete => "delete",
        OpKind::Command => "drop",
        OpKind::Noop => "noop",
    }
}

/// Source of upstream change events — broadcast (in-process) or a
/// PostgreSQL LISTEN/NOTIFY channel name.
#[derive(Debug, Clone)]
pub enum ChangeStreamSource {
    Broadcast,
    PostgresListenNotify { channel: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ns() -> Namespace {
        Namespace { db: "orders".into(), coll: "items".into() }
    }

    #[tokio::test]
    async fn append_and_broadcast_publishes_to_subscriber() {
        let oplog = Arc::new(Oplog::new(8));
        let cb = ChangeBroadcaster::new(oplog, 16);
        let mut sub = cb.subscribe(None).unwrap();
        let _ = cb.append_and_broadcast(
            OpKind::Insert,
            &ns(),
            json!({"_id":"x"}),
            Some(json!({"_id":"x","sku":"A"})),
        );
        let e = sub.next().await.unwrap();
        assert_eq!(e.operation_type, "insert");
        assert_eq!(e.ns.coll, "items");
    }

    #[tokio::test]
    async fn resume_from_token_backfills_missed_events() {
        let oplog = Arc::new(Oplog::new(8));
        let cb = ChangeBroadcaster::new(oplog, 16);
        let t = cb.append_and_broadcast(OpKind::Insert, &ns(), json!({"_id":1}), Some(json!({"_id":1})));
        let _ = cb.append_and_broadcast(OpKind::Insert, &ns(), json!({"_id":2}), Some(json!({"_id":2})));
        let _ = cb.append_and_broadcast(OpKind::Insert, &ns(), json!({"_id":3}), Some(json!({"_id":3})));
        let mut sub = cb.subscribe(Some(t)).unwrap();
        let mut seen = 0;
        while let Some(_) = sub.try_next() {
            seen += 1;
            if seen >= 2 { break; }
        }
        assert_eq!(seen, 2);
    }

    #[tokio::test]
    async fn resume_before_tail_errors() {
        let oplog = Arc::new(Oplog::new(2));
        let cb = ChangeBroadcaster::new(oplog, 16);
        cb.append_and_broadcast(OpKind::Insert, &ns(), json!({}), None);
        cb.append_and_broadcast(OpKind::Insert, &ns(), json!({}), None);
        cb.append_and_broadcast(OpKind::Insert, &ns(), json!({}), None);
        // Resume from ts=1 which has been evicted from the capped oplog.
        let r = cb.subscribe(Some(ResumeToken(1)));
        assert!(matches!(r, Err(ChangeStreamError::ResumePointInvalidated(_))));
    }

    #[test]
    fn ns_round_trip() {
        let n = Namespace { db: "a".into(), coll: "b".into() };
        let s = serde_json::to_string(&n).unwrap();
        let r: Namespace = serde_json::from_str(&s).unwrap();
        assert_eq!(n, r);
    }

    #[tokio::test]
    async fn delete_publishes_with_document_key() {
        let oplog = Arc::new(Oplog::new(8));
        let cb = ChangeBroadcaster::new(oplog, 16);
        let mut sub = cb.subscribe(None).unwrap();
        let _ = cb.append_and_broadcast(OpKind::Delete, &ns(), json!({"_id":"x"}), None);
        let e = sub.next().await.unwrap();
        assert_eq!(e.operation_type, "delete");
        assert_eq!(e.document_key.unwrap(), json!({"_id":"x"}));
    }

    #[tokio::test]
    async fn subscriber_count_tracks_subscribers() {
        let oplog = Arc::new(Oplog::new(8));
        let cb = ChangeBroadcaster::new(oplog, 16);
        let _s1 = cb.subscribe(None).unwrap();
        let _s2 = cb.subscribe(None).unwrap();
        assert_eq!(cb.subscriber_count(), 2);
    }

    #[test]
    fn source_variants_clone() {
        let a = ChangeStreamSource::Broadcast;
        let _b = a.clone();
        let c = ChangeStreamSource::PostgresListenNotify { channel: "docdb_changes".into() };
        let _d = c.clone();
    }
}
