// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kafka-backed offset store. Mirrors upstream
//! `connect/runtime/KafkaOffsetBackingStore.java`.
//!
//! In production Kafka Connect, the source-connector offset
//! store sits on top of a compacted Kafka topic (`connect-offsets`):
//! each commit appends a record, the store replays the topic at
//! start-up to rebuild its in-memory `(connector, partition) →
//! offset` map, and a tombstone (value=None) deletes a key on
//! the next compaction pass.
//!
//! This module ships the *replay + commit* semantics against a
//! pluggable [`RecordLog`] backend. The real Kafka producer/
//! consumer wiring is tracked separately — once the broker side
//! has a stable internal producer client, the same trait
//! ([`OffsetBackingStore`] in [`super::offset_store`]) lets the
//! Kafka-backed adapter swap in.

use std::collections::BTreeMap;

use super::offset_store::{OffsetBackingStore, OffsetKey, OffsetValue};

/// One entry on the offsets log. `Put` carries the value, `Tombstone`
/// signals a key removal (mirrors a Kafka record with value=null).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffsetRecord {
    Put { key: OffsetKey, value: OffsetValue },
    Tombstone { key: OffsetKey },
}

impl OffsetRecord {
    pub fn put(key: OffsetKey, value: OffsetValue) -> Self {
        Self::Put { key, value }
    }
    pub fn tombstone(key: OffsetKey) -> Self {
        Self::Tombstone { key }
    }
    pub fn key(&self) -> &OffsetKey {
        match self {
            Self::Put { key, .. } | Self::Tombstone { key } => key,
        }
    }
}

/// Append-only replay log. In production this is a compacted
/// Kafka topic; in tests + the in-memory plug-in below it is a
/// `Vec`.
#[derive(Debug, Default, Clone)]
pub struct RecordLog {
    records: Vec<OffsetRecord>,
}

impl RecordLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, record: OffsetRecord) {
        self.records.push(record);
    }

    pub fn iter(&self) -> impl Iterator<Item = &OffsetRecord> + '_ {
        self.records.iter()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Kafka-backed offset store. Holds the materialised view +
/// the underlying append-only log.
pub struct KafkaOffsetBackingStore {
    materialised: BTreeMap<OffsetKey, OffsetValue>,
    log: RecordLog,
}

impl KafkaOffsetBackingStore {
    /// Build a store by replaying `log` into the in-memory map.
    /// Mirrors upstream `KafkaOffsetBackingStore.start()` which
    /// calls `consumer.poll()` in a loop until the end of the
    /// `connect-offsets` topic is reached.
    pub fn new(log: RecordLog) -> Self {
        let mut me = Self {
            materialised: BTreeMap::new(),
            log,
        };
        me.replay();
        me
    }

    fn replay(&mut self) {
        // Iterate in append order; later records win.
        for r in self.log.records.iter() {
            match r {
                OffsetRecord::Put { key, value } => {
                    self.materialised.insert(key.clone(), value.clone());
                }
                OffsetRecord::Tombstone { key } => {
                    self.materialised.remove(key);
                }
            }
        }
    }

    /// Append a commit record + update the materialised view.
    pub fn commit(&mut self, key: OffsetKey, value: OffsetValue) {
        self.log.append(OffsetRecord::Put {
            key: key.clone(),
            value: value.clone(),
        });
        self.materialised.insert(key, value);
    }

    /// Append a tombstone + drop the entry from the view.
    pub fn forget(&mut self, key: OffsetKey) {
        self.log.append(OffsetRecord::Tombstone { key: key.clone() });
        self.materialised.remove(&key);
    }

    /// Append a batch atomically — every record is in the log
    /// before the materialised view is observed by readers.
    pub fn commit_batch(&mut self, items: impl IntoIterator<Item = (OffsetKey, OffsetValue)>) {
        let collected: Vec<_> = items.into_iter().collect();
        for (k, v) in &collected {
            self.log.append(OffsetRecord::Put {
                key: k.clone(),
                value: v.clone(),
            });
        }
        for (k, v) in collected {
            self.materialised.insert(k, v);
        }
    }

    /// Drop every offset belonging to `connector` (writes a
    /// tombstone per key).
    pub fn forget_connector(&mut self, connector: &str) {
        let keys: Vec<_> = self
            .materialised
            .keys()
            .filter(|k| k.connector == connector)
            .cloned()
            .collect();
        for k in keys {
            self.forget(k);
        }
    }

    pub fn get(&self, key: &OffsetKey) -> Option<OffsetValue> {
        self.materialised.get(key).cloned()
    }

    pub fn get_many(
        &self,
        connector: &str,
        partitions: &[BTreeMap<String, String>],
    ) -> BTreeMap<BTreeMap<String, String>, OffsetValue> {
        let mut out = BTreeMap::new();
        for p in partitions {
            let key = OffsetKey {
                connector: connector.into(),
                partition: p.clone(),
            };
            if let Some(v) = self.materialised.get(&key) {
                out.insert(p.clone(), v.clone());
            }
        }
        out
    }

    pub fn snapshot(&self) -> BTreeMap<OffsetKey, OffsetValue> {
        self.materialised.clone()
    }

    pub fn snapshot_for(&self, connector: &str) -> Vec<(BTreeMap<String, String>, OffsetValue)> {
        self.materialised
            .iter()
            .filter(|(k, _)| k.connector == connector)
            .map(|(k, v)| (k.partition.clone(), v.clone()))
            .collect()
    }

    pub fn log_len(&self) -> usize {
        self.log.len()
    }

    pub fn len(&self) -> usize {
        self.materialised.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materialised.is_empty()
    }
}

impl OffsetBackingStore for KafkaOffsetBackingStore {
    fn get(&self, key: &OffsetKey) -> Option<OffsetValue> {
        KafkaOffsetBackingStore::get(self, key)
    }
    fn commit(&mut self, key: OffsetKey, value: OffsetValue) {
        KafkaOffsetBackingStore::commit(self, key, value)
    }
    fn forget(&mut self, key: OffsetKey) {
        KafkaOffsetBackingStore::forget(self, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: &str, t: &str) -> OffsetKey {
        let mut m = BTreeMap::new();
        m.insert("table".into(), t.into());
        OffsetKey {
            connector: c.into(),
            partition: m,
        }
    }
    fn val(s: &str) -> OffsetValue {
        let mut m = BTreeMap::new();
        m.insert("position".into(), s.into());
        m
    }

    #[test]
    fn replay_of_empty_log_yields_empty_view() {
        let s = KafkaOffsetBackingStore::new(RecordLog::new());
        assert!(s.is_empty());
        assert_eq!(s.log_len(), 0);
    }

    #[test]
    fn replay_applies_records_in_order() {
        let mut log = RecordLog::new();
        log.append(OffsetRecord::put(key("c", "a"), val("1")));
        log.append(OffsetRecord::put(key("c", "a"), val("2")));
        let s = KafkaOffsetBackingStore::new(log);
        assert_eq!(s.get(&key("c", "a")), Some(val("2")));
    }

    #[test]
    fn replay_honors_tombstones() {
        let mut log = RecordLog::new();
        log.append(OffsetRecord::put(key("c", "a"), val("1")));
        log.append(OffsetRecord::tombstone(key("c", "a")));
        let s = KafkaOffsetBackingStore::new(log);
        assert!(s.get(&key("c", "a")).is_none());
    }

    #[test]
    fn commit_appends_record_to_log() {
        let mut s = KafkaOffsetBackingStore::new(RecordLog::new());
        s.commit(key("c", "a"), val("1"));
        assert_eq!(s.log_len(), 1);
        assert_eq!(s.get(&key("c", "a")), Some(val("1")));
    }

    #[test]
    fn commit_overwrites_replay() {
        let mut log = RecordLog::new();
        log.append(OffsetRecord::put(key("c", "a"), val("1")));
        let mut s = KafkaOffsetBackingStore::new(log);
        s.commit(key("c", "a"), val("9"));
        assert_eq!(s.get(&key("c", "a")), Some(val("9")));
        assert_eq!(s.log_len(), 2);
    }

    #[test]
    fn commit_batch_is_atomic_in_view() {
        let mut s = KafkaOffsetBackingStore::new(RecordLog::new());
        s.commit_batch(vec![(key("c", "a"), val("1")), (key("c", "b"), val("2"))]);
        assert_eq!(s.len(), 2);
        assert_eq!(s.log_len(), 2);
    }

    #[test]
    fn forget_emits_tombstone() {
        let mut s = KafkaOffsetBackingStore::new(RecordLog::new());
        s.commit(key("c", "a"), val("1"));
        s.forget(key("c", "a"));
        assert!(s.get(&key("c", "a")).is_none());
        assert_eq!(s.log_len(), 2);
    }

    #[test]
    fn forget_connector_drops_only_its_keys() {
        let mut s = KafkaOffsetBackingStore::new(RecordLog::new());
        s.commit(key("a", "t"), val("1"));
        s.commit(key("b", "t"), val("2"));
        s.forget_connector("a");
        assert!(s.get(&key("a", "t")).is_none());
        assert!(s.get(&key("b", "t")).is_some());
    }

    #[test]
    fn snapshot_for_filters_by_connector() {
        let mut s = KafkaOffsetBackingStore::new(RecordLog::new());
        s.commit(key("jdbc", "a"), val("1"));
        s.commit(key("jdbc", "b"), val("2"));
        s.commit(key("other", "c"), val("3"));
        assert_eq!(s.snapshot_for("jdbc").len(), 2);
    }

    #[test]
    fn get_many_returns_partial_hits() {
        let mut s = KafkaOffsetBackingStore::new(RecordLog::new());
        let mut p_known = BTreeMap::new();
        p_known.insert("table".into(), "a".into());
        let mut p_miss = BTreeMap::new();
        p_miss.insert("table".into(), "nope".into());
        s.commit(
            OffsetKey {
                connector: "c".into(),
                partition: p_known.clone(),
            },
            val("1"),
        );
        let got = s.get_many("c", &[p_known.clone(), p_miss.clone()]);
        assert_eq!(got.len(), 1);
        assert!(got.contains_key(&p_known));
        assert!(!got.contains_key(&p_miss));
    }

    #[test]
    fn snapshot_after_replay_matches_materialised() {
        let mut log = RecordLog::new();
        log.append(OffsetRecord::put(key("c", "a"), val("1")));
        log.append(OffsetRecord::put(key("c", "b"), val("2")));
        log.append(OffsetRecord::tombstone(key("c", "a")));
        let s = KafkaOffsetBackingStore::new(log);
        assert_eq!(s.snapshot().len(), 1);
        assert!(s.snapshot().contains_key(&key("c", "b")));
    }
}
