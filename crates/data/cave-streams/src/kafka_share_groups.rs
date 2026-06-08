// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KIP-932 "Queues for Kafka" — share groups.
//!
//! A faithful port of Apache Kafka 4.2.0's queue-style share-group subsystem:
//! the `RecordState`/`AcknowledgeType` state machines, the per-partition
//! in-flight record tracker (`SharePartition`) with its acquire / acknowledge
//! / lock-timeout / start-offset-advance behaviour, the thread-safe
//! `SharePartitionManager`, the `ShareGroup` membership + epoch machine, the
//! epoch-validated `ShareSession`, and the durable `ShareGroupOffset` /
//! `PersisterStateBatch` snapshot form written to `__share_group_state`.
//!
//! Upstream references:
//!   - `server/.../share/fetch/RecordState.java`        — Available=0,Acquired=1,Acknowledged=2,Archived=4
//!   - `clients/.../consumer/AcknowledgeType.java`      — Accept=1,Release=2,Reject=3,Renew=4
//!   - `core/.../share/SharePartition.java`
//!   - `core/.../share/SharePartitionManager.java`
//!   - `group-coordinator/.../modern/share/ShareGroup.java`
//!   - `server/.../share/session/ShareSession.java`
//!   - `share-coordinator/.../ShareGroupOffset.java`

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors raised by the share-group state machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareError {
    UnknownRecordState(u8),
    UnknownAcknowledgeType(u8),
    InvalidTransition {
        from: RecordState,
        to: RecordState,
        reason: &'static str,
    },
    InvalidRecordState {
        offset: i64,
        actual: RecordState,
    },
    NotAcquiredByMember {
        first_offset: i64,
        last_offset: i64,
        actual_member: String,
        requesting_member: String,
    },
    OffsetNotFound(i64),
    SessionEpochMismatch {
        expected: u32,
        actual: u32,
    },
    UnknownGroupMember(String),
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShareError::UnknownRecordState(id) => write!(f, "unknown record state id {id}"),
            ShareError::UnknownAcknowledgeType(id) => write!(f, "unknown acknowledge type id {id}"),
            ShareError::InvalidTransition { from, to, reason } => {
                write!(f, "invalid record-state transition {from:?} -> {to:?} ({reason})")
            }
            ShareError::InvalidRecordState { offset, actual } => {
                write!(f, "record at offset {offset} is in state {actual:?}, not Acquired")
            }
            ShareError::NotAcquiredByMember {
                first_offset,
                last_offset,
                actual_member,
                requesting_member,
            } => write!(
                f,
                "batch [{first_offset},{last_offset}] acquired by {actual_member}, not {requesting_member}"
            ),
            ShareError::OffsetNotFound(o) => write!(f, "no in-flight batch at offset {o}"),
            ShareError::SessionEpochMismatch { expected, actual } => {
                write!(f, "share-session epoch mismatch: expected {expected}, got {actual}")
            }
            ShareError::UnknownGroupMember(m) => write!(f, "unknown share-group member {m}"),
        }
    }
}

impl std::error::Error for ShareError {}

/// Result alias for the share-group subsystem.
pub type ShareResult<T> = Result<T, ShareError>;

// ── RecordState ──────────────────────────────────────────────────────────────

/// State of an individual in-flight record (KIP-932 `RecordState`).
///
/// The byte ids are wire-stable and deliberately non-contiguous: `Archived` is
/// **4**, not 3 — upstream skips id 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordState {
    Available,
    Acquired,
    Acknowledged,
    Archived,
}

impl RecordState {
    /// Wire-stable byte id.
    pub fn id(self) -> u8 {
        match self {
            RecordState::Available => 0,
            RecordState::Acquired => 1,
            RecordState::Acknowledged => 2,
            RecordState::Archived => 4,
        }
    }

    /// Parse a byte id; id 3 and any other unknown value error.
    pub fn from_id(id: u8) -> ShareResult<RecordState> {
        match id {
            0 => Ok(RecordState::Available),
            1 => Ok(RecordState::Acquired),
            2 => Ok(RecordState::Acknowledged),
            4 => Ok(RecordState::Archived),
            other => Err(ShareError::UnknownRecordState(other)),
        }
    }

    /// `Acknowledged` and `Archived` are terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, RecordState::Acknowledged | RecordState::Archived)
    }

    /// Validate a state transition, mirroring `RecordState.validateTransition`:
    /// same-state is rejected; terminals reject everything; `Available` may go
    /// only to `Acquired`; otherwise the transition is allowed.
    pub fn validate_transition(self, to: RecordState) -> ShareResult<RecordState> {
        if self == to {
            return Err(ShareError::InvalidTransition {
                from: self,
                to,
                reason: "same state",
            });
        }
        if self.is_terminal() {
            return Err(ShareError::InvalidTransition {
                from: self,
                to,
                reason: "terminal",
            });
        }
        if self == RecordState::Available && to != RecordState::Acquired {
            return Err(ShareError::InvalidTransition {
                from: self,
                to,
                reason: "Available -> only Acquired",
            });
        }
        Ok(to)
    }
}

// ── AcknowledgeType ──────────────────────────────────────────────────────────

/// Client acknowledgement disposition (KIP-932 `AcknowledgeType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgeType {
    Accept,
    Release,
    Reject,
    Renew,
}

impl AcknowledgeType {
    pub fn id(self) -> u8 {
        match self {
            AcknowledgeType::Accept => 1,
            AcknowledgeType::Release => 2,
            AcknowledgeType::Reject => 3,
            AcknowledgeType::Renew => 4,
        }
    }

    pub fn from_id(id: u8) -> ShareResult<AcknowledgeType> {
        match id {
            1 => Ok(AcknowledgeType::Accept),
            2 => Ok(AcknowledgeType::Release),
            3 => Ok(AcknowledgeType::Reject),
            4 => Ok(AcknowledgeType::Renew),
            other => Err(ShareError::UnknownAcknowledgeType(other)),
        }
    }
}

// ── In-flight records ────────────────────────────────────────────────────────

/// Mutable per-batch delivery state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightState {
    pub state: RecordState,
    pub delivery_count: u32,
    pub member_id: String,
    pub lock_expires_at_ms: Option<u64>,
}

impl InFlightState {
    pub fn new_available() -> Self {
        Self {
            state: RecordState::Available,
            delivery_count: 0,
            member_id: String::new(),
            lock_expires_at_ms: None,
        }
    }
}

/// An inclusive `[first_offset, last_offset]` run of records sharing one state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightBatch {
    pub first_offset: i64,
    pub last_offset: i64,
    pub batch_state: InFlightState,
}

impl InFlightBatch {
    pub fn new(first_offset: i64, last_offset: i64) -> Self {
        Self {
            first_offset,
            last_offset,
            batch_state: InFlightState::new_available(),
        }
    }

    /// Inclusive record count, clamped at zero.
    pub fn record_count(&self) -> u64 {
        (self.last_offset - self.first_offset + 1).max(0) as u64
    }

    pub fn contains(&self, offset: i64) -> bool {
        self.first_offset <= offset && offset <= self.last_offset
    }
}

/// Records handed to a consumer by [`SharePartition::acquire`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquiredRecords {
    pub first_offset: i64,
    pub last_offset: i64,
    pub delivery_count: u32,
}

impl AcquiredRecords {
    pub fn record_count(&self) -> u64 {
        (self.last_offset - self.first_offset + 1).max(0) as u64
    }
}

// ── Durable snapshot form ────────────────────────────────────────────────────

/// One persisted batch row in the `__share_group_state` blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersisterStateBatch {
    pub first_offset: i64,
    pub last_offset: i64,
    pub delivery_count: u32,
    pub state: RecordState,
}

/// Durable per-`(group, topic, partition)` snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareGroupOffset {
    pub group_id: String,
    pub topic: String,
    pub partition: i32,
    pub start_offset: i64,
    pub state_epoch: u32,
    pub leader_epoch: i32,
    pub batches: Vec<PersisterStateBatch>,
}

// ── SharePartition ───────────────────────────────────────────────────────────

/// Per-`(group, topic, partition)` in-flight record tracker.
///
/// Batches are stored in a `BTreeMap` keyed by `first_offset`, so iteration is
/// always ascending. Records are acquired in whole batches (never split).
#[derive(Debug)]
pub struct SharePartition {
    group_id: String,
    topic: String,
    partition: i32,
    start_offset: i64,
    next_fetch_offset: i64,
    batches: BTreeMap<i64, InFlightBatch>,
    max_delivery_count: u32,
    record_lock_duration_ms: u64,
    state_epoch: u32,
}

impl SharePartition {
    pub fn new(
        group_id: impl Into<String>,
        topic: impl Into<String>,
        partition: i32,
        start_offset: i64,
    ) -> Self {
        Self {
            group_id: group_id.into(),
            topic: topic.into(),
            partition,
            start_offset,
            next_fetch_offset: start_offset,
            batches: BTreeMap::new(),
            max_delivery_count: 5,
            record_lock_duration_ms: 30_000,
            state_epoch: 0,
        }
    }

    pub fn with_max_delivery_count(mut self, n: u32) -> Self {
        self.max_delivery_count = n;
        self
    }

    pub fn with_record_lock_duration_ms(mut self, ms: u64) -> Self {
        self.record_lock_duration_ms = ms;
        self
    }

    pub fn start_offset(&self) -> i64 {
        self.start_offset
    }

    pub fn next_fetch_offset(&self) -> i64 {
        self.next_fetch_offset
    }

    pub fn state_epoch(&self) -> u32 {
        self.state_epoch
    }

    /// Ascending clone of all in-flight batches (introspection / test aid).
    pub fn batches_snapshot(&self) -> Vec<InFlightBatch> {
        self.batches.values().cloned().collect()
    }

    pub fn batch_state(&self, first_offset: i64) -> Option<RecordState> {
        self.batches.get(&first_offset).map(|b| b.batch_state.state)
    }

    /// Acquire up to `max_records` records for `member_id`.
    ///
    /// First sweep: re-acquire whole `Available` batches at/after `start_offset`
    /// that are still under the delivery cap, in ascending order, never
    /// splitting a batch — if a batch would overshoot the budget the sweep
    /// stops. Second sweep: allocate one fresh batch at `next_fetch_offset`
    /// for the remaining budget.
    pub fn acquire(&mut self, member_id: &str, max_records: u64, now_ms: u64) -> Vec<AcquiredRecords> {
        let mut out = Vec::new();
        if max_records == 0 {
            return out;
        }
        let deadline = now_ms.saturating_add(self.record_lock_duration_ms);
        let mut acquired_total: u64 = 0;

        // First sweep — re-acquire reusable Available batches.
        let keys: Vec<i64> = self.batches.range(self.start_offset..).map(|(k, _)| *k).collect();
        for k in keys {
            let (rc, reusable) = {
                let b = &self.batches[&k];
                (
                    b.record_count(),
                    b.batch_state.state == RecordState::Available
                        && b.batch_state.delivery_count < self.max_delivery_count,
                )
            };
            if !reusable {
                continue;
            }
            if acquired_total + rc > max_records {
                break; // batches are never split
            }
            let b = self.batches.get_mut(&k).unwrap();
            b.batch_state.state = RecordState::Acquired;
            b.batch_state.delivery_count += 1;
            b.batch_state.member_id = member_id.to_string();
            b.batch_state.lock_expires_at_ms = Some(deadline);
            acquired_total += rc;
            out.push(AcquiredRecords {
                first_offset: b.first_offset,
                last_offset: b.last_offset,
                delivery_count: b.batch_state.delivery_count,
            });
        }

        // Second sweep — allocate one fresh batch for the remaining budget.
        let remaining = max_records - acquired_total;
        if remaining > 0 {
            let first = self.next_fetch_offset;
            let last = first + remaining as i64 - 1;
            let mut st = InFlightState::new_available();
            st.state = RecordState::Acquired;
            st.delivery_count = 1;
            st.member_id = member_id.to_string();
            st.lock_expires_at_ms = Some(deadline);
            self.batches.insert(
                first,
                InFlightBatch {
                    first_offset: first,
                    last_offset: last,
                    batch_state: st,
                },
            );
            self.next_fetch_offset = last + 1;
            out.push(AcquiredRecords {
                first_offset: first,
                last_offset: last,
                delivery_count: 1,
            });
        }
        out
    }

    /// Acknowledge a previously-acquired batch.
    ///
    /// Validation order is exactly: first-offset → last-offset → state →
    /// member, matching upstream.
    pub fn acknowledge(
        &mut self,
        member_id: &str,
        first_offset: i64,
        last_offset: i64,
        ack: AcknowledgeType,
        now_ms: u64,
    ) -> ShareResult<()> {
        {
            let b = self
                .batches
                .get(&first_offset)
                .ok_or(ShareError::OffsetNotFound(first_offset))?;
            if b.last_offset != last_offset {
                return Err(ShareError::OffsetNotFound(last_offset));
            }
            if b.batch_state.state != RecordState::Acquired {
                return Err(ShareError::InvalidRecordState {
                    offset: first_offset,
                    actual: b.batch_state.state,
                });
            }
            if b.batch_state.member_id != member_id {
                return Err(ShareError::NotAcquiredByMember {
                    first_offset,
                    last_offset,
                    actual_member: b.batch_state.member_id.clone(),
                    requesting_member: member_id.to_string(),
                });
            }
        }

        let max = self.max_delivery_count;
        let dur = self.record_lock_duration_ms;
        let b = self.batches.get_mut(&first_offset).unwrap();
        match ack {
            AcknowledgeType::Accept => {
                b.batch_state.state.validate_transition(RecordState::Acknowledged)?;
                b.batch_state.state = RecordState::Acknowledged;
                b.batch_state.lock_expires_at_ms = None;
            }
            AcknowledgeType::Release => {
                b.batch_state.state.validate_transition(RecordState::Available)?;
                b.batch_state.state = RecordState::Available;
                b.batch_state.lock_expires_at_ms = None;
                b.batch_state.member_id = String::new();
                // Auto-archive once the delivery cap is reached.
                if b.batch_state.delivery_count >= max {
                    b.batch_state.state = RecordState::Archived;
                }
            }
            AcknowledgeType::Reject => {
                b.batch_state.state.validate_transition(RecordState::Archived)?;
                b.batch_state.state = RecordState::Archived;
                b.batch_state.lock_expires_at_ms = None;
            }
            AcknowledgeType::Renew => {
                // Extend the acquisition lock only; state/member unchanged.
                b.batch_state.lock_expires_at_ms = Some(now_ms.saturating_add(dur));
            }
        }
        Ok(())
    }

    /// Release every `Acquired` batch whose lock deadline is `<= now_ms`
    /// (inclusive). Returns clones of the flipped batches.
    pub fn sweep_expired_locks(&mut self, now_ms: u64) -> Vec<InFlightBatch> {
        let max = self.max_delivery_count;
        let mut flipped = Vec::new();
        for b in self.batches.values_mut() {
            if b.batch_state.state != RecordState::Acquired {
                continue;
            }
            if let Some(deadline) = b.batch_state.lock_expires_at_ms {
                if deadline <= now_ms {
                    b.batch_state.state = RecordState::Available;
                    b.batch_state.lock_expires_at_ms = None;
                    b.batch_state.member_id = String::new();
                    if b.batch_state.delivery_count >= max {
                        b.batch_state.state = RecordState::Archived;
                    }
                    flipped.push(b.clone());
                }
            }
        }
        flipped
    }

    /// Advance the share-partition start offset. Drops fully-stale batches
    /// (`last_offset < new_start`) without splitting partial overlaps, and
    /// bumps `state_epoch`. A non-advancing call is a no-op.
    pub fn move_start_offset(&mut self, new_start: i64) {
        if new_start <= self.start_offset {
            return;
        }
        self.start_offset = new_start;
        self.batches.retain(|_, b| b.last_offset >= new_start);
        if self.next_fetch_offset < new_start {
            self.next_fetch_offset = new_start;
        }
        self.state_epoch += 1;
    }

    /// Produce the durable `ShareGroupOffset` snapshot.
    pub fn snapshot(&self) -> ShareGroupOffset {
        let batches = self
            .batches
            .values()
            .map(|b| PersisterStateBatch {
                first_offset: b.first_offset,
                last_offset: b.last_offset,
                delivery_count: b.batch_state.delivery_count,
                state: b.batch_state.state,
            })
            .collect();
        ShareGroupOffset {
            group_id: self.group_id.clone(),
            topic: self.topic.clone(),
            partition: self.partition,
            start_offset: self.start_offset,
            state_epoch: self.state_epoch,
            leader_epoch: 0,
            batches,
        }
    }
}

// ── SharePartitionManager ────────────────────────────────────────────────────

/// Identity of a share partition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SharePartitionKey {
    pub group_id: String,
    pub topic: String,
    pub partition: i32,
}

impl SharePartitionKey {
    pub fn new(group_id: impl Into<String>, topic: impl Into<String>, partition: i32) -> Self {
        Self {
            group_id: group_id.into(),
            topic: topic.into(),
            partition,
        }
    }
}

/// Thread-safe registry of share partitions.
#[derive(Default)]
pub struct SharePartitionManager {
    partitions: Mutex<HashMap<SharePartitionKey, SharePartition>>,
}

impl SharePartitionManager {
    /// Create the share partition for `key` (with `start_offset`) if absent.
    pub fn get_or_create(&self, key: SharePartitionKey, start_offset: i64) {
        let mut g = self.partitions.lock().unwrap();
        g.entry(key.clone()).or_insert_with(|| {
            SharePartition::new(key.group_id.clone(), key.topic.clone(), key.partition, start_offset)
        });
    }

    pub fn len(&self) -> usize {
        self.partitions.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.partitions.lock().unwrap().is_empty()
    }

    /// Mutably operate on a partition; returns `None` if the key is unknown.
    pub fn with<R>(&self, key: &SharePartitionKey, f: impl FnOnce(&mut SharePartition) -> R) -> Option<R> {
        let mut g = self.partitions.lock().unwrap();
        g.get_mut(key).map(f)
    }

    /// Read-only access to a partition; returns `None` if unknown.
    pub fn read<R>(&self, key: &SharePartitionKey, f: impl FnOnce(&SharePartition) -> R) -> Option<R> {
        let g = self.partitions.lock().unwrap();
        g.get(key).map(f)
    }

    /// Sweep expired locks across every partition; returns the total flipped.
    pub fn tick_sweep(&self, now_ms: u64) -> usize {
        let mut g = self.partitions.lock().unwrap();
        g.values_mut().map(|sp| sp.sweep_expired_locks(now_ms).len()).sum()
    }

    /// Durable snapshot of every partition.
    pub fn snapshot(&self) -> Vec<ShareGroupOffset> {
        let g = self.partitions.lock().unwrap();
        g.values().map(|sp| sp.snapshot()).collect()
    }
}

// ── ShareGroup membership ────────────────────────────────────────────────────

/// Lifecycle state of a share-group member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberState {
    Joining,
    Stable,
    Left,
}

/// A share-group member.
#[derive(Debug, Clone)]
pub struct ShareGroupMember {
    pub id: String,
    pub state: MemberState,
    pub session_timeout_ms: u32,
    pub session_epoch: u32,
}

impl ShareGroupMember {
    pub fn new(id: impl Into<String>, session_timeout_ms: u32) -> Self {
        Self {
            id: id.into(),
            state: MemberState::Joining,
            session_timeout_ms,
            session_epoch: 0,
        }
    }
}

/// KIP-932 share-group configuration defaults.
#[derive(Debug, Clone)]
pub struct ShareGroupConfig {
    pub record_lock_duration_ms: u64,
    pub delivery_count_limit: u32,
    pub session_timeout_ms: u32,
}

impl Default for ShareGroupConfig {
    fn default() -> Self {
        Self {
            record_lock_duration_ms: 30_000,
            delivery_count_limit: 5,
            session_timeout_ms: 45_000,
        }
    }
}

/// Share-group membership + epoch state machine.
///
/// `group_epoch` advances on successful `join`/`leave`; `stabilise` and
/// `bump_session_epoch` do not move it. `session_epoch` is per-member.
#[derive(Debug)]
pub struct ShareGroup {
    pub id: String,
    members: BTreeMap<String, ShareGroupMember>,
    config: ShareGroupConfig,
    group_epoch: u32,
}

impl ShareGroup {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            members: BTreeMap::new(),
            config: ShareGroupConfig::default(),
            group_epoch: 0,
        }
    }

    pub fn with_config(id: impl Into<String>, config: ShareGroupConfig) -> Self {
        Self {
            id: id.into(),
            members: BTreeMap::new(),
            config,
            group_epoch: 0,
        }
    }

    pub fn group_epoch(&self) -> u32 {
        self.group_epoch
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    pub fn member_state(&self, id: &str) -> Option<MemberState> {
        self.members.get(id).map(|m| m.state)
    }

    pub fn members(&self) -> Vec<ShareGroupMember> {
        self.members.values().cloned().collect()
    }

    /// Add a member in `Joining` state and bump `group_epoch`.
    pub fn join(&mut self, id: impl Into<String>) -> u32 {
        let id = id.into();
        self.members
            .insert(id.clone(), ShareGroupMember::new(id, self.config.session_timeout_ms));
        self.group_epoch += 1;
        self.group_epoch
    }

    /// Mark a member `Stable`; does not bump `group_epoch`.
    pub fn stabilise(&mut self, id: &str) -> ShareResult<()> {
        let m = self
            .members
            .get_mut(id)
            .ok_or_else(|| ShareError::UnknownGroupMember(id.to_string()))?;
        m.state = MemberState::Stable;
        Ok(())
    }

    /// Remove a member and bump `group_epoch`. Unknown member → error, no bump.
    pub fn leave(&mut self, id: &str) -> ShareResult<u32> {
        if self.members.remove(id).is_none() {
            return Err(ShareError::UnknownGroupMember(id.to_string()));
        }
        self.group_epoch += 1;
        Ok(self.group_epoch)
    }

    /// Increment a member's session epoch; independent of `group_epoch`.
    pub fn bump_session_epoch(&mut self, id: &str) -> ShareResult<u32> {
        let m = self
            .members
            .get_mut(id)
            .ok_or_else(|| ShareError::UnknownGroupMember(id.to_string()))?;
        m.session_epoch += 1;
        Ok(m.session_epoch)
    }
}

// ── ShareSession ─────────────────────────────────────────────────────────────

/// Per-connection share-fetch session with an epoch that advances one step at
/// a time (KIP-932 share-session epoch).
#[derive(Debug)]
pub struct ShareSession {
    pub group_id: String,
    pub member_id: String,
    pub connection_id: String,
    epoch: u32,
    partitions: BTreeSet<SharePartitionKey>,
}

impl ShareSession {
    pub fn new(
        group_id: impl Into<String>,
        member_id: impl Into<String>,
        connection_id: impl Into<String>,
    ) -> Self {
        Self {
            group_id: group_id.into(),
            member_id: member_id.into(),
            connection_id: connection_id.into(),
            epoch: 0,
            partitions: BTreeSet::new(),
        }
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Advance the session epoch; `incoming_epoch` must equal the current epoch.
    pub fn advance(&mut self, incoming_epoch: u32) -> ShareResult<u32> {
        if incoming_epoch != self.epoch {
            return Err(ShareError::SessionEpochMismatch {
                expected: self.epoch,
                actual: incoming_epoch,
            });
        }
        self.epoch = self.epoch.wrapping_add(1);
        Ok(self.epoch)
    }

    pub fn add_partition(&mut self, key: SharePartitionKey) {
        self.partitions.insert(key);
    }

    pub fn remove_partition(&mut self, key: &SharePartitionKey) -> bool {
        self.partitions.remove(key)
    }

    pub fn partition_count(&self) -> usize {
        self.partitions.len()
    }

    pub fn partitions(&self) -> Vec<SharePartitionKey> {
        self.partitions.iter().cloned().collect()
    }
}
