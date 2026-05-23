// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Kafka Share Groups — deep-port of KIP-932 from Apache Kafka 4.2.0.
//!
//! Apache Kafka 4.2.0 introduced *share groups* as a queue-style consumption
//! pattern that lets multiple consumers in the same group concurrently read
//! and acknowledge records on the same partition (in contrast to classical
//! consumer groups, which assign each partition to a single consumer).
//!
//! Upstream layout this module honours:
//!
//! * `server/.../share/fetch/RecordState.java` — Available / Acquired /
//!   Acknowledged / Archived + `validateTransition`
//! * `server/.../share/fetch/InFlightBatch.java` + `InFlightState.java` —
//!   per-batch state, delivery count, member id, acquisition lock timer
//! * `clients/.../consumer/AcknowledgeType.java` — Accept / Release /
//!   Reject / Renew + byte ids
//! * `core/.../server/share/SharePartition.java` — per-(group, topic-partition)
//!   acquire / acknowledge / release / lock-timeout flow
//! * `core/.../server/share/SharePartitionManager.java` — coordinator-side
//!   map of `SharePartitionKey → SharePartition`
//! * `group-coordinator/.../modern/share/ShareGroup.java` — member + epoch
//!   state machine
//! * `server/.../share/session/ShareSession.java` — share-fetch session
//!   epoch (initial 0, incremented per fetch round)
//! * `share-coordinator/.../ShareGroupOffset.java` +
//!   `server-common/.../share/persister/PersisterStateBatch.java` — the
//!   durable per-(group, topic-partition) state-batch list

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

// ─── RecordState ─────────────────────────────────────────────────────────────

/// Mirrors `RecordState.java` ids (Available=0, Acquired=1, Acknowledged=2,
/// Archived=4 — note 3 is intentionally skipped in upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordState {
    Available,
    Acquired,
    Acknowledged,
    Archived,
}

impl RecordState {
    pub fn id(self) -> u8 {
        match self {
            RecordState::Available => 0,
            RecordState::Acquired => 1,
            RecordState::Acknowledged => 2,
            RecordState::Archived => 4,
        }
    }

    pub fn from_id(id: u8) -> Result<RecordState, ShareError> {
        match id {
            0 => Ok(RecordState::Available),
            1 => Ok(RecordState::Acquired),
            2 => Ok(RecordState::Acknowledged),
            4 => Ok(RecordState::Archived),
            other => Err(ShareError::UnknownRecordState(other)),
        }
    }

    /// Upstream parity: `RecordState.validateTransition`.
    ///   * Terminal states (ACKNOWLEDGED / ARCHIVED) cannot transition.
    ///   * AVAILABLE → only ACQUIRED.
    ///   * ACQUIRED → AVAILABLE / ACKNOWLEDGED / ARCHIVED.
    ///   * Transitioning to the same state is rejected.
    pub fn validate_transition(self, new_state: RecordState) -> Result<RecordState, ShareError> {
        if self == new_state {
            return Err(ShareError::InvalidTransition {
                from: self,
                to: new_state,
                reason: "same state",
            });
        }
        if self == RecordState::Acknowledged || self == RecordState::Archived {
            return Err(ShareError::InvalidTransition {
                from: self,
                to: new_state,
                reason: "terminal",
            });
        }
        if self == RecordState::Available && new_state != RecordState::Acquired {
            return Err(ShareError::InvalidTransition {
                from: self,
                to: new_state,
                reason: "Available → only Acquired",
            });
        }
        Ok(new_state)
    }
}

// ─── AcknowledgeType ─────────────────────────────────────────────────────────

/// Mirrors `clients/.../consumer/AcknowledgeType.java` ids:
/// Accept=1, Release=2, Reject=3, Renew=4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AcknowledgeType {
    /// Successful consume — record transitions to ACKNOWLEDGED (terminal).
    Accept,
    /// Re-deliver — record transitions back to AVAILABLE so any consumer
    /// in the group may re-acquire it.
    Release,
    /// Permanent failure — record transitions to ARCHIVED (terminal).
    Reject,
    /// Extend the acquisition lock; record stays ACQUIRED by the same member.
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

    pub fn from_id(id: u8) -> Result<AcknowledgeType, ShareError> {
        match id {
            1 => Ok(AcknowledgeType::Accept),
            2 => Ok(AcknowledgeType::Release),
            3 => Ok(AcknowledgeType::Reject),
            4 => Ok(AcknowledgeType::Renew),
            other => Err(ShareError::UnknownAcknowledgeType(other)),
        }
    }
}

// ─── Errors ──────────────────────────────────────────────────────────────────

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
            ShareError::UnknownRecordState(b) => write!(f, "unknown record state id {b}"),
            ShareError::UnknownAcknowledgeType(b) => write!(f, "unknown ack type id {b}"),
            ShareError::InvalidTransition { from, to, reason } => {
                write!(f, "invalid record-state transition {:?} → {:?}: {}", from, to, reason)
            }
            ShareError::InvalidRecordState { offset, actual } => {
                write!(f, "invalid record state at offset {offset}: {actual:?}")
            }
            ShareError::NotAcquiredByMember {
                first_offset,
                last_offset,
                actual_member,
                requesting_member,
            } => write!(
                f,
                "batch [{first_offset},{last_offset}] acquired by `{actual_member}`, not `{requesting_member}`"
            ),
            ShareError::OffsetNotFound(o) => write!(f, "offset {o} not found in in-flight batches"),
            ShareError::SessionEpochMismatch { expected, actual } => {
                write!(f, "share-session epoch mismatch expected={expected} got={actual}")
            }
            ShareError::UnknownGroupMember(m) => write!(f, "unknown share-group member `{m}`"),
        }
    }
}

impl std::error::Error for ShareError {}

pub type ShareResult<T> = Result<T, ShareError>;

// ─── InFlightState + InFlightBatch ──────────────────────────────────────────

/// Mirrors `server/.../share/fetch/InFlightState.java`.  Stored *inside*
/// `InFlightBatch` rather than as a separate object because the upstream
/// Java class is tightly coupled to the batch envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightState {
    pub state: RecordState,
    pub delivery_count: u32,
    pub member_id: String,
    /// `acquisitionLockTimeoutTask` analogue — the wall-clock millis at
    /// which the acquisition lock expires.  `None` outside the
    /// `Acquired` state.
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

/// Mirrors `InFlightBatch.java` — a contiguous half-open offset range whose
/// records share a single `InFlightState`.  Boundaries are inclusive on both
/// ends to match upstream `firstOffset`/`lastOffset`.
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

    pub fn record_count(&self) -> u64 {
        (self.last_offset - self.first_offset + 1).max(0) as u64
    }

    pub fn contains(&self, offset: i64) -> bool {
        self.first_offset <= offset && offset <= self.last_offset
    }
}

// ─── SharePartition ──────────────────────────────────────────────────────────

/// Per-(share-group, topic-partition) state machine.
///
/// Mirrors `core/.../server/share/SharePartition.java`, simplified to the
/// state-machine + acquire/acknowledge/release contract.  The persistence
/// boundary (writes to the share-coordinator __share_group_state topic) is
/// abstracted as a `PersisterStateBatch` snapshot.
pub struct SharePartition {
    group_id: String,
    topic: String,
    partition: i32,
    /// Records below this offset are no longer tracked (the partition's
    /// log-start has moved past them, equivalent to upstream `startOffset`).
    start_offset: i64,
    /// First not-yet-fetched offset.
    next_fetch_offset: i64,
    /// In-flight batches keyed by `first_offset`.
    batches: Mutex<BTreeMap<i64, InFlightBatch>>,
    /// Upper bound on the delivery count before a record is auto-archived
    /// (KIP-932 default: 5).
    max_delivery_count: u32,
    /// Per-batch acquisition-lock duration (millis).
    record_lock_duration_ms: u64,
    /// `stateEpoch` — increments every time we write a snapshot to the
    /// share-coordinator; the start_offset is fenced behind this epoch.
    state_epoch: Mutex<u32>,
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
            batches: Mutex::new(BTreeMap::new()),
            max_delivery_count: 5,
            record_lock_duration_ms: 30_000,
            state_epoch: Mutex::new(0),
        }
    }

    pub fn with_max_delivery_count(mut self, max: u32) -> Self {
        self.max_delivery_count = max;
        self
    }

    pub fn with_record_lock_duration_ms(mut self, ms: u64) -> Self {
        self.record_lock_duration_ms = ms;
        self
    }

    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn partition(&self) -> i32 {
        self.partition
    }

    pub fn start_offset(&self) -> i64 {
        self.start_offset
    }

    pub fn next_fetch_offset(&self) -> i64 {
        self.next_fetch_offset
    }

    pub fn state_epoch(&self) -> u32 {
        *self.state_epoch.lock().unwrap()
    }

    pub fn max_delivery_count(&self) -> u32 {
        self.max_delivery_count
    }

    /// Try to acquire up to `max_records` records starting from
    /// `next_fetch_offset`.  Newly-introduced batches become ACQUIRED for
    /// `member_id`; previously AVAILABLE batches (re-released or
    /// re-eligible after lock expiry) are re-acquired.
    pub fn acquire(
        &mut self,
        member_id: &str,
        max_records: u64,
        now_ms: u64,
    ) -> ShareResult<Vec<AcquiredRecords>> {
        if max_records == 0 {
            return Ok(Vec::new());
        }
        // First sweep — re-acquire any AVAILABLE batches above start_offset.
        let mut reacquired_total: u64 = 0;
        let mut acquired = Vec::new();
        {
            let mut map = self.batches.lock().unwrap();
            for (_first, batch) in map.range_mut(self.start_offset..) {
                if batch.batch_state.state == RecordState::Available
                    && batch.batch_state.delivery_count < self.max_delivery_count
                {
                    let take = batch.record_count();
                    if reacquired_total + take > max_records {
                        // Don't split batches here — upstream lets the next
                        // share-fetch round pick up the remainder.
                        break;
                    }
                    batch.batch_state.state = RecordState::Acquired;
                    batch.batch_state.delivery_count += 1;
                    batch.batch_state.member_id = member_id.to_string();
                    batch.batch_state.lock_expires_at_ms =
                        Some(now_ms.saturating_add(self.record_lock_duration_ms));
                    reacquired_total += take;
                    acquired.push(AcquiredRecords {
                        first_offset: batch.first_offset,
                        last_offset: batch.last_offset,
                        delivery_count: batch.batch_state.delivery_count,
                    });
                }
            }
        }

        // Second sweep — introduce a new batch starting at `next_fetch_offset`
        // for the remaining budget (one synthetic record per fetch — upstream
        // SharePartitionManager calls back into the log fetcher; this port
        // gives callers a deterministic single-batch hand-off).
        let remaining = max_records - reacquired_total;
        if remaining > 0 {
            let first = self.next_fetch_offset;
            let last = first + (remaining as i64) - 1;
            let mut batch = InFlightBatch::new(first, last);
            batch.batch_state.state = RecordState::Acquired;
            batch.batch_state.delivery_count = 1;
            batch.batch_state.member_id = member_id.to_string();
            batch.batch_state.lock_expires_at_ms =
                Some(now_ms.saturating_add(self.record_lock_duration_ms));
            self.batches.lock().unwrap().insert(first, batch);
            acquired.push(AcquiredRecords {
                first_offset: first,
                last_offset: last,
                delivery_count: 1,
            });
            self.next_fetch_offset = last + 1;
        }
        Ok(acquired)
    }

    /// Acknowledge a batch.
    ///   * `Accept`  → Acquired → Acknowledged (terminal)
    ///   * `Release` → Acquired → Available (delivery count NOT bumped here;
    ///     the next acquire() increments)
    ///   * `Reject`  → Acquired → Archived (terminal)
    ///   * `Renew`   → Acquired → Acquired with refreshed lock_expires_at_ms
    pub fn acknowledge(
        &self,
        member_id: &str,
        first_offset: i64,
        last_offset: i64,
        ack_type: AcknowledgeType,
        now_ms: u64,
    ) -> ShareResult<()> {
        let mut map = self.batches.lock().unwrap();
        let batch = map
            .get_mut(&first_offset)
            .ok_or(ShareError::OffsetNotFound(first_offset))?;
        if batch.last_offset != last_offset {
            return Err(ShareError::OffsetNotFound(last_offset));
        }
        if batch.batch_state.state != RecordState::Acquired {
            return Err(ShareError::InvalidRecordState {
                offset: first_offset,
                actual: batch.batch_state.state,
            });
        }
        if batch.batch_state.member_id != member_id {
            return Err(ShareError::NotAcquiredByMember {
                first_offset,
                last_offset,
                actual_member: batch.batch_state.member_id.clone(),
                requesting_member: member_id.to_string(),
            });
        }
        match ack_type {
            AcknowledgeType::Accept => {
                batch
                    .batch_state
                    .state
                    .validate_transition(RecordState::Acknowledged)?;
                batch.batch_state.state = RecordState::Acknowledged;
                batch.batch_state.lock_expires_at_ms = None;
            }
            AcknowledgeType::Release => {
                batch
                    .batch_state
                    .state
                    .validate_transition(RecordState::Available)?;
                batch.batch_state.state = RecordState::Available;
                batch.batch_state.lock_expires_at_ms = None;
                batch.batch_state.member_id.clear();
                // Auto-archive if delivery cap hit.
                if batch.batch_state.delivery_count >= self.max_delivery_count {
                    batch.batch_state.state = RecordState::Archived;
                }
            }
            AcknowledgeType::Reject => {
                batch
                    .batch_state
                    .state
                    .validate_transition(RecordState::Archived)?;
                batch.batch_state.state = RecordState::Archived;
                batch.batch_state.lock_expires_at_ms = None;
            }
            AcknowledgeType::Renew => {
                batch.batch_state.lock_expires_at_ms =
                    Some(now_ms.saturating_add(self.record_lock_duration_ms));
            }
        }
        Ok(())
    }

    /// Sweep acquisition locks that have expired at `now_ms`; expired
    /// Acquired batches transition back to Available so other members in
    /// the group can re-acquire them.  Returns the batches that flipped.
    pub fn sweep_expired_locks(&self, now_ms: u64) -> Vec<InFlightBatch> {
        let mut released = Vec::new();
        let mut map = self.batches.lock().unwrap();
        for batch in map.values_mut() {
            if batch.batch_state.state != RecordState::Acquired {
                continue;
            }
            if let Some(deadline) = batch.batch_state.lock_expires_at_ms {
                if deadline <= now_ms {
                    batch.batch_state.state = RecordState::Available;
                    batch.batch_state.lock_expires_at_ms = None;
                    batch.batch_state.member_id.clear();
                    if batch.batch_state.delivery_count >= self.max_delivery_count {
                        batch.batch_state.state = RecordState::Archived;
                    }
                    released.push(batch.clone());
                }
            }
        }
        released
    }

    /// Advance `start_offset` (caller has observed the partition log-start
    /// moved past `new_start`).  Trim every batch whose `last_offset <
    /// new_start`.  This bumps `state_epoch` to fence stale persister
    /// reads.
    pub fn move_start_offset(&mut self, new_start: i64) {
        if new_start <= self.start_offset {
            return;
        }
        self.start_offset = new_start;
        let mut map = self.batches.lock().unwrap();
        let stale: Vec<i64> = map
            .iter()
            .filter(|(_, b)| b.last_offset < new_start)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            map.remove(&k);
        }
        if self.next_fetch_offset < new_start {
            self.next_fetch_offset = new_start;
        }
        *self.state_epoch.lock().unwrap() += 1;
    }

    /// Snapshot the partition's in-flight state as a list of
    /// `PersisterStateBatch` rows — the durable form that the
    /// `share-coordinator/.../ShareGroupOffset.java` writes to
    /// `__share_group_state`.
    pub fn snapshot(&self) -> ShareGroupOffset {
        let map = self.batches.lock().unwrap();
        let batches: Vec<PersisterStateBatch> = map
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
            state_epoch: *self.state_epoch.lock().unwrap(),
            leader_epoch: 0,
            batches,
        }
    }

    /// Get a copy of every batch, sorted by `first_offset`.
    pub fn batches_snapshot(&self) -> Vec<InFlightBatch> {
        self.batches.lock().unwrap().values().cloned().collect()
    }
}

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

// ─── Persister types ─────────────────────────────────────────────────────────

/// Mirrors `server-common/.../share/persister/PersisterStateBatch.java`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersisterStateBatch {
    pub first_offset: i64,
    pub last_offset: i64,
    pub delivery_count: u32,
    pub state: RecordState,
}

/// Mirrors `share-coordinator/.../ShareGroupOffset.java` — the durable
/// per-(group, topic, partition) blob written to `__share_group_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareGroupOffset {
    pub group_id: String,
    pub topic: String,
    pub partition: i32,
    pub start_offset: i64,
    pub state_epoch: u32,
    pub leader_epoch: i32,
    pub batches: Vec<PersisterStateBatch>,
}

// ─── SharePartitionKey + SharePartitionManager ───────────────────────────────

/// Mirrors `server-common/.../share/SharePartitionKey.java`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SharePartitionKey {
    pub group_id: String,
    pub topic: String,
    pub partition: i32,
}

impl SharePartitionKey {
    pub fn new(
        group_id: impl Into<String>,
        topic: impl Into<String>,
        partition: i32,
    ) -> Self {
        Self {
            group_id: group_id.into(),
            topic: topic.into(),
            partition,
        }
    }
}

/// Mirrors `core/.../server/share/SharePartitionManager.java` — a thread-safe
/// map of share-partition state, plus member-level wiring (per-member-id
/// resolution + lock sweep tick).
pub struct SharePartitionManager {
    partitions: Mutex<HashMap<SharePartitionKey, SharePartition>>,
}

impl Default for SharePartitionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SharePartitionManager {
    pub fn new() -> Self {
        Self {
            partitions: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_or_create(
        &self,
        key: SharePartitionKey,
        start_offset: i64,
    ) -> SharePartitionHandle<'_> {
        {
            let mut map = self.partitions.lock().unwrap();
            map.entry(key.clone()).or_insert_with(|| {
                SharePartition::new(
                    key.group_id.clone(),
                    key.topic.clone(),
                    key.partition,
                    start_offset,
                )
            });
        }
        SharePartitionHandle {
            mgr: self,
            key,
        }
    }

    pub fn len(&self) -> usize {
        self.partitions.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.partitions.lock().unwrap().is_empty()
    }

    /// Sweep every share-partition for expired locks.  Returns the total
    /// number of in-flight batches that flipped Acquired → Available.
    pub fn tick_sweep(&self, now_ms: u64) -> usize {
        let mut total = 0;
        for sp in self.partitions.lock().unwrap().values() {
            total += sp.sweep_expired_locks(now_ms).len();
        }
        total
    }

    pub fn snapshot(&self) -> Vec<ShareGroupOffset> {
        self.partitions
            .lock()
            .unwrap()
            .values()
            .map(|sp| sp.snapshot())
            .collect()
    }
}

/// RAII-ish handle for the (one-of-many) shared `SharePartition`.  Wraps
/// the manager-level lock acquisition so callers don't reach into the
/// inner map directly.
pub struct SharePartitionHandle<'a> {
    mgr: &'a SharePartitionManager,
    key: SharePartitionKey,
}

impl SharePartitionHandle<'_> {
    pub fn with<R>(&self, f: impl FnOnce(&mut SharePartition) -> R) -> R {
        let mut map = self.mgr.partitions.lock().unwrap();
        let sp = map.get_mut(&self.key).expect("partition vanished");
        f(sp)
    }

    pub fn read<R>(&self, f: impl FnOnce(&SharePartition) -> R) -> R {
        let map = self.mgr.partitions.lock().unwrap();
        let sp = map.get(&self.key).expect("partition vanished");
        f(sp)
    }
}

// ─── ShareGroup + members ────────────────────────────────────────────────────

/// State of a member of a share group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberState {
    /// Member has just joined; coordinator has not yet returned an
    /// assignment.
    Joining,
    /// Member is actively fetching + acknowledging.
    Stable,
    /// Member has left the group (graceful) or its session expired.
    Left,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareGroupConfig {
    /// `share.record.lock.duration.ms` — KIP-932 default: 30_000.
    pub record_lock_duration_ms: u64,
    /// `share.delivery.count.limit` — KIP-932 default: 5.
    pub delivery_count_limit: u32,
    /// `share.session.timeout.ms` — KIP-932 default: 45_000.
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

/// Mirrors `group-coordinator/.../modern/share/ShareGroup.java`.
pub struct ShareGroup {
    id: String,
    members: Mutex<BTreeMap<String, ShareGroupMember>>,
    config: ShareGroupConfig,
    /// Group-level epoch — bumped on every join/leave.
    group_epoch: Mutex<u32>,
}

impl ShareGroup {
    pub fn new(id: impl Into<String>, config: ShareGroupConfig) -> Self {
        Self {
            id: id.into(),
            members: Mutex::new(BTreeMap::new()),
            config,
            group_epoch: Mutex::new(0),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn config(&self) -> &ShareGroupConfig {
        &self.config
    }

    pub fn group_epoch(&self) -> u32 {
        *self.group_epoch.lock().unwrap()
    }

    /// Add a member; bumps `group_epoch`.
    pub fn join(&self, member_id: impl Into<String>) -> ShareGroupMember {
        let id: String = member_id.into();
        let member =
            ShareGroupMember::new(id.clone(), self.config.session_timeout_ms);
        self.members.lock().unwrap().insert(id, member.clone());
        *self.group_epoch.lock().unwrap() += 1;
        member
    }

    /// Mark a member Stable (coordinator has assigned + acknowledged).
    pub fn stabilise(&self, member_id: &str) -> ShareResult<()> {
        let mut members = self.members.lock().unwrap();
        let m = members
            .get_mut(member_id)
            .ok_or_else(|| ShareError::UnknownGroupMember(member_id.to_string()))?;
        m.state = MemberState::Stable;
        Ok(())
    }

    /// Remove a member; bumps `group_epoch`.
    pub fn leave(&self, member_id: &str) -> ShareResult<()> {
        let mut members = self.members.lock().unwrap();
        if members.remove(member_id).is_none() {
            return Err(ShareError::UnknownGroupMember(member_id.to_string()));
        }
        *self.group_epoch.lock().unwrap() += 1;
        Ok(())
    }

    pub fn member_count(&self) -> usize {
        self.members.lock().unwrap().len()
    }

    pub fn members(&self) -> Vec<ShareGroupMember> {
        self.members.lock().unwrap().values().cloned().collect()
    }

    /// Increment a member's session epoch — called once per share-fetch
    /// round.  Mirrors `ShareSession.epoch++`.
    pub fn bump_session_epoch(&self, member_id: &str) -> ShareResult<u32> {
        let mut members = self.members.lock().unwrap();
        let m = members
            .get_mut(member_id)
            .ok_or_else(|| ShareError::UnknownGroupMember(member_id.to_string()))?;
        m.session_epoch += 1;
        Ok(m.session_epoch)
    }
}

// ─── ShareSession ────────────────────────────────────────────────────────────

/// Mirrors `server/.../share/session/ShareSession.java` — a thin envelope
/// over (key, epoch, partitions) used by the share-fetch RPC to detect a
/// stale fetch (`expectedEpoch != actualEpoch`).
#[derive(Debug, Clone)]
pub struct ShareSession {
    pub group_id: String,
    pub member_id: String,
    pub connection_id: String,
    pub epoch: u32,
    pub partitions: BTreeSet<SharePartitionKey>,
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

    /// Validate that `incoming_epoch` matches the session's expected next
    /// epoch.  Returns the new epoch on success.
    pub fn advance(&mut self, incoming_epoch: u32) -> ShareResult<u32> {
        let expected = self.epoch;
        if incoming_epoch != expected {
            return Err(ShareError::SessionEpochMismatch {
                expected,
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
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_sp() -> SharePartition {
        SharePartition::new("group-1", "topic-A", 0, 0)
            .with_max_delivery_count(3)
            .with_record_lock_duration_ms(1_000)
    }

    #[test]
    fn record_state_round_trip_ids() {
        for s in [
            RecordState::Available,
            RecordState::Acquired,
            RecordState::Acknowledged,
            RecordState::Archived,
        ] {
            assert_eq!(RecordState::from_id(s.id()).unwrap(), s);
        }
        assert!(RecordState::from_id(7).is_err());
    }

    #[test]
    fn record_state_terminals_cannot_transition() {
        for terminal in [RecordState::Acknowledged, RecordState::Archived] {
            for other in [
                RecordState::Available,
                RecordState::Acquired,
                RecordState::Acknowledged,
                RecordState::Archived,
            ] {
                if other == terminal {
                    continue;
                }
                assert!(terminal.validate_transition(other).is_err());
            }
        }
    }

    #[test]
    fn record_state_available_only_to_acquired() {
        assert!(RecordState::Available.validate_transition(RecordState::Acquired).is_ok());
        for bad in [
            RecordState::Available,
            RecordState::Acknowledged,
            RecordState::Archived,
        ] {
            assert!(RecordState::Available.validate_transition(bad).is_err());
        }
    }

    #[test]
    fn record_state_acquired_can_terminal_or_release() {
        for ok in [
            RecordState::Available,
            RecordState::Acknowledged,
            RecordState::Archived,
        ] {
            assert!(RecordState::Acquired.validate_transition(ok).is_ok());
        }
        assert!(RecordState::Acquired.validate_transition(RecordState::Acquired).is_err());
    }

    #[test]
    fn ack_type_round_trip_ids() {
        for a in [
            AcknowledgeType::Accept,
            AcknowledgeType::Release,
            AcknowledgeType::Reject,
            AcknowledgeType::Renew,
        ] {
            assert_eq!(AcknowledgeType::from_id(a.id()).unwrap(), a);
        }
        assert!(AcknowledgeType::from_id(0).is_err());
    }

    #[test]
    fn acquire_introduces_new_batch_at_next_fetch_offset() {
        let mut sp = fresh_sp();
        let acquired = sp.acquire("m1", 10, 1_000).unwrap();
        assert_eq!(acquired.len(), 1);
        assert_eq!(acquired[0].first_offset, 0);
        assert_eq!(acquired[0].last_offset, 9);
        assert_eq!(acquired[0].delivery_count, 1);
        assert_eq!(sp.next_fetch_offset(), 10);
        assert_eq!(sp.batches_snapshot().len(), 1);
    }

    #[test]
    fn accept_terminates_batch() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 5, 1_000).unwrap();
        sp.acknowledge("m1", 0, 4, AcknowledgeType::Accept, 1_500)
            .unwrap();
        let snap = sp.batches_snapshot();
        assert_eq!(snap[0].batch_state.state, RecordState::Acknowledged);
        assert!(snap[0].batch_state.lock_expires_at_ms.is_none());
    }

    #[test]
    fn release_returns_batch_to_available_for_redeliver() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 4, 1_000).unwrap();
        sp.acknowledge("m1", 0, 3, AcknowledgeType::Release, 1_100)
            .unwrap();
        let snap = sp.batches_snapshot();
        assert_eq!(snap[0].batch_state.state, RecordState::Available);
        assert!(snap[0].batch_state.lock_expires_at_ms.is_none());
        // Re-acquire bumps delivery count.
        let again = sp.acquire("m2", 4, 1_200).unwrap();
        assert_eq!(again[0].delivery_count, 2);
    }

    #[test]
    fn reject_archives_immediately() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 2, 1_000).unwrap();
        sp.acknowledge("m1", 0, 1, AcknowledgeType::Reject, 1_100)
            .unwrap();
        let snap = sp.batches_snapshot();
        assert_eq!(snap[0].batch_state.state, RecordState::Archived);
    }

    #[test]
    fn renew_extends_lock_only() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 2, 1_000).unwrap();
        let snap_before = sp.batches_snapshot();
        let before = snap_before[0].batch_state.lock_expires_at_ms.unwrap();
        sp.acknowledge("m1", 0, 1, AcknowledgeType::Renew, 5_000)
            .unwrap();
        let snap_after = sp.batches_snapshot();
        assert_eq!(snap_after[0].batch_state.state, RecordState::Acquired);
        assert!(snap_after[0].batch_state.lock_expires_at_ms.unwrap() > before);
    }

    #[test]
    fn ack_from_wrong_member_rejected() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 2, 1_000).unwrap();
        let err = sp
            .acknowledge("m2", 0, 1, AcknowledgeType::Accept, 1_100)
            .expect_err("wrong member");
        assert!(matches!(err, ShareError::NotAcquiredByMember { .. }));
    }

    #[test]
    fn ack_on_unknown_offset_rejected() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 1, 1_000).unwrap();
        let err = sp
            .acknowledge("m1", 99, 99, AcknowledgeType::Accept, 1_100)
            .expect_err("unknown offset");
        assert!(matches!(err, ShareError::OffsetNotFound(99)));
    }

    #[test]
    fn ack_on_non_acquired_rejected() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 1, 1_000).unwrap();
        sp.acknowledge("m1", 0, 0, AcknowledgeType::Accept, 1_100)
            .unwrap();
        let err = sp
            .acknowledge("m1", 0, 0, AcknowledgeType::Accept, 1_200)
            .expect_err("non-acquired");
        assert!(matches!(err, ShareError::InvalidRecordState { .. }));
    }

    #[test]
    fn sweep_expired_locks_releases_batches() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 2, 0).unwrap();
        let released = sp.sweep_expired_locks(5_000);
        assert_eq!(released.len(), 1);
        let snap = sp.batches_snapshot();
        assert_eq!(snap[0].batch_state.state, RecordState::Available);
    }

    #[test]
    fn sweep_does_not_release_acknowledged_or_acquired_with_future_lock() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 2, 100_000).unwrap();
        let released = sp.sweep_expired_locks(1_000);
        assert!(released.is_empty());
    }

    #[test]
    fn release_after_max_delivery_archives() {
        let mut sp = fresh_sp(); // max_delivery_count = 3
        // Three acquire+release cycles → next release archives.
        for _ in 0..3 {
            sp.acquire("m1", 1, 1_000).unwrap();
            sp.acknowledge("m1", 0, 0, AcknowledgeType::Release, 1_100)
                .unwrap();
        }
        let snap = sp.batches_snapshot();
        assert_eq!(snap[0].batch_state.state, RecordState::Archived);
    }

    #[test]
    fn move_start_offset_drops_stale_batches_and_bumps_epoch() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 10, 1_000).unwrap();
        sp.acknowledge("m1", 0, 9, AcknowledgeType::Accept, 1_100)
            .unwrap();
        let before = sp.state_epoch();
        // Move start past the batch's last_offset; the batch is fully stale.
        sp.move_start_offset(10);
        assert_eq!(sp.state_epoch(), before + 1);
        assert!(sp.batches_snapshot().is_empty());
        // Moving partway through a batch leaves it intact (upstream parity:
        // start_offset advance does NOT split batches — only fully-stale
        // ones are dropped, the next acquire allocates a fresh batch above).
        sp.acquire("m1", 5, 1_200).unwrap();
        sp.move_start_offset(12);
        assert_eq!(sp.batches_snapshot().len(), 1);
    }

    #[test]
    fn snapshot_round_trip_carries_state_batches() {
        let mut sp = fresh_sp();
        sp.acquire("m1", 2, 1_000).unwrap();
        let snap = sp.snapshot();
        assert_eq!(snap.group_id, "group-1");
        assert_eq!(snap.partition, 0);
        assert_eq!(snap.batches.len(), 1);
        assert_eq!(snap.batches[0].state, RecordState::Acquired);
        assert_eq!(snap.batches[0].delivery_count, 1);
    }

    #[test]
    fn manager_isolates_by_key() {
        let mgr = SharePartitionManager::new();
        let k1 = SharePartitionKey::new("g1", "t", 0);
        let k2 = SharePartitionKey::new("g1", "t", 1);
        let h1 = mgr.get_or_create(k1.clone(), 0);
        let h2 = mgr.get_or_create(k2.clone(), 100);
        h1.with(|sp| sp.acquire("m1", 5, 1_000)).unwrap();
        h2.with(|sp| sp.acquire("m1", 5, 1_000)).unwrap();
        assert_eq!(mgr.len(), 2);
        let snap1 = h1.read(|sp| sp.snapshot());
        let snap2 = h2.read(|sp| sp.snapshot());
        assert_eq!(snap1.start_offset, 0);
        assert_eq!(snap2.start_offset, 100);
    }

    #[test]
    fn manager_tick_sweep_counts_releases() {
        let mgr = SharePartitionManager::new();
        let k = SharePartitionKey::new("g1", "t", 0);
        let h = mgr.get_or_create(k, 0);
        h.with(|sp| sp.acquire("m1", 2, 0)).unwrap();
        let released = mgr.tick_sweep(60_000);
        assert_eq!(released, 1);
    }

    #[test]
    fn share_group_join_leave_bumps_epoch() {
        let g = ShareGroup::new("g1", ShareGroupConfig::default());
        assert_eq!(g.group_epoch(), 0);
        g.join("m1");
        assert_eq!(g.group_epoch(), 1);
        g.join("m2");
        assert_eq!(g.group_epoch(), 2);
        g.leave("m1").unwrap();
        assert_eq!(g.group_epoch(), 3);
        assert_eq!(g.member_count(), 1);
    }

    #[test]
    fn share_group_member_state_machine() {
        let g = ShareGroup::new("g1", ShareGroupConfig::default());
        g.join("m1");
        assert!(matches!(
            g.members().into_iter().find(|m| m.id == "m1").unwrap().state,
            MemberState::Joining
        ));
        g.stabilise("m1").unwrap();
        assert!(matches!(
            g.members().into_iter().find(|m| m.id == "m1").unwrap().state,
            MemberState::Stable
        ));
    }

    #[test]
    fn share_group_unknown_member_errors() {
        let g = ShareGroup::new("g1", ShareGroupConfig::default());
        assert!(matches!(
            g.stabilise("m1").unwrap_err(),
            ShareError::UnknownGroupMember(_)
        ));
        assert!(matches!(
            g.leave("m1").unwrap_err(),
            ShareError::UnknownGroupMember(_)
        ));
    }

    #[test]
    fn share_group_bump_session_epoch_monotonic() {
        let g = ShareGroup::new("g1", ShareGroupConfig::default());
        g.join("m1");
        assert_eq!(g.bump_session_epoch("m1").unwrap(), 1);
        assert_eq!(g.bump_session_epoch("m1").unwrap(), 2);
        assert_eq!(g.bump_session_epoch("m1").unwrap(), 3);
    }

    #[test]
    fn share_session_advance_validates_epoch() {
        let mut s = ShareSession::new("g1", "m1", "conn-A");
        assert_eq!(s.advance(0).unwrap(), 1);
        assert_eq!(s.advance(1).unwrap(), 2);
        let err = s.advance(99).expect_err("stale epoch rejected");
        assert!(matches!(err, ShareError::SessionEpochMismatch { .. }));
    }

    #[test]
    fn share_session_add_remove_partition_dedup() {
        let mut s = ShareSession::new("g1", "m1", "conn-A");
        let k = SharePartitionKey::new("g1", "t", 0);
        s.add_partition(k.clone());
        s.add_partition(k.clone());
        assert_eq!(s.partitions.len(), 1);
        assert!(s.remove_partition(&k));
        assert!(!s.remove_partition(&k));
    }

    #[test]
    fn record_state_id_4_is_archived_not_3() {
        // Upstream parity — RecordState.ARCHIVED = 4, id 3 is intentionally
        // skipped.  Guard against accidental renumbering.
        assert_eq!(RecordState::Archived.id(), 4);
        assert!(RecordState::from_id(3).is_err());
    }

    #[test]
    fn in_flight_batch_record_count() {
        let b = InFlightBatch::new(0, 4);
        assert_eq!(b.record_count(), 5);
        let single = InFlightBatch::new(7, 7);
        assert_eq!(single.record_count(), 1);
    }

    #[test]
    fn share_group_config_defaults_match_kip_932() {
        let c = ShareGroupConfig::default();
        assert_eq!(c.record_lock_duration_ms, 30_000);
        assert_eq!(c.delivery_count_limit, 5);
        assert_eq!(c.session_timeout_ms, 45_000);
    }

    #[test]
    fn acquire_with_zero_budget_returns_empty() {
        let mut sp = fresh_sp();
        let r = sp.acquire("m1", 0, 1_000).unwrap();
        assert!(r.is_empty());
        assert_eq!(sp.next_fetch_offset(), 0);
    }

    #[test]
    fn reacquire_skips_batch_with_delivery_cap_hit() {
        let mut sp = SharePartition::new("g1", "t", 0, 0).with_max_delivery_count(1);
        sp.acquire("m1", 2, 1_000).unwrap();
        // Release once — delivery_count==1, which equals max, so the
        // release-path archives.
        sp.acknowledge("m1", 0, 1, AcknowledgeType::Release, 1_100)
            .unwrap();
        let snap = sp.batches_snapshot();
        assert_eq!(snap[0].batch_state.state, RecordState::Archived);
        // Re-acquire should NOT pick up the archived batch.
        let again = sp.acquire("m1", 2, 1_200).unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].first_offset, 2); // brand-new batch starts at next_fetch_offset
    }
}
