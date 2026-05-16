// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/MessageDeduplication.java

//! Producer-side dedup ledger.
//!
//! Pulsar drops a `(producer_name, sequence_id)` duplicate by tracking
//! the *highest sequence id observed per producer* on the topic.  An
//! incoming SEND is admitted iff its `sequence_id > last_seen[producer]`,
//! otherwise the broker returns `SendReceipt` with the cached id (the
//! producer never knows it dropped — this is "at-most-once for the
//! same `(name, sequence)`, at-least-once for distinct ones").
//!
//! The legacy dispatcher [`crate::pulsar_dispatch`] doesn't reach into
//! producer state, so this module exposes a [`DedupHook`] trait that
//! `pulsar_dispatch.rs` (or the broker SEND handler) calls before
//! committing a record.  The default implementation
//! [`InMemoryDedupHook`] is sufficient for in-process tests.

use std::collections::HashMap;
use std::sync::Mutex;

/// Outcome of a dedup check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupVerdict {
    /// Record is new — broker should append it.
    Accept,
    /// Record duplicates an earlier admit — broker should drop it but
    /// echo a receipt as if it succeeded.
    Duplicate,
}

/// Hook trait — `pulsar_dispatch` or the SEND handler calls
/// [`check`](DedupHook::check) before deciding to append.
pub trait DedupHook: Send + Sync {
    /// Returns whether to accept the record + records the new high
    /// watermark when accepting.  Implementations MUST be idempotent
    /// under retries.
    fn check(&self, producer_name: &str, sequence_id: u64) -> DedupVerdict;

    /// Highest sequence id seen for this producer, or `None` when the
    /// producer is unknown.
    fn last_seen(&self, producer_name: &str) -> Option<u64>;

    /// Drop a producer from the ledger (used on producer disconnect
    /// after `producer_access_mode = Exclusive`).
    fn forget(&self, producer_name: &str);
}

#[derive(Debug, Default)]
pub struct InMemoryDedupHook {
    inner: Mutex<HashMap<String, u64>>,
}

impl InMemoryDedupHook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn producer_count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

impl DedupHook for InMemoryDedupHook {
    fn check(&self, producer_name: &str, sequence_id: u64) -> DedupVerdict {
        let mut inner = self.inner.lock().unwrap();
        match inner.get(producer_name) {
            Some(&seen) if sequence_id <= seen => DedupVerdict::Duplicate,
            _ => {
                inner.insert(producer_name.to_string(), sequence_id);
                DedupVerdict::Accept
            }
        }
    }

    fn last_seen(&self, producer_name: &str) -> Option<u64> {
        self.inner.lock().unwrap().get(producer_name).copied()
    }

    fn forget(&self, producer_name: &str) {
        self.inner.lock().unwrap().remove(producer_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dedup_accepts_first_sequence_from_new_producer() {
        // cite: pulsar 4.2.0 MessageDeduplication accept on first SEND
        // ensemble = dd-001
        let h = InMemoryDedupHook::new();
        assert_eq!(h.check("p-1", 0), DedupVerdict::Accept);
        assert_eq!(h.last_seen("p-1"), Some(0));
    }

    #[test]
    fn test_dedup_rejects_same_sequence_id() {
        // cite: pulsar 4.2.0 dup on equal sequence
        // ensemble = dd-002
        let h = InMemoryDedupHook::new();
        h.check("p-1", 5);
        assert_eq!(h.check("p-1", 5), DedupVerdict::Duplicate);
    }

    #[test]
    fn test_dedup_rejects_lower_sequence_id() {
        // cite: pulsar 4.2.0 dup on lower sequence (out-of-order retry)
        // ensemble = dd-003
        let h = InMemoryDedupHook::new();
        h.check("p-1", 10);
        assert_eq!(h.check("p-1", 7), DedupVerdict::Duplicate);
        // Watermark stays at 10.
        assert_eq!(h.last_seen("p-1"), Some(10));
    }

    #[test]
    fn test_dedup_accepts_strictly_higher_sequence_id() {
        // cite: pulsar 4.2.0 accept on monotonic increase
        // ensemble = dd-004
        let h = InMemoryDedupHook::new();
        h.check("p-1", 10);
        assert_eq!(h.check("p-1", 11), DedupVerdict::Accept);
        assert_eq!(h.last_seen("p-1"), Some(11));
    }

    #[test]
    fn test_dedup_independent_per_producer() {
        // cite: pulsar 4.2.0 per-producer-name dedup ledger
        // ensemble = dd-005
        let h = InMemoryDedupHook::new();
        h.check("p-1", 100);
        // p-2 starts fresh, must accept seq=0.
        assert_eq!(h.check("p-2", 0), DedupVerdict::Accept);
    }

    #[test]
    fn test_dedup_forget_clears_producer_state() {
        // cite: pulsar 4.2.0 producer disconnect cleanup
        // ensemble = dd-006
        let h = InMemoryDedupHook::new();
        h.check("p-1", 5);
        h.forget("p-1");
        assert_eq!(h.last_seen("p-1"), None);
        // Fresh sequence again admitted.
        assert_eq!(h.check("p-1", 0), DedupVerdict::Accept);
    }

    #[test]
    fn test_dedup_unknown_producer_last_seen_is_none() {
        // cite: pulsar 4.2.0 absent producer returns -1 sentinel
        // ensemble = dd-007
        let h = InMemoryDedupHook::new();
        assert_eq!(h.last_seen("nobody"), None);
    }

    #[test]
    fn test_dedup_producer_count_tracks_distinct_names() {
        // cite: pulsar 4.2.0 MessageDeduplication.cursorNames count
        // ensemble = dd-008
        let h = InMemoryDedupHook::new();
        h.check("p-1", 0);
        h.check("p-2", 0);
        h.check("p-1", 1); // same producer, no new entry
        assert_eq!(h.producer_count(), 2);
    }
}
