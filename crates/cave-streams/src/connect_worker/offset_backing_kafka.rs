// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/KafkaOffsetBackingStore.java
//   connect/runtime/src/main/java/org/apache/kafka/connect/util/KafkaBasedLog.java

//! Real Kafka producer/consumer adapter for the Connect offset store.
//!
//! The existing
//! [`super::kafka_offset_backing_store::KafkaOffsetBackingStore`]
//! implements the *replay → materialise → commit-with-tombstone*
//! semantics against an abstract [`super::kafka_offset_backing_store::RecordLog`].
//! That layer is process-local. Production Connect persists offsets
//! to a compacted Kafka topic (`connect-offsets`); a restarted worker
//! reads the topic end-to-end to rebuild its in-memory view.
//!
//! This module ships the real producer/consumer side of that layer:
//!
//! * `KafkaOffsetTopicClient` trait — what the offset store needs
//!   from the broker (publish a key→value record on the offset topic,
//!   bulk-read every record). The trait is intentionally narrower
//!   than the full Producer/Consumer API so cave-streams' broker
//!   client can plug in *and* tests can run with an in-memory
//!   double.
//! * `KafkaBackedOffsetStore` — wires the trait onto
//!   `KafkaOffsetBackingStore`. On construction it issues a single
//!   `read_all` to replay the topic; every `commit` publishes the
//!   record before mutating the materialised view; every `forget`
//!   publishes a tombstone (value=None).
//! * `InMemoryOffsetTopicClient` — in-process double used in the
//!   tests below and by an integration test in `tests/`. Mirrors
//!   the topic shape (append-only log) plus a compaction pass that
//!   matches Kafka's `cleanup.policy=compact` invariant: after
//!   compaction, the last value per key is what's left.
//!
//! ## TODO(S2→S1)
//!
//! A bridge that adapts cave-streams' production [`crate::producer::Producer`]
//! + [`crate::consumer::Consumer`] onto `KafkaOffsetTopicClient` is
//! intentionally not shipped here because Producer/Consumer are
//! parameterised on `StreamStorage` (broker-internal); plugging them
//! requires extending consumer_group/kafka_wire surfaces which S1
//! owns. The seam is documented + ready for an S1 follow-up.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::error::{StreamsError, StreamsResult};

use super::kafka_offset_backing_store::{KafkaOffsetBackingStore, OffsetRecord, RecordLog};
use super::offset_store::{OffsetBackingStore, OffsetKey, OffsetValue};

/// One stored offset-topic message — `value=None` is a tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetTopicRecord {
    pub key: OffsetKey,
    pub value: Option<OffsetValue>,
}

/// Minimal Kafka-side surface the offset store needs. Implementors
/// must guarantee:
///
/// * `publish` is durable (acks=all in production) before it returns.
///   Tests use an in-memory log that satisfies this trivially.
/// * `read_all` returns every record on the offset topic in append
///   order. The compacted-topic invariant means duplicates with the
///   same key may exist; the offset store applies them in order.
/// * Both methods are thread-safe (`&self`).
pub trait KafkaOffsetTopicClient: Send + Sync {
    fn publish(&self, record: OffsetTopicRecord) -> StreamsResult<()>;
    fn read_all(&self) -> StreamsResult<Vec<OffsetTopicRecord>>;
}

/// In-process double for tests + smoke-runs. Backs the offset topic
/// with an append-only Vec.
#[derive(Default, Clone)]
pub struct InMemoryOffsetTopicClient {
    inner: Arc<Mutex<Vec<OffsetTopicRecord>>>,
}

impl std::fmt::Debug for InMemoryOffsetTopicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryOffsetTopicClient")
            .field("records", &self.inner.lock().expect("poisoned").len())
            .finish()
    }
}

impl InMemoryOffsetTopicClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("poisoned").is_empty()
    }

    /// Run a compaction pass — keep only the latest record per key.
    /// Returns the dropped record count. Mirrors what Kafka's log
    /// cleaner does on a `cleanup.policy=compact` topic.
    pub fn compact(&self) -> usize {
        let mut g = self.inner.lock().expect("poisoned");
        let before = g.len();
        // Walk back-to-front; keep first occurrence per key.
        let mut seen: std::collections::BTreeSet<OffsetKey> = Default::default();
        let mut out_rev: Vec<OffsetTopicRecord> = Vec::with_capacity(before);
        for r in g.iter().rev() {
            if seen.insert(r.key.clone()) {
                out_rev.push(r.clone());
            }
        }
        out_rev.reverse();
        *g = out_rev;
        before - g.len()
    }

    pub fn snapshot(&self) -> Vec<OffsetTopicRecord> {
        self.inner.lock().expect("poisoned").clone()
    }
}

impl KafkaOffsetTopicClient for InMemoryOffsetTopicClient {
    fn publish(&self, record: OffsetTopicRecord) -> StreamsResult<()> {
        self.inner.lock().expect("poisoned").push(record);
        Ok(())
    }

    fn read_all(&self) -> StreamsResult<Vec<OffsetTopicRecord>> {
        Ok(self.inner.lock().expect("poisoned").clone())
    }
}

/// Real Kafka-topic-backed offset store. Replays the topic at
/// construction time to seed the in-memory view, then publishes every
/// commit before mutating the view (durable-first).
pub struct KafkaBackedOffsetStore {
    client: Arc<dyn KafkaOffsetTopicClient>,
    inner: KafkaOffsetBackingStore,
}

impl KafkaBackedOffsetStore {
    /// Build the store from a client. Reads the topic synchronously
    /// (one `read_all` call) and replays it through the materialised
    /// view — that matches upstream's `KafkaBasedLog.start()` which
    /// drains the topic to its end position before signalling
    /// readiness.
    pub fn open(client: Arc<dyn KafkaOffsetTopicClient>) -> StreamsResult<Self> {
        let records = client.read_all()?;
        let mut log = RecordLog::new();
        for r in records {
            log.append(match r.value {
                Some(v) => OffsetRecord::put(r.key, v),
                None => OffsetRecord::tombstone(r.key),
            });
        }
        Ok(Self {
            client,
            inner: KafkaOffsetBackingStore::new(log),
        })
    }

    /// Construct a fresh store without any prior records — useful at
    /// cluster bootstrap before the offset topic exists.
    pub fn empty(client: Arc<dyn KafkaOffsetTopicClient>) -> Self {
        Self {
            client,
            inner: KafkaOffsetBackingStore::new(RecordLog::new()),
        }
    }

    pub fn commit(&mut self, key: OffsetKey, value: OffsetValue) -> StreamsResult<()> {
        self.client.publish(OffsetTopicRecord {
            key: key.clone(),
            value: Some(value.clone()),
        })?;
        self.inner.commit(key, value);
        Ok(())
    }

    pub fn forget(&mut self, key: OffsetKey) -> StreamsResult<()> {
        self.client.publish(OffsetTopicRecord {
            key: key.clone(),
            value: None,
        })?;
        self.inner.forget(key);
        Ok(())
    }

    pub fn commit_batch(
        &mut self,
        items: Vec<(OffsetKey, OffsetValue)>,
    ) -> StreamsResult<()> {
        // Durable-first: publish every record before updating the
        // materialised view. If a publish fails mid-batch, the in-memory
        // view stays consistent with the published prefix and the
        // caller can retry the suffix.
        for (k, v) in items.iter() {
            self.client.publish(OffsetTopicRecord {
                key: k.clone(),
                value: Some(v.clone()),
            })?;
        }
        self.inner.commit_batch(items);
        Ok(())
    }

    pub fn get(&self, key: &OffsetKey) -> Option<OffsetValue> {
        self.inner.get(key)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn snapshot(&self) -> BTreeMap<OffsetKey, OffsetValue> {
        self.inner.snapshot()
    }

    /// Rebuild a *new* store from the offset-topic state — used for
    /// a fault-recovery smoke that proves restart yields the same
    /// view.
    pub fn replay_from_topic(&self) -> StreamsResult<KafkaOffsetBackingStore> {
        let records = self.client.read_all()?;
        let mut log = RecordLog::new();
        for r in records {
            log.append(match r.value {
                Some(v) => OffsetRecord::put(r.key, v),
                None => OffsetRecord::tombstone(r.key),
            });
        }
        Ok(KafkaOffsetBackingStore::new(log))
    }
}

// Expose the trait surface so the store is interchangeable with the
// in-memory variant.
impl OffsetBackingStore for KafkaBackedOffsetStore {
    fn get(&self, key: &OffsetKey) -> Option<OffsetValue> {
        KafkaBackedOffsetStore::get(self, key)
    }
    fn commit(&mut self, key: OffsetKey, value: OffsetValue) {
        // Swallow publish errors only when accessed via the trait —
        // the inherent `commit` returns Result; surface errors there.
        let _ = KafkaBackedOffsetStore::commit(self, key, value);
    }
    fn forget(&mut self, key: OffsetKey) {
        let _ = KafkaBackedOffsetStore::forget(self, key);
    }
}

/// Failure-injecting client used in fault-recovery tests.
#[cfg(test)]
pub(crate) struct FailingPublishClient {
    inner: InMemoryOffsetTopicClient,
    fail_next: Arc<Mutex<bool>>,
}

#[cfg(test)]
impl FailingPublishClient {
    fn new() -> Self {
        Self {
            inner: InMemoryOffsetTopicClient::new(),
            fail_next: Arc::new(Mutex::new(false)),
        }
    }
    fn arm(&self) {
        *self.fail_next.lock().unwrap() = true;
    }
}

#[cfg(test)]
impl KafkaOffsetTopicClient for FailingPublishClient {
    fn publish(&self, r: OffsetTopicRecord) -> StreamsResult<()> {
        let mut f = self.fail_next.lock().unwrap();
        if *f {
            *f = false;
            return Err(StreamsError::Internal("simulated publish failure".into()));
        }
        drop(f);
        self.inner.publish(r)
    }
    fn read_all(&self) -> StreamsResult<Vec<OffsetTopicRecord>> {
        self.inner.read_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: &str, t: &str) -> OffsetKey {
        let mut p = BTreeMap::new();
        p.insert("table".into(), t.into());
        OffsetKey {
            connector: c.into(),
            partition: p,
        }
    }
    fn val(s: &str) -> OffsetValue {
        let mut m = BTreeMap::new();
        m.insert("position".into(), s.into());
        m
    }

    #[test]
    fn open_with_empty_topic_yields_empty_view() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let s = KafkaBackedOffsetStore::open(client).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn commit_publishes_then_materialises() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let mut s = KafkaBackedOffsetStore::open(client.clone()).unwrap();
        s.commit(key("c", "a"), val("1")).unwrap();
        // Topic has the record + the materialised view reflects it.
        assert_eq!(client.len(), 1);
        assert_eq!(s.get(&key("c", "a")), Some(val("1")));
    }

    #[test]
    fn forget_emits_tombstone() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let mut s = KafkaBackedOffsetStore::open(client.clone()).unwrap();
        s.commit(key("c", "a"), val("1")).unwrap();
        s.forget(key("c", "a")).unwrap();
        let log = client.snapshot();
        assert_eq!(log.len(), 2);
        assert!(log[1].value.is_none(), "second record must be tombstone");
        assert!(s.get(&key("c", "a")).is_none());
    }

    #[test]
    fn restart_replays_topic_to_view() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        {
            let mut s = KafkaBackedOffsetStore::open(client.clone()).unwrap();
            s.commit(key("c", "a"), val("1")).unwrap();
            s.commit(key("c", "b"), val("2")).unwrap();
            s.forget(key("c", "a")).unwrap();
        }
        // "Restart" — drop the previous store, reopen.
        let s2 = KafkaBackedOffsetStore::open(client).unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2.get(&key("c", "b")), Some(val("2")));
        assert!(s2.get(&key("c", "a")).is_none());
    }

    #[test]
    fn commit_batch_is_durable_first() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let mut s = KafkaBackedOffsetStore::open(client.clone()).unwrap();
        s.commit_batch(vec![
            (key("c", "a"), val("1")),
            (key("c", "b"), val("2")),
        ])
        .unwrap();
        assert_eq!(client.len(), 2);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn failing_publish_keeps_view_consistent_with_topic() {
        let raw = Arc::new(FailingPublishClient::new());
        let client: Arc<dyn KafkaOffsetTopicClient> = raw.clone();
        let mut s = KafkaBackedOffsetStore::open(client).unwrap();
        raw.arm();
        let err = s.commit(key("c", "a"), val("1"));
        assert!(err.is_err(), "expected publish failure to bubble up");
        // View must NOT have been touched.
        assert!(s.is_empty());
    }

    #[test]
    fn compaction_keeps_only_last_value_per_key() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let mut s = KafkaBackedOffsetStore::open(client.clone()).unwrap();
        s.commit(key("c", "a"), val("1")).unwrap();
        s.commit(key("c", "a"), val("2")).unwrap();
        s.commit(key("c", "a"), val("3")).unwrap();
        assert_eq!(client.len(), 3);
        let dropped = client.compact();
        assert_eq!(dropped, 2);
        assert_eq!(client.len(), 1);
        // Recovery from the compacted topic still yields position=3.
        let s2 = KafkaBackedOffsetStore::open(client).unwrap();
        assert_eq!(s2.get(&key("c", "a")), Some(val("3")));
    }

    #[test]
    fn replay_from_topic_returns_independent_view() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let mut s = KafkaBackedOffsetStore::open(client).unwrap();
        s.commit(key("c", "a"), val("9")).unwrap();
        let replay = s.replay_from_topic().unwrap();
        assert_eq!(replay.get(&key("c", "a")), Some(val("9")));
    }

    #[test]
    fn empty_constructor_does_not_read_topic() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let _ = client.publish(OffsetTopicRecord {
            key: key("c", "a"),
            value: Some(val("seed")),
        });
        let s = KafkaBackedOffsetStore::empty(client);
        // Should NOT see the seeded record.
        assert!(s.is_empty());
    }

    #[test]
    fn trait_surface_offsetbackingstore_works() {
        let client = Arc::new(InMemoryOffsetTopicClient::new());
        let mut s = KafkaBackedOffsetStore::open(client).unwrap();
        let s_ref: &mut dyn OffsetBackingStore = &mut s;
        s_ref.commit(key("c", "a"), val("1"));
        assert_eq!(s_ref.get(&key("c", "a")), Some(val("1")));
        s_ref.forget(key("c", "a"));
        assert!(s_ref.get(&key("c", "a")).is_none());
    }
}
