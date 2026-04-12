//! In-memory store for cave-streams state.

use crate::models::*;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct StreamsStore {
    pub streams: HashMap<Uuid, Stream>,
    pub subscriptions: HashMap<Uuid, Subscription>,
    pub messages: Vec<Message>,
    pub schemas: HashMap<Uuid, Schema>,
    pub connectors: HashMap<Uuid, Connector>,
    pub dlq: HashMap<Uuid, DeadLetterEntry>,
    pub storage_config: StorageTierConfig,
    pub metrics: HashMap<Uuid, StreamMetrics>,
    /// Deduplication window for exactly-once publish.
    pub dedup_ids: HashSet<Uuid>,
    /// Per-stream monotonic sequence counters.
    pub stream_sequences: HashMap<Uuid, u64>,
    /// Per-subject schema version counters.
    pub schema_versions: HashMap<String, u32>,
    /// Cumulative exactly-once dedup hits (platform-wide).
    pub dedup_hit_count: u64,
}

impl Default for StreamsStore {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            subscriptions: HashMap::new(),
            messages: Vec::new(),
            schemas: HashMap::new(),
            connectors: HashMap::new(),
            dlq: HashMap::new(),
            storage_config: StorageTierConfig::default(),
            metrics: HashMap::new(),
            dedup_ids: HashSet::new(),
            stream_sequences: HashMap::new(),
            schema_versions: HashMap::new(),
            dedup_hit_count: 0,
        }
    }
}

impl StreamsStore {
    /// Advance and return the next sequence number for a stream.
    pub fn next_sequence(&mut self, stream_id: Uuid) -> u64 {
        let seq = self.stream_sequences.entry(stream_id).or_insert(0);
        *seq += 1;
        *seq
    }

    /// Advance and return the next schema version for a subject.
    pub fn next_schema_version(&mut self, subject: &str) -> u32 {
        let ver = self
            .schema_versions
            .entry(subject.to_string())
            .or_insert(0);
        *ver += 1;
        *ver
    }

    /// FNV-1a fingerprint — no extra dependency needed.
    pub fn fingerprint(s: &str) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in s.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    /// Check deduplication window; returns true if the ID is a duplicate.
    pub fn is_duplicate(&mut self, dedup_id: Uuid) -> bool {
        if self.dedup_ids.contains(&dedup_id) {
            self.dedup_hit_count += 1;
            true
        } else {
            self.dedup_ids.insert(dedup_id);
            false
        }
    }

    /// Update the subscriber count on a stream, if it exists.
    pub fn refresh_subscriber_count(&mut self, stream_id: Uuid) {
        let count = self
            .subscriptions
            .values()
            .filter(|s| s.stream_id == stream_id)
            .count() as u32;
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.subscriber_count = count;
        }
    }
}
