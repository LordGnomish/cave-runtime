// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/src/main/java/org/apache/pulsar/compaction/Compactor.java
//   pulsar-broker/src/main/java/org/apache/pulsar/compaction/TwoPhaseCompactor.java
//   pulsar-broker/src/main/java/org/apache/pulsar/compaction/CompactedTopic.java

//! Pulsar topic compaction — keep only the latest message per key.
//!
//! The compactor reads the topic backlog (one entry per (key, value)
//! pair plus tombstones), folds duplicates so only the *latest* value
//! per key survives, and writes the result back to the `__compaction`
//! ledger.  A Reader subscription with `read_compacted=true` then
//! serves the folded view; live producers continue appending to the
//! original topic.
//!
//! Differences vs upstream `TwoPhaseCompactor`:
//! - Single-pass in-process — Pulsar's two phases (compute survivors
//!   then re-publish) are merged because we operate over an in-memory
//!   backlog.  The phase distinction is a durability concern, not a
//!   semantic one.
//! - No ledger pointer commit to ZooKeeper — owner of the compacted
//!   ledger id lives in the `CompactionTopic` struct only.

use crate::error::StreamsResult;
use std::collections::HashMap;
use std::sync::Mutex;

/// A single record in the backlog, keyed by an arbitrary byte string.
/// `value = None` is a tombstone (compaction drops the key entirely).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionRecord {
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>,
}

impl CompactionRecord {
    pub fn put(key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            key: key.into(),
            value: Some(value.into()),
        }
    }

    pub fn tombstone(key: impl Into<Vec<u8>>) -> Self {
        Self {
            key: key.into(),
            value: None,
        }
    }
}

/// Compactor — folds a sequence of records into a per-key snapshot.
pub struct TopicCompactor;

impl TopicCompactor {
    /// Compact `records` (presented in append order — oldest first).
    /// Returns the surviving records sorted by key (Pulsar emits the
    /// compacted ledger in key order).
    pub fn compact(records: &[CompactionRecord]) -> Vec<CompactionRecord> {
        let mut latest: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();
        for r in records {
            latest.insert(r.key.clone(), r.value.clone());
        }
        let mut out: Vec<CompactionRecord> = latest
            .into_iter()
            .filter_map(|(k, v)| {
                // Tombstones drop the key from the snapshot.
                v.map(|val| CompactionRecord {
                    key: k,
                    value: Some(val),
                })
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }
}

/// Topic + its compaction state.  Producers append to `backlog`;
/// `run_compaction` snapshots that backlog and produces a fresh
/// compacted view kept around for `Reader.read_compacted=true`.
pub struct CompactionTopic {
    pub name: String,
    inner: Mutex<CompactionInner>,
}

#[derive(Debug, Default)]
struct CompactionInner {
    backlog: Vec<CompactionRecord>,
    compacted: Vec<CompactionRecord>,
    /// Monotonic id of the latest compaction ledger.
    compacted_ledger_id: Option<u64>,
}

impl CompactionTopic {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inner: Mutex::new(CompactionInner::default()),
        }
    }

    /// Append a producer-side record to the topic backlog.
    pub fn append(&self, record: CompactionRecord) {
        let mut inner = self.inner.lock().unwrap();
        inner.backlog.push(record);
    }

    /// Run the compactor.  Returns the new compacted ledger id.
    pub fn run_compaction(&self) -> StreamsResult<u64> {
        let mut inner = self.inner.lock().unwrap();
        let compacted = TopicCompactor::compact(&inner.backlog);
        let new_id = inner.compacted_ledger_id.unwrap_or(0) + 1;
        inner.compacted_ledger_id = Some(new_id);
        inner.compacted = compacted;
        Ok(new_id)
    }

    /// Read the full backlog (Reader without `read_compacted`).
    pub fn read_backlog(&self) -> Vec<CompactionRecord> {
        self.inner.lock().unwrap().backlog.clone()
    }

    /// Read the compacted view (Reader with `read_compacted=true`).
    pub fn read_compacted(&self) -> Vec<CompactionRecord> {
        self.inner.lock().unwrap().compacted.clone()
    }

    pub fn compacted_ledger_id(&self) -> Option<u64> {
        self.inner.lock().unwrap().compacted_ledger_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compactor_keeps_only_latest_value_per_key() {
        // cite: pulsar 4.2.0 Compactor.compactedRecord keeps last value per key
        // ensemble = cm-001
        let records = vec![
            CompactionRecord::put(b"a".to_vec(), b"v1"),
            CompactionRecord::put(b"a".to_vec(), b"v2"),
            CompactionRecord::put(b"a".to_vec(), b"v3"), // wins
        ];
        let out = TopicCompactor::compact(&records);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].value.as_deref(), Some(b"v3".as_slice()));
    }

    #[test]
    fn test_compactor_tombstone_drops_key() {
        // cite: pulsar 4.2.0 null-value records as tombstones
        // ensemble = cm-002
        let records = vec![
            CompactionRecord::put(b"k".to_vec(), b"v"),
            CompactionRecord::tombstone(b"k".to_vec()),
        ];
        let out = TopicCompactor::compact(&records);
        assert!(out.is_empty());
    }

    #[test]
    fn test_compactor_emits_sorted_by_key() {
        // cite: pulsar 4.2.0 compacted ledger is key-sorted
        // ensemble = cm-003
        let records = vec![
            CompactionRecord::put(b"z".to_vec(), b"vz"),
            CompactionRecord::put(b"a".to_vec(), b"va"),
            CompactionRecord::put(b"m".to_vec(), b"vm"),
        ];
        let out = TopicCompactor::compact(&records);
        let keys: Vec<&[u8]> = out.iter().map(|r| r.key.as_slice()).collect();
        assert_eq!(keys, vec![&b"a"[..], &b"m"[..], &b"z"[..]]);
    }

    #[test]
    fn test_compaction_topic_initial_compacted_ledger_id_is_one() {
        // cite: pulsar 4.2.0 first compaction produces a fresh ledger id
        // ensemble = cm-004
        let t = CompactionTopic::new("persistent://public/default/t");
        t.append(CompactionRecord::put(b"k".to_vec(), b"v"));
        let id = t.run_compaction().unwrap();
        assert_eq!(id, 1);
        assert_eq!(t.compacted_ledger_id(), Some(1));
    }

    #[test]
    fn test_compaction_topic_subsequent_compaction_bumps_ledger_id() {
        // cite: pulsar 4.2.0 each compaction emits a new ledger
        // ensemble = cm-005
        let t = CompactionTopic::new("t");
        t.append(CompactionRecord::put(b"k".to_vec(), b"v"));
        t.run_compaction().unwrap();
        t.append(CompactionRecord::put(b"k".to_vec(), b"v2"));
        let id2 = t.run_compaction().unwrap();
        assert_eq!(id2, 2);
    }

    #[test]
    fn test_compaction_topic_read_compacted_returns_folded_view() {
        // cite: pulsar 4.2.0 Reader.read_compacted=true serves snapshot
        // ensemble = cm-006
        let t = CompactionTopic::new("t");
        t.append(CompactionRecord::put(b"a".to_vec(), b"1"));
        t.append(CompactionRecord::put(b"b".to_vec(), b"2"));
        t.append(CompactionRecord::put(b"a".to_vec(), b"3")); // overrides
        t.run_compaction().unwrap();
        let view = t.read_compacted();
        assert_eq!(view.len(), 2);
        let a = view.iter().find(|r| r.key == b"a").unwrap();
        assert_eq!(a.value.as_deref(), Some(b"3".as_slice()));
    }

    #[test]
    fn test_compaction_topic_backlog_independent_of_compaction() {
        // cite: pulsar 4.2.0 backlog kept until retention drops it
        // ensemble = cm-007
        let t = CompactionTopic::new("t");
        t.append(CompactionRecord::put(b"k".to_vec(), b"v1"));
        t.append(CompactionRecord::put(b"k".to_vec(), b"v2"));
        t.run_compaction().unwrap();
        // Backlog still has BOTH appends.
        assert_eq!(t.read_backlog().len(), 2);
        // Compacted view only the latest.
        assert_eq!(t.read_compacted().len(), 1);
    }

    #[test]
    fn test_compactor_empty_input_yields_empty_output() {
        // cite: pulsar 4.2.0 empty topic compacts to empty
        // ensemble = cm-008
        let out = TopicCompactor::compact(&[]);
        assert!(out.is_empty());
    }
}
