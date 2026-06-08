// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Replication oplog — a MongoDB-style operations log and applier.
//!
//! The oplog is the ordered, append-only record of every write a primary
//! applies. A secondary replays the tail to converge to the primary's state;
//! [`crate::change_streams`] is the user-facing view over the same stream.
//!
//! Entries mirror `local.oplog.rs`: a `(secs, inc)` BSON timestamp, an op type
//! (`i`/`u`/`d`/`n`/`c`), the namespace, the operation document `o`, and for
//! updates the query document `o2`. The logical clock is driven by a caller
//! supplied wall-clock second, so the whole module is deterministic and unit
//! testable without a real clock.

use crate::backend::StorageBackend;
use serde_json::Value;
use std::sync::Mutex;

/// A BSON timestamp: increasing `(secs, inc)`. Ordered lexicographically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BsonTimestamp {
    /// Seconds since the epoch (or any monotonic second source).
    pub secs: u32,
    /// Ordinal within `secs`, incremented per op in the same second.
    pub inc: u32,
}

/// Oplog operation type (`o` semantics depend on this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpType {
    /// Insert: `o` is the inserted document.
    Insert,
    /// Update: `o` is the update spec, `o2` is the query selector.
    Update,
    /// Delete: `o` is the selector.
    Delete,
    /// No-op (heartbeat / init).
    Noop,
    /// Command (e.g. createIndexes); applied opaquely.
    Command,
}

impl OpType {
    /// Single-character oplog code (`i`/`u`/`d`/`n`/`c`).
    pub fn code(&self) -> char {
        match self {
            OpType::Insert => 'i',
            OpType::Update => 'u',
            OpType::Delete => 'd',
            OpType::Noop => 'n',
            OpType::Command => 'c',
        }
    }
}

/// A single oplog record.
#[derive(Debug, Clone, PartialEq)]
pub struct OplogEntry {
    /// Timestamp / ordering key.
    pub ts: BsonTimestamp,
    /// Operation type.
    pub op: OpType,
    /// Namespace, `db.collection`.
    pub ns: String,
    /// Operation document.
    pub o: Value,
    /// Query selector (updates/deletes).
    pub o2: Option<Value>,
}

/// Append-only operations log with a monotonic logical clock.
pub struct OpLog {
    entries: Mutex<Vec<OplogEntry>>,
    clock: Mutex<BsonTimestamp>,
}

impl OpLog {
    /// New empty oplog.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            clock: Mutex::new(BsonTimestamp { secs: 0, inc: 0 }),
        }
    }

    /// Mint the next monotonic timestamp for the given wall-clock second.
    ///
    /// Within the same (or an older) second the ordinal advances; a newer
    /// second resets the ordinal to 1. The result is always strictly greater
    /// than the previous one, even if `secs` moves backwards.
    pub fn next_ts(&self, secs: u32) -> BsonTimestamp {
        let mut clock = self.clock.lock().unwrap();
        let next = if secs > clock.secs {
            BsonTimestamp { secs, inc: 1 }
        } else {
            BsonTimestamp {
                secs: clock.secs,
                inc: clock.inc + 1,
            }
        };
        *clock = next;
        next
    }

    /// Append an entry, minting its timestamp; returns the assigned ts.
    pub fn append(
        &self,
        op: OpType,
        ns: &str,
        o: Value,
        o2: Option<Value>,
        secs: u32,
    ) -> BsonTimestamp {
        let ts = self.next_ts(secs);
        self.entries.lock().unwrap().push(OplogEntry {
            ts,
            op,
            ns: ns.to_string(),
            o,
            o2,
        });
        ts
    }

    /// Entries strictly after `after` (or all entries when `None`).
    pub fn tail(&self, after: Option<BsonTimestamp>) -> Vec<OplogEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| after.map(|a| e.ts > a).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Number of recorded entries.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Whether the oplog is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The most recent timestamp, if any.
    pub fn latest_ts(&self) -> Option<BsonTimestamp> {
        self.entries.lock().unwrap().last().map(|e| e.ts)
    }
}

impl Default for OpLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Collection name from a `db.collection` namespace (drops the db prefix).
fn collection_of(ns: &str) -> &str {
    ns.split_once('.').map(|(_, c)| c).unwrap_or(ns)
}

/// Apply one oplog entry to a backend (idempotent for i/u/d).
///
/// Returns `true` if a data operation was applied, `false` for `n`/`c` no-ops.
pub async fn apply_entry(backend: &dyn StorageBackend, entry: &OplogEntry) -> Result<bool, String> {
    let coll = collection_of(&entry.ns);
    match entry.op {
        OpType::Insert => {
            backend.insert(coll, entry.o.clone()).await?;
            Ok(true)
        }
        OpType::Update => {
            let selector = entry.o2.clone().unwrap_or_else(|| Value::Object(Default::default()));
            backend.update(coll, &selector, &entry.o).await?;
            Ok(true)
        }
        OpType::Delete => {
            backend.delete(coll, &entry.o).await?;
            Ok(true)
        }
        OpType::Noop | OpType::Command => Ok(false),
    }
}

/// Replay a sequence of oplog entries onto a backend; returns ops applied.
///
/// `n`/`c` entries are skipped and do not count toward the applied total.
pub async fn replicate(
    backend: &dyn StorageBackend,
    entries: &[OplogEntry],
) -> Result<u64, String> {
    let mut applied = 0u64;
    for entry in entries {
        if apply_entry(backend, entry).await? {
            applied += 1;
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use serde_json::json;

    #[test]
    fn clock_increments_within_second_and_resets_across() {
        let log = OpLog::new();
        assert_eq!(log.next_ts(100), BsonTimestamp { secs: 100, inc: 1 });
        assert_eq!(log.next_ts(100), BsonTimestamp { secs: 100, inc: 2 });
        assert_eq!(log.next_ts(101), BsonTimestamp { secs: 101, inc: 1 });
        // A second that goes backwards still advances monotonically.
        assert_eq!(log.next_ts(50), BsonTimestamp { secs: 101, inc: 2 });
    }

    #[test]
    fn append_records_entries_in_order() {
        let log = OpLog::new();
        let t1 = log.append(OpType::Insert, "db.c", json!({"_id": "1"}), None, 10);
        let t2 = log.append(OpType::Delete, "db.c", json!({"_id": "1"}), None, 10);
        assert!(t1 < t2);
        assert_eq!(log.len(), 2);
        assert_eq!(log.latest_ts(), Some(t2));
    }

    #[test]
    fn tail_returns_only_later_entries() {
        let log = OpLog::new();
        let t1 = log.append(OpType::Insert, "db.c", json!({"_id": "1"}), None, 1);
        let _t2 = log.append(OpType::Insert, "db.c", json!({"_id": "2"}), None, 1);
        let after = log.tail(Some(t1));
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].o, json!({"_id": "2"}));
        assert_eq!(log.tail(None).len(), 2);
    }

    #[test]
    fn optype_codes() {
        assert_eq!(OpType::Insert.code(), 'i');
        assert_eq!(OpType::Update.code(), 'u');
        assert_eq!(OpType::Delete.code(), 'd');
    }

    #[tokio::test]
    async fn replication_converges_secondary_to_primary() {
        // Primary applies writes and records them to its oplog.
        let primary = MemoryBackend::new();
        let log = OpLog::new();

        primary.insert("c", json!({"_id": "1", "n": 1})).await.unwrap();
        log.append(OpType::Insert, "test.c", json!({"_id": "1", "n": 1}), None, 1);

        primary.insert("c", json!({"_id": "2", "n": 2})).await.unwrap();
        log.append(OpType::Insert, "test.c", json!({"_id": "2", "n": 2}), None, 1);

        primary
            .update("c", &json!({"_id": "1"}), &json!({"$set": {"n": 9}}))
            .await
            .unwrap();
        log.append(
            OpType::Update,
            "test.c",
            json!({"$set": {"n": 9}}),
            Some(json!({"_id": "1"})),
            2,
        );

        primary.delete("c", &json!({"_id": "2"})).await.unwrap();
        log.append(OpType::Delete, "test.c", json!({"_id": "2"}), None, 3);

        // Secondary starts empty and replays the full oplog tail.
        let secondary = MemoryBackend::new();
        let applied = replicate(&secondary, &log.tail(None)).await.unwrap();
        assert_eq!(applied, 4);

        // Converged: one doc, _id "1", n == 9.
        let docs = secondary.find("c", &json!({})).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["_id"], json!("1"));
        assert_eq!(docs[0]["n"], json!(9));
    }

    #[tokio::test]
    async fn apply_entry_noop_is_ignored() {
        let be = MemoryBackend::new();
        let entry = OplogEntry {
            ts: BsonTimestamp { secs: 1, inc: 1 },
            op: OpType::Noop,
            ns: "test.c".into(),
            o: json!({}),
            o2: None,
        };
        apply_entry(&be, &entry).await.unwrap();
        assert_eq!(be.count("c", &json!({})).await.unwrap(), 0);
    }
}
