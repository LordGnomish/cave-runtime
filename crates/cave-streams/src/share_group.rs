// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka share groups (KIP-932) — queue-style consumption.
//!
//! upstream: apache/kafka —
//!   * `server-common/.../share/{RecordState,AcknowledgeType}`
//!   * `core/.../share/SharePartition.java` (in-flight record state machine)
//!   * `group-coordinator/.../share/{ShareGroup,ShareGroupMember,
//!     ShareGroupState}`
//!
//! A *share group* lets many consumers cooperatively drain a single
//! partition the way a classic message queue does, instead of the
//! exclusive partition-per-member ownership of a consumer group. Records
//! are individually *acquired* (with a time-bounded acquisition lock),
//! then *acknowledged* with one of three dispositions:
//!   * `ACCEPT`  → the record is done (ACKNOWLEDGED),
//!   * `RELEASE` → hand it back for redelivery (AVAILABLE again),
//!   * `REJECT`  → drop it permanently (ARCHIVED).
//!
//! Each delivery bumps a per-record delivery count; once it reaches
//! `max_delivery_count` the record is ARCHIVED instead of being redelivered
//! (the poison-pill guard). The *share-partition start offset* (SPSO)
//! advances over any contiguous prefix of ACKNOWLEDGED/ARCHIVED records, so
//! completed work is reclaimed.
//!
//! This module is the in-memory parity port of `SharePartition`'s per-offset
//! state machine plus a thin [`ShareGroup`] registry. It is the Pulsar
//! `Shared`-subscription analog on the Kafka side; classic consumer-group
//! semantics live in `consumer_group.rs`.

use crate::error::{StreamsError, StreamsResult};
use std::collections::BTreeMap;

/// Per-record delivery state — `org.apache.kafka.server.share.RecordState`.
///
/// Ordinals match Kafka's enum (`AVAILABLE=0, ACQUIRED=1, ACKNOWLEDGED=2,
/// ARCHIVED=4`) so they can be persisted to the share-state topic unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordState {
    /// Free to be acquired by any member.
    Available = 0,
    /// Held by a member under an acquisition lock.
    Acquired = 1,
    /// Successfully processed — terminal.
    Acknowledged = 2,
    /// Discarded (rejected or out of delivery attempts) — terminal.
    Archived = 4,
}

impl RecordState {
    fn is_terminal(self) -> bool {
        matches!(self, RecordState::Acknowledged | RecordState::Archived)
    }
}

/// Acknowledgement disposition — `org.apache.kafka.clients.consumer.AcknowledgeType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgeType {
    /// `ACCEPT(1)` — processed successfully.
    Accept = 1,
    /// `RELEASE(2)` — return for redelivery.
    Release = 2,
    /// `REJECT(3)` — drop permanently.
    Reject = 3,
}

#[derive(Debug, Clone)]
struct InFlightRecord {
    state: RecordState,
    delivery_count: u32,
    acquired_by: Option<String>,
    lock_expiry_ms: u64,
}

impl InFlightRecord {
    fn available() -> Self {
        InFlightRecord {
            state: RecordState::Available,
            delivery_count: 0,
            acquired_by: None,
            lock_expiry_ms: 0,
        }
    }
}

/// In-flight state machine for one share-partition — `SharePartition`.
#[derive(Debug)]
pub struct SharePartition {
    /// Share-partition start offset (SPSO) — lowest offset still tracked.
    start_offset: u64,
    /// Highest tracked offset + 1.
    end_offset: u64,
    max_delivery_count: u32,
    record_lock_duration_ms: u64,
    records: BTreeMap<u64, InFlightRecord>,
}

impl SharePartition {
    pub fn new(start_offset: u64, max_delivery_count: u32, record_lock_duration_ms: u64) -> Self {
        SharePartition {
            start_offset,
            end_offset: start_offset,
            max_delivery_count,
            record_lock_duration_ms,
            records: BTreeMap::new(),
        }
    }

    pub fn start_offset(&self) -> u64 {
        self.start_offset
    }

    pub fn end_offset(&self) -> u64 {
        self.end_offset
    }

    /// `SharePartition.acquire` — hand a member up to `max_records` available
    /// records from `start_offset` up to (exclusive) `fetch_end_offset` (the
    /// log high-watermark). Acquired records get an acquisition lock and a
    /// bumped delivery count. Returns the acquired offsets in order.
    pub fn acquire(
        &mut self,
        member_id: &str,
        max_records: usize,
        fetch_end_offset: u64,
        now_ms: u64,
    ) -> Vec<u64> {
        let mut acquired = Vec::new();
        let mut offset = self.start_offset;
        while offset < fetch_end_offset && acquired.len() < max_records {
            // Offsets at/above end_offset have never been seen → implicitly
            // AVAILABLE; materialise them lazily on first acquire.
            let rec = self
                .records
                .entry(offset)
                .or_insert_with(InFlightRecord::available);
            if rec.state == RecordState::Available {
                rec.state = RecordState::Acquired;
                rec.delivery_count += 1;
                rec.acquired_by = Some(member_id.to_string());
                rec.lock_expiry_ms = now_ms + self.record_lock_duration_ms;
                acquired.push(offset);
            }
            if offset + 1 > self.end_offset {
                self.end_offset = offset + 1;
            }
            offset += 1;
        }
        acquired
    }

    /// `SharePartition.acknowledge` — apply a member's disposition to one
    /// previously-acquired offset, then advance the SPSO over any completed
    /// prefix.
    pub fn acknowledge(
        &mut self,
        member_id: &str,
        offset: u64,
        ack_type: AcknowledgeType,
        now_ms: u64,
    ) -> StreamsResult<()> {
        let max = self.max_delivery_count;
        let rec = self.records.get_mut(&offset).ok_or_else(|| {
            StreamsError::Internal(format!("share: offset {offset} not in flight"))
        })?;
        if rec.state != RecordState::Acquired {
            return Err(StreamsError::Internal(format!(
                "share: offset {offset} is {:?}, not ACQUIRED",
                rec.state
            )));
        }
        if rec.acquired_by.as_deref() != Some(member_id) {
            return Err(StreamsError::Internal(format!(
                "share: offset {offset} acquired by another member"
            )));
        }
        match ack_type {
            AcknowledgeType::Accept => rec.state = RecordState::Acknowledged,
            AcknowledgeType::Reject => rec.state = RecordState::Archived,
            AcknowledgeType::Release => {
                // RED placeholder — poison-pill guard not yet ported.
                let _ = max;
                rec.state = RecordState::Available;
            }
        }
        rec.acquired_by = None;
        let _ = now_ms;
        self.advance_start_offset();
        Ok(())
    }

    /// `SharePartition.releaseAcquisitionLockOnTimeout` — acquired records
    /// whose lock has expired revert to AVAILABLE (or ARCHIVED if they are
    /// out of delivery attempts). Returns the offsets that were released.
    pub fn release_expired_locks(&mut self, now_ms: u64) -> Vec<u64> {
        let max = self.max_delivery_count;
        let mut released = Vec::new();
        for (offset, rec) in self.records.iter_mut() {
            if rec.state == RecordState::Acquired && now_ms >= rec.lock_expiry_ms {
                rec.state = if rec.delivery_count >= max {
                    RecordState::Archived
                } else {
                    RecordState::Available
                };
                rec.acquired_by = None;
                released.push(*offset);
            }
        }
        self.advance_start_offset();
        released
    }

    /// Slide the SPSO forward over a contiguous prefix of terminal records,
    /// dropping their tracking entries.
    fn advance_start_offset(&mut self) {
        // RED placeholder — SPSO sliding over the terminal prefix not yet ported.
    }

    fn count(&self, state: RecordState) -> usize {
        self.records.values().filter(|r| r.state == state).count()
    }

    pub fn available_count(&self) -> usize {
        self.count(RecordState::Available)
    }

    pub fn acquired_count(&self) -> usize {
        self.count(RecordState::Acquired)
    }

    pub fn archived_count(&self) -> usize {
        self.count(RecordState::Archived)
    }

    /// Current delivery count for an in-flight offset (0 if untracked).
    pub fn delivery_count(&self, offset: u64) -> u32 {
        self.records.get(&offset).map(|r| r.delivery_count).unwrap_or(0)
    }
}

/// A member of a share group — `ShareGroupMember`.
#[derive(Debug, Clone)]
pub struct ShareGroupMember {
    pub member_id: String,
    /// Share session epoch (KIP-932 ShareSession) — bumped on each
    /// (re)join; a stale epoch is fenced.
    pub member_epoch: i32,
}

/// Share group registry — `ShareGroup` coordinator surface.
///
/// Owns the member set and one [`SharePartition`] state machine per
/// `(topic, partition)`.
#[derive(Debug, Default)]
pub struct ShareGroup {
    pub group_id: String,
    members: BTreeMap<String, ShareGroupMember>,
    next_epoch: i32,
    partitions: BTreeMap<(String, i32), SharePartition>,
}

impl ShareGroup {
    pub fn new(group_id: impl Into<String>) -> Self {
        ShareGroup {
            group_id: group_id.into(),
            members: BTreeMap::new(),
            next_epoch: 0,
            partitions: BTreeMap::new(),
        }
    }

    /// Join (or re-join) a member, handing back a freshly bumped epoch.
    pub fn join(&mut self, member_id: impl Into<String>) -> i32 {
        self.next_epoch += 1;
        let epoch = self.next_epoch;
        let member_id = member_id.into();
        self.members.insert(
            member_id.clone(),
            ShareGroupMember {
                member_id,
                member_epoch: epoch,
            },
        );
        epoch
    }

    /// Fence a request carrying a stale epoch (KIP-932 epoch fencing).
    pub fn check_epoch(&self, member_id: &str, epoch: i32) -> StreamsResult<()> {
        match self.members.get(member_id) {
            Some(m) if m.member_epoch == epoch => Ok(()),
            Some(m) => Err(StreamsError::IllegalGeneration {
                group: self.group_id.clone(),
                expected: m.member_epoch,
                got: epoch,
            }),
            None => Err(StreamsError::MemberNotFound {
                group: self.group_id.clone(),
                member: member_id.to_string(),
            }),
        }
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Get-or-create the share-partition state machine for a topic-partition.
    pub fn share_partition(
        &mut self,
        topic: impl Into<String>,
        partition: i32,
        max_delivery_count: u32,
        record_lock_duration_ms: u64,
    ) -> &mut SharePartition {
        self.partitions
            .entry((topic.into(), partition))
            .or_insert_with(|| SharePartition::new(0, max_delivery_count, record_lock_duration_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_DELIVERY: u32 = 3;
    const LOCK_MS: u64 = 30_000;

    fn sp() -> SharePartition {
        SharePartition::new(0, MAX_DELIVERY, LOCK_MS)
    }

    // ── acquire ───────────────────────────────────────────────────────────

    #[test]
    fn acquire_materialises_and_locks_records() {
        let mut p = sp();
        // Log has offsets 0..5 available.
        let got = p.acquire("m1", 10, 5, 1000);
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
        assert_eq!(p.acquired_count(), 5);
        assert_eq!(p.delivery_count(0), 1);
        assert_eq!(p.end_offset(), 5);
    }

    #[test]
    fn acquire_respects_max_records() {
        let mut p = sp();
        let got = p.acquire("m1", 2, 100, 1000);
        assert_eq!(got, vec![0, 1]);
        assert_eq!(p.acquired_count(), 2);
    }

    #[test]
    fn acquired_records_are_not_reacquired() {
        let mut p = sp();
        p.acquire("m1", 3, 100, 1000);
        // m2 finds nothing available below offset 3 — picks up fresh ones.
        let got = p.acquire("m2", 2, 100, 1000);
        assert_eq!(got, vec![3, 4]);
    }

    // ── acknowledge ────────────────────────────────────────────────────────

    #[test]
    fn accept_acknowledges_and_advances_spso() {
        let mut p = sp();
        p.acquire("m1", 3, 100, 1000);
        p.acknowledge("m1", 0, AcknowledgeType::Accept, 1100).unwrap();
        p.acknowledge("m1", 1, AcknowledgeType::Accept, 1100).unwrap();
        // 0 and 1 are a contiguous acknowledged prefix → SPSO moves to 2.
        assert_eq!(p.start_offset(), 2);
    }

    #[test]
    fn spso_does_not_advance_past_a_gap() {
        let mut p = sp();
        p.acquire("m1", 3, 100, 1000);
        // Acknowledge offset 1 but not 0 — prefix is still blocked at 0.
        p.acknowledge("m1", 1, AcknowledgeType::Accept, 1100).unwrap();
        assert_eq!(p.start_offset(), 0);
        // Now ack 0 → SPSO jumps over both 0 and 1 to 2.
        p.acknowledge("m1", 0, AcknowledgeType::Accept, 1100).unwrap();
        assert_eq!(p.start_offset(), 2);
    }

    #[test]
    fn release_returns_record_to_available() {
        let mut p = sp();
        p.acquire("m1", 1, 100, 1000);
        p.acknowledge("m1", 0, AcknowledgeType::Release, 1100).unwrap();
        assert_eq!(p.available_count(), 1);
        assert_eq!(p.acquired_count(), 0);
        // Re-acquirable, delivery count climbs.
        let got = p.acquire("m2", 1, 100, 1200);
        assert_eq!(got, vec![0]);
        assert_eq!(p.delivery_count(0), 2);
    }

    #[test]
    fn reject_archives_record_and_advances_spso() {
        let mut p = sp();
        p.acquire("m1", 1, 100, 1000);
        p.acknowledge("m1", 0, AcknowledgeType::Reject, 1100).unwrap();
        assert_eq!(p.archived_count(), 0); // archived offset 0 was reclaimed by SPSO
        assert_eq!(p.start_offset(), 1);
    }

    #[test]
    fn acknowledge_requires_owning_member() {
        let mut p = sp();
        p.acquire("m1", 1, 100, 1000);
        let err = p.acknowledge("intruder", 0, AcknowledgeType::Accept, 1100);
        assert!(err.is_err());
    }

    #[test]
    fn acknowledge_unacquired_offset_errors() {
        let mut p = sp();
        let err = p.acknowledge("m1", 0, AcknowledgeType::Accept, 1100);
        assert!(err.is_err());
    }

    // ── delivery-count poison-pill guard ────────────────────────────────────

    #[test]
    fn record_archived_after_max_delivery_attempts() {
        let mut p = sp(); // MAX_DELIVERY = 3
        // attempt 1
        p.acquire("m1", 1, 100, 0);
        p.acknowledge("m1", 0, AcknowledgeType::Release, 0).unwrap();
        // attempt 2
        p.acquire("m1", 1, 100, 0);
        p.acknowledge("m1", 0, AcknowledgeType::Release, 0).unwrap();
        assert_eq!(p.delivery_count(0), 2);
        assert_eq!(p.available_count(), 1);
        // attempt 3 reaches max; release now archives instead of redelivering.
        p.acquire("m1", 1, 100, 0);
        assert_eq!(p.delivery_count(0), 3);
        p.acknowledge("m1", 0, AcknowledgeType::Release, 0).unwrap();
        // archived offset 0 is the SPSO prefix → reclaimed, nothing left.
        assert_eq!(p.start_offset(), 1);
        assert_eq!(p.available_count(), 0);
    }

    // ── acquisition-lock timeout ────────────────────────────────────────────

    #[test]
    fn expired_lock_releases_record() {
        let mut p = sp();
        p.acquire("m1", 1, 100, 1000); // lock expires at 1000+30000
        // Before expiry: still acquired.
        assert_eq!(p.release_expired_locks(2000), Vec::<u64>::new());
        assert_eq!(p.acquired_count(), 1);
        // After expiry: released back to available.
        assert_eq!(p.release_expired_locks(1000 + LOCK_MS), vec![0]);
        assert_eq!(p.available_count(), 1);
    }

    // ── share group registry / epoch fencing ────────────────────────────────

    #[test]
    fn join_bumps_epoch_and_registers_member() {
        let mut g = ShareGroup::new("orders");
        let e1 = g.join("m1");
        let e2 = g.join("m2");
        assert_eq!(e1, 1);
        assert_eq!(e2, 2);
        assert_eq!(g.member_count(), 2);
        g.check_epoch("m1", 1).unwrap();
    }

    #[test]
    fn stale_epoch_is_fenced() {
        let mut g = ShareGroup::new("orders");
        g.join("m1");
        g.join("m1"); // rejoin → epoch 2
        assert!(g.check_epoch("m1", 1).is_err());
        g.check_epoch("m1", 2).unwrap();
    }

    #[test]
    fn unknown_member_is_rejected() {
        let g = ShareGroup::new("orders");
        assert!(g.check_epoch("ghost", 1).is_err());
    }

    #[test]
    fn share_partition_is_per_topic_partition() {
        let mut g = ShareGroup::new("orders");
        let p = g.share_partition("t", 0, MAX_DELIVERY, LOCK_MS);
        let got = p.acquire("m1", 5, 3, 0);
        assert_eq!(got, vec![0, 1, 2]);
    }
}
