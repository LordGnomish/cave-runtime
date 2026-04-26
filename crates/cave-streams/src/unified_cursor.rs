//! Unified cursor — Pulsar `ManagedCursor` ↔ Kafka consumer-group offset.
//!
//! Both protocols track *the highest message a subscriber has acked* on a
//! per-(topic, partition) basis; cave-streams unifies the two surfaces so
//! a Pulsar subscription and a Kafka consumer-group can share durable
//! positions when they front the same underlying log.
//!
//! Mirrors:
//!   * Pulsar 4.2.0 `pulsar-broker/.../ManagedCursorImpl.java`
//!     (`markDelete`, `delete`)
//!   * Kafka 4.2.0 `core/src/main/scala/kafka/server/GroupCoordinator.scala`
//!     (`commitOffsets`, `fetchOffsets`)

use crate::error::{StreamsError, StreamsResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// Wire-style identifier for a subscriber: a Kafka consumer-group or a
/// Pulsar subscription.  Both happen to share the same shape (string +
/// topic + partition); the `kind` discriminator lets cave-streams report
/// metrics per-protocol without duplicating the storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubscriberKind {
    KafkaConsumerGroup,
    PulsarSubscription,
}

/// Composite key used by [`UnifiedCursorStore`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CursorKey {
    pub kind: SubscriberKind,
    pub group_or_subscription: String,
    pub topic: String,
    pub partition: i32,
}

/// Current cursor position for a subscriber.  `committed` is monotonic
/// (a commit below the current value is silently rejected — matches
/// Kafka's `OffsetCommit` behaviour).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPosition {
    pub committed: i64,
    /// Optional metadata blob — Kafka stores up to 4 KiB here; Pulsar
    /// stores subscription properties.
    pub metadata: Option<String>,
    /// Last commit timestamp (Unix ms).  Used for retention.
    pub committed_at_ms: i64,
}

/// In-process cursor store.  Lock-free reads via DashMap.
pub struct UnifiedCursorStore {
    cursors: DashMap<CursorKey, CursorPosition>,
}

impl UnifiedCursorStore {
    pub fn new() -> Self {
        Self {
            cursors: DashMap::new(),
        }
    }

    /// Record a commit.  Rejects regression — returns
    /// `Internal("OFFSET_OUT_OF_ORDER_COMMIT")` when the new offset is
    /// strictly below the existing one (Kafka error code 28's
    /// commit-side analogue).
    pub fn commit(
        &self,
        key: CursorKey,
        offset: i64,
        metadata: Option<String>,
        now_ms: i64,
    ) -> StreamsResult<CursorPosition> {
        if offset < 0 {
            return Err(StreamsError::Internal(format!(
                "negative offset {offset}"
            )));
        }
        let mut entry = self
            .cursors
            .entry(key)
            .or_insert_with(|| CursorPosition {
                committed: -1,
                metadata: None,
                committed_at_ms: now_ms,
            });
        if offset < entry.committed {
            return Err(StreamsError::Internal(format!(
                "OFFSET_OUT_OF_ORDER_COMMIT: had={}, got={}",
                entry.committed, offset
            )));
        }
        entry.committed = offset;
        entry.metadata = metadata;
        entry.committed_at_ms = now_ms;
        Ok(entry.clone())
    }

    /// Fetch the latest committed position.  `None` when never committed.
    pub fn fetch(&self, key: &CursorKey) -> Option<CursorPosition> {
        self.cursors.get(key).map(|r| r.clone())
    }

    /// Convenience: commit a Kafka consumer-group offset.
    pub fn kafka_commit(
        &self,
        group_id: &str,
        topic: &str,
        partition: i32,
        offset: i64,
        metadata: Option<String>,
        now_ms: i64,
    ) -> StreamsResult<CursorPosition> {
        self.commit(
            CursorKey {
                kind: SubscriberKind::KafkaConsumerGroup,
                group_or_subscription: group_id.to_string(),
                topic: topic.to_string(),
                partition,
            },
            offset,
            metadata,
            now_ms,
        )
    }

    /// Convenience: mark-delete a Pulsar subscription cursor.  Pulsar
    /// uses `(ledger, entry)` IDs internally; for the unified store we
    /// flatten to a single `i64` (the entry id of the most-recently-acked
    /// message on the partition).
    pub fn pulsar_mark_delete(
        &self,
        subscription: &str,
        topic: &str,
        partition: i32,
        entry_id: i64,
        now_ms: i64,
    ) -> StreamsResult<CursorPosition> {
        self.commit(
            CursorKey {
                kind: SubscriberKind::PulsarSubscription,
                group_or_subscription: subscription.to_string(),
                topic: topic.to_string(),
                partition,
            },
            entry_id,
            None,
            now_ms,
        )
    }

    /// Read the current position for a Kafka consumer group / Pulsar
    /// subscription.  Wraps the `fetch` lookup with the right `kind`.
    pub fn kafka_fetch(
        &self,
        group_id: &str,
        topic: &str,
        partition: i32,
    ) -> Option<CursorPosition> {
        self.fetch(&CursorKey {
            kind: SubscriberKind::KafkaConsumerGroup,
            group_or_subscription: group_id.to_string(),
            topic: topic.to_string(),
            partition,
        })
    }

    pub fn pulsar_fetch(
        &self,
        subscription: &str,
        topic: &str,
        partition: i32,
    ) -> Option<CursorPosition> {
        self.fetch(&CursorKey {
            kind: SubscriberKind::PulsarSubscription,
            group_or_subscription: subscription.to_string(),
            topic: topic.to_string(),
            partition,
        })
    }

    /// Total cursors tracked across both protocols.
    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }

    /// All cursors that match `(topic, partition)` regardless of kind.
    /// Used by the storage layer to compute the "minimum acked offset"
    /// when deciding whether a tombstone or expired entry is truly
    /// reclaimable.
    pub fn min_committed_for(&self, topic: &str, partition: i32) -> Option<i64> {
        let mut min = i64::MAX;
        let mut found = false;
        for entry in self.cursors.iter() {
            let k = entry.key();
            if k.topic == topic && k.partition == partition {
                if entry.value().committed < min {
                    min = entry.value().committed;
                }
                found = true;
            }
        }
        if found {
            Some(min)
        } else {
            None
        }
    }
}

impl Default for UnifiedCursorStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Unified cursor tests — feat/cave-streams-deeper-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(tenant_id: &str, suffix: &str) -> String {
        format!("tenants/{}/{}", tenant_id, suffix)
    }

    #[test]
    fn test_kafka_commit_then_fetch_round_trip() {
        // cite: kafka 4.2.0 GroupCoordinator#handleCommitOffsets
        let tenant_id = "uc-001";
        let store = UnifiedCursorStore::new();
        store
            .kafka_commit(
                "g",
                &topic(tenant_id, "t"),
                0,
                42,
                Some("meta".into()),
                1_000,
            )
            .unwrap();
        let p = store.kafka_fetch("g", &topic(tenant_id, "t"), 0).unwrap();
        assert_eq!(p.committed, 42);
        assert_eq!(p.metadata.as_deref(), Some("meta"));
    }

    #[test]
    fn test_pulsar_mark_delete_round_trip() {
        // cite: pulsar 4.2.0 ManagedCursorImpl#asyncMarkDelete
        let tenant_id = "uc-002";
        let store = UnifiedCursorStore::new();
        store
            .pulsar_mark_delete("sub", &topic(tenant_id, "t"), 0, 99, 1_000)
            .unwrap();
        let p = store.pulsar_fetch("sub", &topic(tenant_id, "t"), 0).unwrap();
        assert_eq!(p.committed, 99);
    }

    #[test]
    fn test_commit_rejects_regression() {
        // cite: kafka 4.2.0 (commit must be ≥ existing offset)
        let tenant_id = "uc-003";
        let store = UnifiedCursorStore::new();
        store
            .kafka_commit("g", &topic(tenant_id, "t"), 0, 100, None, 1)
            .unwrap();
        let err = store.kafka_commit("g", &topic(tenant_id, "t"), 0, 50, None, 2);
        assert!(matches!(err, Err(StreamsError::Internal(_))));
    }

    #[test]
    fn test_commit_rejects_negative_offset() {
        // cite: kafka 4.2.0 errors.OFFSET_OUT_OF_RANGE for negative offsets
        let tenant_id = "uc-004";
        let store = UnifiedCursorStore::new();
        let err = store.kafka_commit("g", &topic(tenant_id, "t"), 0, -1, None, 1);
        assert!(matches!(err, Err(StreamsError::Internal(_))));
    }

    #[test]
    fn test_kafka_and_pulsar_keys_are_distinct() {
        // cite: ADR-RUNTIME-STREAMING-CONSOLIDATION-001 §addressing
        let tenant_id = "uc-005";
        let store = UnifiedCursorStore::new();
        store
            .kafka_commit("same-name", &topic(tenant_id, "t"), 0, 10, None, 1)
            .unwrap();
        store
            .pulsar_mark_delete("same-name", &topic(tenant_id, "t"), 0, 200, 1)
            .unwrap();
        assert_eq!(store.len(), 2);
        let k = store.kafka_fetch("same-name", &topic(tenant_id, "t"), 0).unwrap();
        let p = store.pulsar_fetch("same-name", &topic(tenant_id, "t"), 0).unwrap();
        assert_eq!(k.committed, 10);
        assert_eq!(p.committed, 200);
    }

    #[test]
    fn test_min_committed_for_picks_floor_across_protocols() {
        // cite: ADR-RUNTIME-STREAMING-CONSOLIDATION-001 §retention
        let tenant_id = "uc-006";
        let store = UnifiedCursorStore::new();
        store
            .kafka_commit("g1", &topic(tenant_id, "t"), 0, 100, None, 1)
            .unwrap();
        store
            .kafka_commit("g2", &topic(tenant_id, "t"), 0, 50, None, 1)
            .unwrap();
        store
            .pulsar_mark_delete("sub", &topic(tenant_id, "t"), 0, 75, 1)
            .unwrap();
        assert_eq!(
            store.min_committed_for(&topic(tenant_id, "t"), 0),
            Some(50)
        );
    }

    #[test]
    fn test_fetch_unknown_returns_none() {
        // cite: kafka 4.2.0 (fetch returns -1 / None when no commit)
        let _tenant_id = "uc-007";
        let store = UnifiedCursorStore::new();
        assert!(store.kafka_fetch("ghost", "t", 0).is_none());
        assert!(store.pulsar_fetch("ghost", "t", 0).is_none());
    }
}
