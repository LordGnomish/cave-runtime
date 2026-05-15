// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Append-only, compacted log of `MetadataRecord`s — the single
//! source of truth for cluster metadata in KRaft mode.
//!
//! Mirrors the in-memory half of
//! `org.apache.kafka.metadata.MetadataDelta` plus the compactor
//! from upstream `metadata/`. The on-disk persistence layer
//! (KIP-630 snapshots) is intentionally out of scope here — see
//! the module doc on the [`super`] module.

use std::collections::HashMap;
use std::sync::RwLock;

use super::epoch::ControllerEpoch;
use super::metadata::{ClusterMetadata, MetadataKey, MetadataRecord};

/// One committed entry — record + the log offset it landed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub offset: u64,
    pub record: MetadataRecord,
}

/// Append-only metadata log with by-key compaction. `&self`
/// methods + interior `RwLock` so the controller can share an
/// instance across threads (matches the upstream
/// `MetadataLoader` shape, which is also lock-protected).
pub struct MetadataLog {
    inner: RwLock<MetadataLogInner>,
}

struct MetadataLogInner {
    /// Next offset to assign.
    next_offset: u64,
    /// Live entries — compacted by `MetadataKey`. Most recent
    /// non-tombstone wins.
    by_key: HashMap<MetadataKey, LogEntry>,
    /// Append order — used for replication. Entries removed by
    /// compaction stay here until the snapshot threshold rolls
    /// them off; we expose `len()` to mark progress.
    appended: Vec<LogEntry>,
}

impl Default for MetadataLog {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataLog {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MetadataLogInner {
                next_offset: 0,
                by_key: HashMap::new(),
                appended: Vec::new(),
            }),
        }
    }

    /// Append `record` and return the offset it received.
    /// Caller must already be the elected leader at the
    /// record's `epoch` — the log doesn't re-validate that.
    pub fn append(&self, record: MetadataRecord) -> LogEntry {
        let mut g = self.inner.write().expect("poisoned");
        let offset = g.next_offset;
        g.next_offset += 1;
        let entry = LogEntry { offset, record };
        let key = entry.record.key();
        if entry.record.is_tombstone() {
            g.by_key.remove(&key);
        } else {
            g.by_key.insert(key, entry.clone());
        }
        g.appended.push(entry.clone());
        entry
    }

    /// Append all records in order. Convenience for batch
    /// commits (e.g. CreateTopic emits one TopicRecord + N
    /// PartitionRecords atomically).
    pub fn append_batch(&self, records: Vec<MetadataRecord>) -> Vec<LogEntry> {
        records.into_iter().map(|r| self.append(r)).collect()
    }

    /// Current high-water mark (next offset that will be
    /// assigned).
    pub fn high_water_mark(&self) -> u64 {
        self.inner.read().expect("poisoned").next_offset
    }

    /// Number of entries appended (incl. tombstones). Distinct
    /// from `live_keys` because tombstones bump the count.
    pub fn appended_count(&self) -> usize {
        self.inner.read().expect("poisoned").appended.len()
    }

    /// Number of live keys after compaction.
    pub fn live_keys(&self) -> usize {
        self.inner.read().expect("poisoned").by_key.len()
    }

    /// Build a `ClusterMetadata` snapshot by folding every live
    /// (post-compaction) record. Allocates — call from
    /// metadata-serve paths, not hot ones.
    pub fn snapshot(&self) -> ClusterMetadata {
        let g = self.inner.read().expect("poisoned");
        let mut snap = ClusterMetadata::default();
        // Collect entries sorted by offset for deterministic
        // application order (the controller's correctness
        // doesn't depend on this, but tests appreciate it).
        let mut entries: Vec<&LogEntry> = g.by_key.values().collect();
        entries.sort_by_key(|e| e.offset);
        for e in entries {
            snap.apply(&e.record);
        }
        snap
    }

    /// Highest epoch observed in any live record. Used by the
    /// controller to reject stale appends.
    pub fn last_epoch(&self) -> ControllerEpoch {
        let g = self.inner.read().expect("poisoned");
        g.by_key
            .values()
            .map(|e| e.record.epoch())
            .max()
            .unwrap_or(ControllerEpoch::INITIAL)
    }
}

#[cfg(test)]
mod tests {
    use super::super::metadata::{PartitionRecord, TopicRecord};
    use super::*;

    fn topic_record(name: &str) -> MetadataRecord {
        MetadataRecord::Topic {
            epoch: ControllerEpoch(1),
            record: TopicRecord {
                name: name.into(),
                topic_id: uuid::Uuid::new_v4(),
                partition_count: 1,
                replication_factor: 1,
            },
        }
    }

    fn partition_record(t: &str, p: i32, leader: i32) -> MetadataRecord {
        MetadataRecord::Partition {
            epoch: ControllerEpoch(1),
            record: PartitionRecord {
                topic: t.into(),
                partition_id: p,
                leader,
                isr: vec![leader],
                replicas: vec![leader],
                leader_epoch: 0,
            },
        }
    }

    #[test]
    fn append_assigns_monotonic_offsets() {
        let log = MetadataLog::new();
        let e0 = log.append(topic_record("a"));
        let e1 = log.append(topic_record("b"));
        let e2 = log.append(topic_record("c"));
        assert_eq!(e0.offset, 0);
        assert_eq!(e1.offset, 1);
        assert_eq!(e2.offset, 2);
        assert_eq!(log.high_water_mark(), 3);
    }

    #[test]
    fn append_batch_atomic_ordering() {
        let log = MetadataLog::new();
        let recs = vec![
            topic_record("orders"),
            partition_record("orders", 0, 1),
            partition_record("orders", 1, 2),
        ];
        let entries = log.append_batch(recs);
        assert_eq!(entries.len(), 3);
        let offs: Vec<u64> = entries.iter().map(|e| e.offset).collect();
        assert_eq!(offs, vec![0, 1, 2]);
    }

    #[test]
    fn compaction_drops_predecessor_for_same_key() {
        let log = MetadataLog::new();
        let v1 = MetadataRecord::Topic {
            epoch: ControllerEpoch(1),
            record: TopicRecord {
                name: "orders".into(),
                topic_id: uuid::Uuid::new_v4(),
                partition_count: 1,
                replication_factor: 1,
            },
        };
        let v2 = MetadataRecord::Topic {
            epoch: ControllerEpoch(1),
            record: TopicRecord {
                name: "orders".into(),
                topic_id: uuid::Uuid::new_v4(),
                partition_count: 5,
                replication_factor: 3,
            },
        };
        log.append(v1);
        log.append(v2.clone());
        let snap = log.snapshot();
        assert_eq!(snap.topics["orders"].partition_count, 5);
        assert_eq!(snap.topics["orders"].replication_factor, 3);
        assert_eq!(log.live_keys(), 1);
        assert_eq!(log.appended_count(), 2);
    }

    #[test]
    fn tombstone_removes_live_entry() {
        let log = MetadataLog::new();
        log.append(topic_record("orders"));
        log.append(MetadataRecord::TopicRemoved {
            epoch: ControllerEpoch(1),
            name: "orders".into(),
        });
        let snap = log.snapshot();
        assert!(!snap.topics.contains_key("orders"));
        assert_eq!(log.live_keys(), 0);
    }

    #[test]
    fn snapshot_reflects_post_compaction_state() {
        let log = MetadataLog::new();
        log.append(topic_record("orders"));
        log.append(partition_record("orders", 0, 1));
        log.append(partition_record("orders", 1, 1));
        // Re-lead partition 1 onto broker 2.
        log.append(partition_record("orders", 1, 2));
        let snap = log.snapshot();
        assert_eq!(snap.partitions[&("orders".into(), 0)].leader, 1);
        assert_eq!(snap.partitions[&("orders".into(), 1)].leader, 2);
    }

    #[test]
    fn last_epoch_reflects_highest_record() {
        let log = MetadataLog::new();
        log.append(MetadataRecord::Topic {
            epoch: ControllerEpoch(2),
            record: TopicRecord {
                name: "a".into(),
                topic_id: uuid::Uuid::new_v4(),
                partition_count: 1,
                replication_factor: 1,
            },
        });
        log.append(MetadataRecord::Topic {
            epoch: ControllerEpoch(5),
            record: TopicRecord {
                name: "b".into(),
                topic_id: uuid::Uuid::new_v4(),
                partition_count: 1,
                replication_factor: 1,
            },
        });
        assert_eq!(log.last_epoch(), ControllerEpoch(5));
    }
}
