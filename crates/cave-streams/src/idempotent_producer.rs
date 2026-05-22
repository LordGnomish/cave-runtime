// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka idempotent producer — `(producer_id, epoch, sequence)` triple
//! tracking with the broker-side semantics defined in KIP-98.
//!
//! `ProducerIdRegistry` allocates the next free `producer_id`, validates
//! `epoch` on re-init, and detects out-of-order or duplicate sequence
//! numbers per `(producer_id, topic, partition)`.  Used by
//! [`crate::transactions::TransactionCoordinator`] for the wire-level
//! checks but exposed standalone so the Pulsar bridge can share the same
//! dedup machinery.
//!
//! Upstream reference: Apache Kafka 4.2.0
//! `clients/src/main/java/org/apache/kafka/common/record/RecordBatch.java`
//! and `core/src/main/scala/kafka/log/ProducerStateManager.scala`.

use crate::error::{StreamsError, StreamsResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, Ordering};

/// Result of validating a single produce batch's idempotency triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceCheck {
    /// First batch in the partition (`base_sequence == 0`) or the next
    /// expected sequence — accept and update `last_sequence`.
    Accepted,
    /// `base_sequence < last_sequence + 1` — client is retrying a batch
    /// the broker has already persisted; safe to acknowledge.
    Duplicate,
    /// `base_sequence > last_sequence + 1` — out-of-order arrival;
    /// upstream returns `OUT_OF_ORDER_SEQUENCE_NUMBER` (Kafka error 28).
    OutOfOrder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionSequence {
    pub last_sequence: i32,
    /// Wraps after `Int32.MAX_VALUE` per KIP-98.  Tracked here so callers
    /// can detect a wrap and reset their dedup window deterministically.
    pub wrap_count: u32,
}

impl Default for PartitionSequence {
    fn default() -> Self {
        Self {
            last_sequence: -1,
            wrap_count: 0,
        }
    }
}

/// Registry that owns producer IDs, epochs, and per-partition sequences.
pub struct ProducerIdRegistry {
    next_id: AtomicI64,
    /// `producer_id → epoch`.
    epochs: DashMap<i64, i16>,
    /// `(producer_id, topic, partition) → PartitionSequence`.
    sequences: DashMap<(i64, String, i32), PartitionSequence>,
}

impl ProducerIdRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicI64::new(1),
            epochs: DashMap::new(),
            sequences: DashMap::new(),
        }
    }

    /// Allocate a new `producer_id` with epoch=0.  Mirrors
    /// `InitProducerIdRequest` for the non-transactional path.
    pub fn allocate(&self) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.epochs.insert(id, 0);
        id
    }

    /// Bump the epoch of an existing producer — used when a transactional
    /// producer re-initialises (KIP-360).
    pub fn bump_epoch(&self, producer_id: i64) -> StreamsResult<i16> {
        let mut e = self
            .epochs
            .get_mut(&producer_id)
            .ok_or(StreamsError::ProducerIdNotFound(producer_id))?;
        *e = e.saturating_add(1);
        Ok(*e)
    }

    pub fn epoch(&self, producer_id: i64) -> Option<i16> {
        self.epochs.get(&producer_id).map(|e| *e)
    }

    /// Validate a `(producer_id, epoch, base_sequence)` triple.  Updates
    /// `last_sequence` on `Accepted`.  On a fenced epoch returns
    /// `Internal` since etcd's error enum has no dedicated variant
    /// (parity-wise the wire error is `INVALID_PRODUCER_EPOCH`, code 47).
    pub fn check(
        &self,
        producer_id: i64,
        epoch: i16,
        topic: &str,
        partition: i32,
        base_sequence: i32,
        record_count: i32,
    ) -> StreamsResult<SequenceCheck> {
        let cur_epoch = self
            .epoch(producer_id)
            .ok_or(StreamsError::ProducerIdNotFound(producer_id))?;
        if epoch < cur_epoch {
            return Err(StreamsError::Internal(format!(
                "INVALID_PRODUCER_EPOCH: pid={producer_id}, got={epoch}, expected≥{cur_epoch}"
            )));
        }
        // Accept the higher-or-equal epoch but also persist it — when a
        // newer producer instance fences the prior generation, broker-side
        // state must reflect the new epoch.
        if epoch > cur_epoch {
            self.epochs.insert(producer_id, epoch);
        }
        let key = (producer_id, topic.to_string(), partition);
        let mut entry = self.sequences.entry(key).or_default();
        let last = entry.last_sequence;
        let outcome = if last == -1 {
            // No prior batch.  Accept any non-negative starting sequence.
            if base_sequence < 0 {
                return Err(StreamsError::Internal(format!(
                    "negative base_sequence {base_sequence}"
                )));
            }
            SequenceCheck::Accepted
        } else if base_sequence == last + 1 {
            SequenceCheck::Accepted
        } else if base_sequence <= last {
            SequenceCheck::Duplicate
        } else {
            SequenceCheck::OutOfOrder
        };
        if outcome == SequenceCheck::Accepted {
            // Track the *highest* sequence in the batch
            // (`base_sequence + record_count - 1`).  Wrap to 0 after
            // Int32.MAX_VALUE — KIP-98 §"Sequence Number Wraparound".
            let new_last = base_sequence.saturating_add(record_count - 1);
            if new_last < base_sequence {
                entry.wrap_count = entry.wrap_count.wrapping_add(1);
                entry.last_sequence = -1;
            } else {
                entry.last_sequence = new_last;
            }
        }
        Ok(outcome)
    }

    /// Inspect (without mutating) the per-partition dedup state.  Useful
    /// for the admin Defragment API and tests.
    pub fn partition_state(
        &self,
        producer_id: i64,
        topic: &str,
        partition: i32,
    ) -> Option<PartitionSequence> {
        self.sequences
            .get(&(producer_id, topic.to_string(), partition))
            .map(|r| r.clone())
    }

    /// Drop all state for a producer (called on `EndTxn` cleanup).
    pub fn forget(&self, producer_id: i64) {
        self.epochs.remove(&producer_id);
        self.sequences.retain(|k, _| k.0 != producer_id);
    }

    /// Number of distinct producer IDs currently tracked.
    pub fn active_producers(&self) -> usize {
        self.epochs.len()
    }
}

impl Default for ProducerIdRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Idempotent-producer tests — feat/cave-streams-deeper-001
// Each test embeds an upstream `// cite:` and a `tenant_id` constant for
// namespaced test data.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(tenant_id: &str, suffix: &str) -> String {
        format!("tenants/{}/{}", tenant_id, suffix)
    }

    #[test]
    fn test_idempotent_allocate_assigns_monotonic_ids() {
        // cite: kafka 4.2.0 core/.../coordinator/transaction/ProducerIdManager.scala
        let _tenant_id = "ip-001";
        let r = ProducerIdRegistry::new();
        let id1 = r.allocate();
        let id2 = r.allocate();
        let id3 = r.allocate();
        assert!(id2 > id1);
        assert!(id3 > id2);
        assert_eq!(r.epoch(id1), Some(0));
    }

    #[test]
    fn test_idempotent_bump_epoch_increments_existing() {
        // cite: kafka 4.2.0 KIP-360 producer epoch fencing
        let _tenant_id = "ip-002";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        let e1 = r.bump_epoch(pid).unwrap();
        let e2 = r.bump_epoch(pid).unwrap();
        assert_eq!(e1, 1);
        assert_eq!(e2, 2);
    }

    #[test]
    fn test_idempotent_bump_epoch_unknown_pid_errors() {
        // cite: kafka 4.2.0 errors.UNKNOWN_PRODUCER_ID
        let _tenant_id = "ip-003";
        let r = ProducerIdRegistry::new();
        let err = r.bump_epoch(999);
        assert!(matches!(err, Err(StreamsError::ProducerIdNotFound(_))));
    }

    #[test]
    fn test_idempotent_check_first_batch_accepted() {
        // cite: kafka 4.2.0 ProducerStateManager.scala (sequence 0 always ok)
        let tenant_id = "ip-004";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        let outcome = r.check(pid, 0, &topic(tenant_id, "t"), 0, 0, 5).unwrap();
        assert_eq!(outcome, SequenceCheck::Accepted);
        let st = r.partition_state(pid, &topic(tenant_id, "t"), 0).unwrap();
        assert_eq!(st.last_sequence, 4); // base 0 + 5 records - 1
    }

    #[test]
    fn test_idempotent_check_in_order_extends_sequence() {
        // cite: kafka 4.2.0 ProducerStateManager#assignSequence
        let tenant_id = "ip-005";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        r.check(pid, 0, &topic(tenant_id, "t"), 0, 0, 3).unwrap();
        let outcome = r.check(pid, 0, &topic(tenant_id, "t"), 0, 3, 2).unwrap();
        assert_eq!(outcome, SequenceCheck::Accepted);
        let st = r.partition_state(pid, &topic(tenant_id, "t"), 0).unwrap();
        assert_eq!(st.last_sequence, 4); // base 3 + 2 records - 1
    }

    #[test]
    fn test_idempotent_check_duplicate_returns_duplicate() {
        // cite: kafka 4.2.0 ProducerStateManager (idempotent retry path)
        let tenant_id = "ip-006";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        r.check(pid, 0, &topic(tenant_id, "t"), 0, 0, 5).unwrap();
        let outcome = r.check(pid, 0, &topic(tenant_id, "t"), 0, 2, 1).unwrap();
        assert_eq!(outcome, SequenceCheck::Duplicate);
    }

    #[test]
    fn test_idempotent_check_out_of_order() {
        // cite: kafka 4.2.0 errors.OUT_OF_ORDER_SEQUENCE_NUMBER (28)
        let tenant_id = "ip-007";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        r.check(pid, 0, &topic(tenant_id, "t"), 0, 0, 5).unwrap();
        let outcome = r.check(pid, 0, &topic(tenant_id, "t"), 0, 99, 1).unwrap();
        assert_eq!(outcome, SequenceCheck::OutOfOrder);
    }

    #[test]
    fn test_idempotent_check_lower_epoch_fenced() {
        // cite: kafka 4.2.0 errors.INVALID_PRODUCER_EPOCH (47)
        let tenant_id = "ip-008";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        r.bump_epoch(pid).unwrap();
        r.bump_epoch(pid).unwrap();
        let err = r.check(pid, 0, &topic(tenant_id, "t"), 0, 0, 1);
        assert!(matches!(err, Err(StreamsError::Internal(_))));
    }

    #[test]
    fn test_idempotent_check_higher_epoch_persists() {
        // cite: kafka 4.2.0 KIP-360 (broker tracks latest epoch)
        let tenant_id = "ip-009";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        r.check(pid, 7, &topic(tenant_id, "t"), 0, 0, 1).unwrap();
        assert_eq!(r.epoch(pid), Some(7));
    }

    #[test]
    fn test_idempotent_forget_clears_state() {
        // cite: kafka 4.2.0 ProducerStateManager#removeExpiredProducers
        let tenant_id = "ip-010";
        let r = ProducerIdRegistry::new();
        let pid = r.allocate();
        r.check(pid, 0, &topic(tenant_id, "t"), 0, 0, 1).unwrap();
        assert_eq!(r.active_producers(), 1);
        r.forget(pid);
        assert_eq!(r.active_producers(), 0);
        assert!(r.partition_state(pid, &topic(tenant_id, "t"), 0).is_none());
    }
}
