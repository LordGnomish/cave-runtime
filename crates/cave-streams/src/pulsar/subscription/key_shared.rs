// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/.../service/persistent/PersistentStickyKeyDispatcherMultipleConsumers.java
//   pulsar-broker/.../service/HashRangeAutoSplitStickyKeyConsumerSelector.java
//   pulsar-broker/.../service/HashRangeExclusiveStickyKeyConsumerSelector.java

//! Key_Shared subscription — sticky-by-key routing.
//!
//! Two modes mirror Pulsar:
//! - [`KeySharedMode::AutoSplit`] — broker partitions the 65536-slot
//!   hash space evenly across consumers.  When a consumer leaves, its
//!   range is reabsorbed; remaining consumers split the gap.
//! - [`KeySharedMode::Sticky`] — each consumer declares an explicit
//!   non-overlapping `[low, high]` range when subscribing.

use super::{key_to_slot, PolicyConsumer, SubscriptionPolicy, HASH_RANGE_SIZE};
use crate::error::{StreamsError, StreamsResult};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySharedMode {
    AutoSplit,
    Sticky,
}

/// Explicit hash range used in `KeySharedMode::Sticky`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickyRange {
    pub low: u16,
    pub high: u16,
}

impl StickyRange {
    pub fn new(low: u16, high: u16) -> StreamsResult<Self> {
        if low > high {
            return Err(StreamsError::Internal(format!(
                "sticky range low={low} > high={high}"
            )));
        }
        Ok(Self { low, high })
    }

    pub fn contains(&self, slot: u16) -> bool {
        slot >= self.low && slot <= self.high
    }

    pub fn overlaps(&self, other: &StickyRange) -> bool {
        !(self.high < other.low || other.high < self.low)
    }
}

#[derive(Debug)]
pub struct KeySharedPolicy {
    mode: KeySharedMode,
    consumers: BTreeMap<u64, PolicyConsumer>,
    /// Explicit ranges (Sticky mode only).  Auto-split derives them on
    /// demand from the consumer set.
    sticky_ranges: BTreeMap<u64, StickyRange>,
}

impl KeySharedPolicy {
    pub fn new(mode: KeySharedMode) -> Self {
        Self {
            mode,
            consumers: BTreeMap::new(),
            sticky_ranges: BTreeMap::new(),
        }
    }

    pub fn mode(&self) -> KeySharedMode {
        self.mode
    }

    /// Sticky-mode attach — caller must supply an explicit range.
    pub fn add_consumer_with_range(
        &mut self,
        c: PolicyConsumer,
        range: StickyRange,
    ) -> StreamsResult<()> {
        if self.mode != KeySharedMode::Sticky {
            return Err(StreamsError::Internal(
                "explicit ranges only valid in Sticky mode".into(),
            ));
        }
        for existing in self.sticky_ranges.values() {
            if existing.overlaps(&range) {
                return Err(StreamsError::Internal(format!(
                    "sticky range {:?} overlaps with {:?}",
                    range, existing
                )));
            }
        }
        self.sticky_ranges.insert(c.consumer_id, range);
        self.consumers.insert(c.consumer_id, c);
        Ok(())
    }

    /// Auto-split: divide the 65536-slot space across N consumers
    /// (sorted by consumer_id).  Returns the (low, high) for each.
    /// Hash range `[0, HASH_RANGE_SIZE-1]` for N consumers gives
    /// chunk = HASH_RANGE_SIZE / N (the last chunk picks up the
    /// remainder slots).
    pub fn auto_split_ranges(&self) -> Vec<(u64, StickyRange)> {
        if self.mode != KeySharedMode::AutoSplit || self.consumers.is_empty() {
            return vec![];
        }
        let ids: Vec<u64> = self.consumers.keys().copied().collect();
        let n = ids.len() as u32;
        let chunk = HASH_RANGE_SIZE / n;
        let mut out = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let low = (i as u32) * chunk;
            let high = if i + 1 == ids.len() {
                HASH_RANGE_SIZE - 1
            } else {
                ((i + 1) as u32) * chunk - 1
            };
            out.push((
                *id,
                StickyRange {
                    low: low as u16,
                    high: high as u16,
                },
            ));
        }
        out
    }

    /// Owner of the slot under the current configuration.
    pub fn owner_of_slot(&self, slot: u16) -> Option<u64> {
        match self.mode {
            KeySharedMode::Sticky => self
                .sticky_ranges
                .iter()
                .find(|(_, r)| r.contains(slot))
                .map(|(id, _)| *id),
            KeySharedMode::AutoSplit => self
                .auto_split_ranges()
                .into_iter()
                .find(|(_, r)| r.contains(slot))
                .map(|(id, _)| id),
        }
    }
}

impl SubscriptionPolicy for KeySharedPolicy {
    fn add_consumer(&mut self, c: PolicyConsumer) -> StreamsResult<()> {
        match self.mode {
            KeySharedMode::AutoSplit => {
                self.consumers.insert(c.consumer_id, c);
                Ok(())
            }
            KeySharedMode::Sticky => Err(StreamsError::Internal(
                "Sticky mode requires explicit range — use add_consumer_with_range".into(),
            )),
        }
    }

    fn remove_consumer(&mut self, consumer_id: u64) {
        self.consumers.remove(&consumer_id);
        self.sticky_ranges.remove(&consumer_id);
    }

    fn pick(&mut self, key: Option<&[u8]>) -> Option<u64> {
        let key = key?;
        let slot = key_to_slot(key);
        let id = self.owner_of_slot(slot)?;
        // Skip consumers without permits — message stays in the backlog.
        match self.consumers.get(&id) {
            Some(c) if c.has_permits => Some(id),
            _ => None,
        }
    }

    fn consumer_count(&self) -> usize {
        self.consumers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_shared_auto_split_partitions_hash_space_evenly() {
        // cite: pulsar 4.2.0 HashRangeAutoSplitStickyKeyConsumerSelector
        // ensemble = ks-001
        let mut p = KeySharedPolicy::new(KeySharedMode::AutoSplit);
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        p.add_consumer(PolicyConsumer::new(2)).unwrap();
        let ranges = p.auto_split_ranges();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].1.low, 0);
        assert_eq!(ranges[0].1.high, 32767);
        assert_eq!(ranges[1].1.low, 32768);
        assert_eq!(ranges[1].1.high, 65535);
    }

    #[test]
    fn test_key_shared_auto_split_last_chunk_absorbs_remainder() {
        // cite: pulsar 4.2.0 last consumer gets remainder slots
        // ensemble = ks-002
        let mut p = KeySharedPolicy::new(KeySharedMode::AutoSplit);
        for id in [1u64, 2, 3] {
            p.add_consumer(PolicyConsumer::new(id)).unwrap();
        }
        let r = p.auto_split_ranges();
        // 65536 / 3 = 21845 r=1; first two chunks [0,21844] [21845,43689]
        // last chunk picks up 43690..=65535
        assert_eq!(r[2].1.high, 65535);
    }

    #[test]
    fn test_key_shared_sticky_same_key_routes_to_same_consumer() {
        // cite: pulsar 4.2.0 sticky key dispatcher consistent assignment
        // ensemble = ks-003
        let mut p = KeySharedPolicy::new(KeySharedMode::AutoSplit);
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        p.add_consumer(PolicyConsumer::new(2)).unwrap();
        let k = b"order-42";
        let a = p.pick(Some(k)).unwrap();
        let b = p.pick(Some(k)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_key_shared_sticky_explicit_range_routing() {
        // cite: pulsar 4.2.0 HashRangeExclusiveStickyKeyConsumerSelector
        // ensemble = ks-004
        let mut p = KeySharedPolicy::new(KeySharedMode::Sticky);
        p.add_consumer_with_range(PolicyConsumer::new(1), StickyRange::new(0, 32767).unwrap())
            .unwrap();
        p.add_consumer_with_range(
            PolicyConsumer::new(2),
            StickyRange::new(32768, 65535).unwrap(),
        )
        .unwrap();
        // Probe each half of the hash space.
        let slot_low = 100u16;
        let slot_high = 60_000u16;
        assert_eq!(p.owner_of_slot(slot_low), Some(1));
        assert_eq!(p.owner_of_slot(slot_high), Some(2));
    }

    #[test]
    fn test_key_shared_sticky_rejects_overlapping_range() {
        // cite: pulsar 4.2.0 exclusive ranges disjoint
        // ensemble = ks-005
        let mut p = KeySharedPolicy::new(KeySharedMode::Sticky);
        p.add_consumer_with_range(PolicyConsumer::new(1), StickyRange::new(0, 100).unwrap())
            .unwrap();
        let err = p.add_consumer_with_range(
            PolicyConsumer::new(2),
            StickyRange::new(50, 200).unwrap(),
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_key_shared_sticky_rejects_default_add_consumer() {
        // cite: pulsar 4.2.0 Sticky mode requires hashRanges
        // ensemble = ks-006
        let mut p = KeySharedPolicy::new(KeySharedMode::Sticky);
        assert!(p.add_consumer(PolicyConsumer::new(1)).is_err());
    }

    #[test]
    fn test_key_shared_rebalance_on_consumer_remove() {
        // cite: pulsar 4.2.0 auto-split redistributes when consumer leaves
        // ensemble = ks-007
        let mut p = KeySharedPolicy::new(KeySharedMode::AutoSplit);
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        p.add_consumer(PolicyConsumer::new(2)).unwrap();
        let before = p.auto_split_ranges();
        assert_eq!(before.len(), 2);
        p.remove_consumer(1);
        let after = p.auto_split_ranges();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].1.low, 0);
        assert_eq!(after[0].1.high, 65535);
    }

    #[test]
    fn test_key_shared_pick_without_key_returns_none() {
        // cite: pulsar 4.2.0 keyless dispatch on Key_Shared is None
        // ensemble = ks-008
        let mut p = KeySharedPolicy::new(KeySharedMode::AutoSplit);
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        assert_eq!(p.pick(None), None);
    }

    #[test]
    fn test_key_shared_skips_owner_without_permits() {
        // cite: pulsar 4.2.0 Key_Shared blocks key when owner has no permits
        // ensemble = ks-009
        let mut p = KeySharedPolicy::new(KeySharedMode::AutoSplit);
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        p.consumers.get_mut(&1).unwrap().has_permits = false;
        // Key would map to consumer 1 but they can't take it.
        assert_eq!(p.pick(Some(b"x")), None);
    }
}
