// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Source-connector offset store. Mirrors
//! `org.apache.kafka.connect.storage.OffsetStorageReader/Writer`
//! from upstream — the durable map of `(connector,
//! source_partition) → source_offset` that lets a restarted
//! source task pick up where the previous instance left off.
//!
//! In production, Kafka Connect uses a compacted Kafka topic
//! (`connect-offsets`) for this. cave-streams ships an
//! in-memory implementation behind the same logical API; the
//! Kafka-backed store can swap in via the same trait once the
//! producer/consumer adapter lands (tracked, not in this batch).

use std::collections::BTreeMap;
use std::sync::RwLock;

/// Connector-scoped partition reference. Keyed by connector
/// name + the source-partition map the SourceTask supplied
/// when it emitted records.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OffsetKey {
    pub connector: String,
    pub partition: BTreeMap<String, String>,
}

/// What the source recorded as its position. Free-form per
/// connector (e.g. a database connector uses `{"offset":
/// "1234"}`; a file connector uses `{"file": "/x", "line":
/// "42"}`).
pub type OffsetValue = BTreeMap<String, String>;

/// Common surface between the in-memory [`OffsetStore`] and the
/// Kafka-backed
/// [`super::kafka_offset_backing_store::KafkaOffsetBackingStore`].
/// Mirrors upstream `OffsetBackingStore` (Java interface).
pub trait OffsetBackingStore {
    fn get(&self, key: &OffsetKey) -> Option<OffsetValue>;
    fn commit(&mut self, key: OffsetKey, value: OffsetValue);
    fn forget(&mut self, key: OffsetKey);
}

/// `&self` everywhere — interior mutability via `RwLock` so the
/// store can be `Arc`-shared across runtime tasks.
#[derive(Default)]
pub struct OffsetStore {
    inner: RwLock<BTreeMap<OffsetKey, OffsetValue>>,
}

impl OffsetBackingStore for OffsetStore {
    fn get(&self, key: &OffsetKey) -> Option<OffsetValue> {
        OffsetStore::get(self, key)
    }
    fn commit(&mut self, key: OffsetKey, value: OffsetValue) {
        OffsetStore::commit(self, key, value)
    }
    fn forget(&mut self, key: OffsetKey) {
        let mut g = self.inner.write().expect("poisoned");
        g.remove(&key);
    }
}

impl OffsetStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the offset previously committed for `key`.
    pub fn get(&self, key: &OffsetKey) -> Option<OffsetValue> {
        self.inner.read().expect("poisoned").get(key).cloned()
    }

    /// Bulk read — order-preserving since the underlying map is
    /// a `BTreeMap`. Used by a SourceTask at start-up to load
    /// every partition it owns.
    pub fn get_many(&self, connector: &str, partitions: &[BTreeMap<String, String>]) -> BTreeMap<BTreeMap<String, String>, OffsetValue> {
        let g = self.inner.read().expect("poisoned");
        let mut out = BTreeMap::new();
        for p in partitions {
            let key = OffsetKey {
                connector: connector.into(),
                partition: p.clone(),
            };
            if let Some(v) = g.get(&key) {
                out.insert(p.clone(), v.clone());
            }
        }
        out
    }

    /// Commit a single offset. Atomic — concurrent readers
    /// see either the previous value or the new value, never a
    /// partial mix.
    pub fn commit(&self, key: OffsetKey, value: OffsetValue) {
        let mut g = self.inner.write().expect("poisoned");
        g.insert(key, value);
    }

    /// Commit a batch — single lock hold so the entire batch
    /// is observed atomically.
    pub fn commit_batch(&self, items: impl IntoIterator<Item = (OffsetKey, OffsetValue)>) {
        let mut g = self.inner.write().expect("poisoned");
        for (k, v) in items {
            g.insert(k, v);
        }
    }

    /// Drop every offset that belongs to `connector`. Used
    /// when the connector is deleted via the REST API.
    pub fn forget_connector(&self, connector: &str) {
        let mut g = self.inner.write().expect("poisoned");
        g.retain(|k, _| k.connector != connector);
    }

    /// Total live offsets across all connectors.
    pub fn len(&self) -> usize {
        self.inner.read().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().expect("poisoned").is_empty()
    }

    /// Snapshot of all offsets for a connector — handy for
    /// admin UIs.
    pub fn snapshot_for(&self, connector: &str) -> Vec<(BTreeMap<String, String>, OffsetValue)> {
        let g = self.inner.read().expect("poisoned");
        g.iter()
            .filter(|(k, _)| k.connector == connector)
            .map(|(k, v)| (k.partition.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partition(table: &str) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("table".into(), table.into());
        m
    }

    fn offset(value: &str) -> OffsetValue {
        let mut m = BTreeMap::new();
        m.insert("position".into(), value.into());
        m
    }

    #[test]
    fn commit_then_get_round_trips() {
        let s = OffsetStore::new();
        let key = OffsetKey {
            connector: "jdbc-1".into(),
            partition: partition("orders"),
        };
        s.commit(key.clone(), offset("100"));
        assert_eq!(s.get(&key), Some(offset("100")));
    }

    #[test]
    fn get_missing_returns_none() {
        let s = OffsetStore::new();
        let key = OffsetKey {
            connector: "missing".into(),
            partition: partition("x"),
        };
        assert_eq!(s.get(&key), None);
    }

    #[test]
    fn commit_overwrites_previous_value() {
        let s = OffsetStore::new();
        let key = OffsetKey {
            connector: "c".into(),
            partition: partition("t"),
        };
        s.commit(key.clone(), offset("1"));
        s.commit(key.clone(), offset("2"));
        assert_eq!(s.get(&key), Some(offset("2")));
    }

    #[test]
    fn commit_batch_atomically_applies_all() {
        let s = OffsetStore::new();
        let items = vec![
            (
                OffsetKey {
                    connector: "c".into(),
                    partition: partition("a"),
                },
                offset("10"),
            ),
            (
                OffsetKey {
                    connector: "c".into(),
                    partition: partition("b"),
                },
                offset("20"),
            ),
        ];
        s.commit_batch(items);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn get_many_returns_only_known_partitions() {
        let s = OffsetStore::new();
        s.commit(
            OffsetKey {
                connector: "c".into(),
                partition: partition("a"),
            },
            offset("1"),
        );
        let got = s.get_many("c", &[partition("a"), partition("nope")]);
        assert_eq!(got.len(), 1);
        assert!(got.contains_key(&partition("a")));
    }

    #[test]
    fn forget_connector_drops_only_its_entries() {
        let s = OffsetStore::new();
        s.commit(
            OffsetKey {
                connector: "a".into(),
                partition: partition("t"),
            },
            offset("1"),
        );
        s.commit(
            OffsetKey {
                connector: "b".into(),
                partition: partition("t"),
            },
            offset("2"),
        );
        s.forget_connector("a");
        assert_eq!(s.len(), 1);
        assert_eq!(
            s.get(&OffsetKey {
                connector: "b".into(),
                partition: partition("t"),
            }),
            Some(offset("2"))
        );
    }

    #[test]
    fn snapshot_for_returns_per_connector() {
        let s = OffsetStore::new();
        for t in ["a", "b", "c"] {
            s.commit(
                OffsetKey {
                    connector: "jdbc".into(),
                    partition: partition(t),
                },
                offset("0"),
            );
        }
        s.commit(
            OffsetKey {
                connector: "other".into(),
                partition: partition("x"),
            },
            offset("99"),
        );
        let snap = s.snapshot_for("jdbc");
        assert_eq!(snap.len(), 3);
    }

    #[test]
    fn empty_store_is_empty() {
        let s = OffsetStore::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }
}
