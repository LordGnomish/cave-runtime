// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Apache Pulsar transaction support — coordinator, buffer, pending-ack, and
//! timeout tracker.
//!
//! A faithful in-memory port of Apache Pulsar v4.2.0's transaction subsystem.
//! Kafka EOS (idempotent producer + transaction markers, see
//! [`crate::transactions`]) remains the canonical mapped path per
//! `ADR-RUNTIME-STREAMING-CONSOLIDATION-001`; this module ports Pulsar's
//! parallel-track transaction coordinator.
//!
//! Upstream references:
//!   - `pulsar-client-api/.../transaction/TxnID.java`
//!   - `pulsar-transaction/common/.../proto/TxnStatus.java` + `util/TransactionUtil.java`
//!   - `pulsar-transaction/coordinator/.../TxnMeta.java` + `impl/*`
//!   - `pulsar-broker/.../transaction/buffer/impl/InMemTransactionBuffer.java`
//!   - `pulsar-broker/.../transaction/buffer/AbortedTxnProcessor.java`
//!   - `pulsar-broker/.../transaction/pendingack/PendingAckHandle.java`
//!   - `pulsar-transaction/coordinator/.../TransactionTimeoutTracker.java`

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors raised by the Pulsar transaction subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxnError {
    /// Compare-and-set / gate failure (mirrors `InvalidTxnStatusException`).
    InvalidTxnStatus {
        txn_id: Option<TxnId>,
        expected: TxnStatus,
        actual: TxnStatus,
    },
    /// No transaction with the given id is known to the store.
    TransactionNotFound(TxnId),
    /// A second pending ack landed on a position already held (mirrors
    /// `TransactionConflictException`).
    TransactionConflict {
        position: PendingAckPosition,
        existing: TxnId,
    },
}

impl std::fmt::Display for TxnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxnError::InvalidTxnStatus {
                txn_id,
                expected,
                actual,
            } => write!(
                f,
                "invalid txn status for {txn_id:?}: expected {expected:?}, actual {actual:?}"
            ),
            TxnError::TransactionNotFound(id) => write!(f, "transaction {id} not found"),
            TxnError::TransactionConflict { position, existing } => write!(
                f,
                "pending-ack conflict at {position:?}: already held by {existing}"
            ),
        }
    }
}

impl std::error::Error for TxnError {}

/// Result alias for the Pulsar transaction subsystem.
pub type TxnResult<T> = Result<T, TxnError>;

// ── TxnId ────────────────────────────────────────────────────────────────────

/// Pulsar transaction id: `(most_sig_bits = coordinator id, least_sig_bits =
/// monotonic local sequence)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxnId {
    most_sig_bits: i64,
    least_sig_bits: i64,
}

impl TxnId {
    pub fn new(most_sig_bits: i64, least_sig_bits: i64) -> Self {
        Self {
            most_sig_bits,
            least_sig_bits,
        }
    }

    pub fn most_sig_bits(&self) -> i64 {
        self.most_sig_bits
    }

    pub fn least_sig_bits(&self) -> i64 {
        self.least_sig_bits
    }
}

impl std::fmt::Display for TxnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({},{})", self.most_sig_bits, self.least_sig_bits)
    }
}

// ── TxnStatus ────────────────────────────────────────────────────────────────

/// Transaction status (mirrors the proto enum order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnStatus {
    Open,
    Committing,
    Committed,
    Aborting,
    Aborted,
}

impl TxnStatus {
    /// `TransactionUtil.canTransitionTo`: Open may go anywhere except straight
    /// to a terminal; Committing → {Committing, Committed}; Aborting →
    /// {Aborting, Aborted}; terminals self-loop only.
    pub fn can_transition_to(current: TxnStatus, new: TxnStatus) -> bool {
        use TxnStatus::*;
        match current {
            Open => !matches!(new, Committed | Aborted),
            Committing => matches!(new, Committing | Committed),
            Committed => matches!(new, Committed),
            Aborting => matches!(new, Aborting | Aborted),
            Aborted => matches!(new, Aborted),
        }
    }
}

// ── TransactionSubscription ──────────────────────────────────────────────────

/// A `(topic, subscription)` pair — the element of a txn's acked-subscription
/// set. Ordered lexicographically.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionSubscription {
    pub topic: String,
    pub subscription: String,
}

impl TransactionSubscription {
    pub fn new(topic: impl Into<String>, subscription: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            subscription: subscription.into(),
        }
    }
}

// ── TxnMeta ──────────────────────────────────────────────────────────────────

/// Per-transaction metadata held by the coordinator store.
#[derive(Debug, Clone)]
pub struct TxnMeta {
    id: TxnId,
    produced_partitions: BTreeSet<String>,
    acked_partitions: BTreeSet<TransactionSubscription>,
    status: TxnStatus,
    pub open_timestamp: i64,
    pub timeout_at: i64,
    pub owner: String,
}

impl TxnMeta {
    pub fn new(id: TxnId, open_timestamp: i64, timeout_at: i64, owner: impl Into<String>) -> Self {
        Self {
            id,
            produced_partitions: BTreeSet::new(),
            acked_partitions: BTreeSet::new(),
            status: TxnStatus::Open,
            open_timestamp,
            timeout_at,
            owner: owner.into(),
        }
    }

    pub fn id(&self) -> TxnId {
        self.id
    }

    pub fn status(&self) -> TxnStatus {
        self.status
    }

    /// Produced partitions, ascending (BTreeSet order).
    pub fn produced_partitions(&self) -> Vec<String> {
        self.produced_partitions.iter().cloned().collect()
    }

    /// Acked subscriptions, ordered by `(topic, subscription)`.
    pub fn acked_partitions(&self) -> Vec<TransactionSubscription> {
        self.acked_partitions.iter().cloned().collect()
    }

    fn require_open(&self) -> TxnResult<()> {
        if self.status != TxnStatus::Open {
            return Err(TxnError::InvalidTxnStatus {
                txn_id: Some(self.id),
                expected: TxnStatus::Open,
                actual: self.status,
            });
        }
        Ok(())
    }

    /// Add produced partitions — gated to `Open`. Idempotent (set semantics).
    pub fn add_produced_partitions(&mut self, partitions: Vec<String>) -> TxnResult<()> {
        self.require_open()?;
        self.produced_partitions.extend(partitions);
        Ok(())
    }

    /// Add acked subscriptions — gated to `Open`. Idempotent (set semantics).
    pub fn add_acked_partitions(&mut self, subs: Vec<TransactionSubscription>) -> TxnResult<()> {
        self.require_open()?;
        self.acked_partitions.extend(subs);
        Ok(())
    }

    /// Compare-and-set the status: `expected` must equal the current status,
    /// and the transition must be permitted by [`TxnStatus::can_transition_to`].
    pub fn update_txn_status(&mut self, new: TxnStatus, expected: TxnStatus) -> TxnResult<()> {
        if self.status != expected {
            return Err(TxnError::InvalidTxnStatus {
                txn_id: Some(self.id),
                expected,
                actual: self.status,
            });
        }
        if !TxnStatus::can_transition_to(self.status, new) {
            return Err(TxnError::InvalidTxnStatus {
                txn_id: Some(self.id),
                expected: new,
                actual: self.status,
            });
        }
        self.status = new;
        Ok(())
    }
}

// ── Stats ────────────────────────────────────────────────────────────────────

/// Coordinator counters (mirror `TransactionMetadataStoreStats`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransactionStats {
    pub created: u64,
    pub committed: u64,
    pub aborted: u64,
    pub timed_out: u64,
}

// ── TransactionTimeoutTracker ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct TimeoutEntry {
    deadline: i64,
    txn_id: TxnId,
}

impl Ord for TimeoutEntry {
    // Reverse ordering on deadline so the `BinaryHeap` (a max-heap) yields the
    // earliest deadline first; tie-break by txn_id (also reversed) for a total,
    // Eq-consistent order.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.txn_id.cmp(&self.txn_id))
    }
}

impl PartialOrd for TimeoutEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Min-heap of transaction deadlines.
#[derive(Debug, Default)]
pub struct TransactionTimeoutTracker {
    heap: BinaryHeap<TimeoutEntry>,
}

impl TransactionTimeoutTracker {
    pub fn add_transaction(&mut self, txn_id: TxnId, deadline: i64) {
        self.heap.push(TimeoutEntry { deadline, txn_id });
    }

    /// Drain transactions whose deadline is strictly before `now` (earliest
    /// first), stopping at the first non-expired entry.
    pub fn poll_expired(&mut self, now: i64) -> Vec<TxnId> {
        let mut out = Vec::new();
        while let Some(top) = self.heap.peek() {
            if top.deadline < now {
                out.push(self.heap.pop().unwrap().txn_id);
            } else {
                break;
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

// ── TransactionMetadataStore ─────────────────────────────────────────────────

/// In-memory transaction metadata store for a single coordinator.
#[derive(Debug)]
pub struct TransactionMetadataStore {
    coordinator_id: i64,
    sequence_id_generator: i64,
    txns: HashMap<i64, TxnMeta>,
    stats: TransactionStats,
    timeouts: TransactionTimeoutTracker,
}

impl TransactionMetadataStore {
    pub fn new(coordinator_id: i64) -> Self {
        Self {
            coordinator_id,
            sequence_id_generator: 0,
            txns: HashMap::new(),
            stats: TransactionStats::default(),
            timeouts: TransactionTimeoutTracker::default(),
        }
    }

    pub fn coordinator_id(&self) -> i64 {
        self.coordinator_id
    }

    pub fn stats(&self) -> &TransactionStats {
        &self.stats
    }

    /// Allocate a fresh `Open` transaction with a monotonic `least_sig_bits`,
    /// registering its timeout deadline.
    pub fn new_transaction(&mut self, timeout_ms: i64, owner: impl Into<String>, now_ms: i64) -> TxnId {
        let least = self.sequence_id_generator;
        self.sequence_id_generator += 1;
        let id = TxnId::new(self.coordinator_id, least);
        let deadline = now_ms + timeout_ms;
        let meta = TxnMeta::new(id, now_ms, deadline, owner);
        self.txns.insert(least, meta);
        self.timeouts.add_transaction(id, deadline);
        self.stats.created += 1;
        id
    }

    pub fn get(&self, id: &TxnId) -> Option<&TxnMeta> {
        if id.most_sig_bits() != self.coordinator_id {
            return None;
        }
        self.txns.get(&id.least_sig_bits())
    }

    pub fn get_mut(&mut self, id: &TxnId) -> Option<&mut TxnMeta> {
        if id.most_sig_bits() != self.coordinator_id {
            return None;
        }
        self.txns.get_mut(&id.least_sig_bits())
    }

    /// Compare-and-set the status of a stored transaction, updating the
    /// terminal counters (`committed` / `aborted`) on success.
    pub fn update_status(&mut self, id: &TxnId, new: TxnStatus, expected: TxnStatus) -> TxnResult<()> {
        let meta = self
            .txns
            .get_mut(&id.least_sig_bits())
            .ok_or(TxnError::TransactionNotFound(*id))?;
        meta.update_txn_status(new, expected)?;
        match new {
            TxnStatus::Committed => self.stats.committed += 1,
            TxnStatus::Aborted => self.stats.aborted += 1,
            _ => {}
        }
        Ok(())
    }

    /// Abort every still-`Open` transaction whose deadline has passed.
    /// The `timed_out` counter is bumped at the `Open -> Aborting` step (and
    /// only then), keeping it distinct from the regular `aborted` counter.
    pub fn process_timeouts(&mut self, now_ms: i64) -> Vec<TxnId> {
        let expired = self.timeouts.poll_expired(now_ms);
        let mut aborted = Vec::new();
        for id in expired {
            if let Some(meta) = self.txns.get_mut(&id.least_sig_bits()) {
                if meta.status() == TxnStatus::Open {
                    // Timeout-driven abort: bump timed_out at the Aborting step.
                    let _ = meta.update_txn_status(TxnStatus::Aborting, TxnStatus::Open);
                    self.stats.timed_out += 1;
                    let _ = meta.update_txn_status(TxnStatus::Aborted, TxnStatus::Aborting);
                    aborted.push(id);
                }
            }
        }
        aborted
    }
}

// ── TransactionCoordinator (facade) ──────────────────────────────────────────

/// Facade over a [`TransactionMetadataStore`] driving the begin / commit /
/// abort / timeout lifecycle.
#[derive(Debug)]
pub struct TransactionCoordinator {
    store: TransactionMetadataStore,
}

impl TransactionCoordinator {
    pub fn new(coordinator_id: i64) -> Self {
        Self {
            store: TransactionMetadataStore::new(coordinator_id),
        }
    }

    pub fn store(&self) -> &TransactionMetadataStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut TransactionMetadataStore {
        &mut self.store
    }

    pub fn begin(&mut self, timeout_ms: i64, owner: impl Into<String>, now_ms: i64) -> TxnId {
        self.store.new_transaction(timeout_ms, owner, now_ms)
    }

    /// Drive `Open -> Committing -> Committed`.
    pub fn commit(&mut self, id: &TxnId) -> TxnResult<()> {
        self.store.update_status(id, TxnStatus::Committing, TxnStatus::Open)?;
        self.store.update_status(id, TxnStatus::Committed, TxnStatus::Committing)?;
        Ok(())
    }

    /// Drive `Open -> Aborting -> Aborted`.
    pub fn abort(&mut self, id: &TxnId) -> TxnResult<()> {
        self.store.update_status(id, TxnStatus::Aborting, TxnStatus::Open)?;
        self.store.update_status(id, TxnStatus::Aborted, TxnStatus::Aborting)?;
        Ok(())
    }

    pub fn process_timeouts(&mut self, now_ms: i64) -> Vec<TxnId> {
        self.store.process_timeouts(now_ms)
    }
}

// ── Transaction buffer ───────────────────────────────────────────────────────

/// A persisted entry position appended to a transaction buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPosition {
    pub ledger_id: i64,
    pub entry_id: i64,
}

/// Per-transaction in-memory buffer (`InMemTransactionBuffer.TxnBuffer`).
#[derive(Debug)]
pub struct TxnBuffer {
    txn_id: TxnId,
    status: TxnStatus,
    entries: BTreeMap<i64, EntryPosition>,
    committed_at: Option<(i64, i64)>,
}

impl TxnBuffer {
    pub fn new(txn_id: TxnId) -> Self {
        Self {
            txn_id,
            status: TxnStatus::Open,
            entries: BTreeMap::new(),
            committed_at: None,
        }
    }

    pub fn txn_id(&self) -> TxnId {
        self.txn_id
    }

    pub fn status(&self) -> TxnStatus {
        self.status
    }

    pub fn committed_at(&self) -> Option<(i64, i64)> {
        self.committed_at
    }

    /// Append an entry at `sequence_id` — gated to `Open` (last write wins).
    pub fn append_entry(&mut self, sequence_id: i64, position: EntryPosition) -> TxnResult<()> {
        if self.status != TxnStatus::Open {
            return Err(TxnError::InvalidTxnStatus {
                txn_id: Some(self.txn_id),
                expected: TxnStatus::Open,
                actual: self.status,
            });
        }
        self.entries.insert(sequence_id, position);
        Ok(())
    }

    /// Move `Open -> Committing`.
    pub fn committing_txn(&mut self) {
        self.status = TxnStatus::Committing;
    }

    /// Mark committed and record the commit position; reads become valid.
    pub fn commit_txn(&mut self, committed_ledger: i64, committed_entry: i64) {
        self.status = TxnStatus::Committed;
        self.committed_at = Some((committed_ledger, committed_entry));
    }

    /// Abort the buffer — `Open`-only in the in-mem impl.
    pub fn abort_txn(&mut self) -> TxnResult<()> {
        if self.status != TxnStatus::Open {
            return Err(TxnError::InvalidTxnStatus {
                txn_id: Some(self.txn_id),
                expected: TxnStatus::Open,
                actual: self.status,
            });
        }
        self.status = TxnStatus::Aborted;
        Ok(())
    }

    /// Read up to `num` entries with `sequence_id >= start_sequence_id`,
    /// ascending (tail-map semantics).
    pub fn read_entries(&self, num: usize, start_sequence_id: i64) -> Vec<EntryPosition> {
        self.entries
            .range(start_sequence_id..)
            .take(num)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Greatest sequence id currently buffered.
    pub fn last_sequence_id(&self) -> Option<i64> {
        self.entries.keys().next_back().copied()
    }
}

/// Tracks aborted transactions for read-committed visibility.
#[derive(Debug, Default)]
pub struct AbortedTxnProcessor {
    aborted: HashSet<TxnId>,
}

impl AbortedTxnProcessor {
    pub fn put_aborted_txn(&mut self, txn_id: TxnId) {
        self.aborted.insert(txn_id);
    }

    pub fn check_aborted_transaction(&self, txn_id: TxnId) -> bool {
        self.aborted.contains(&txn_id)
    }

    /// Drop entries owned by aborted transactions, preserving input order.
    pub fn filter_aborted(&self, entries: Vec<(TxnId, EntryPosition)>) -> Vec<EntryPosition> {
        entries
            .into_iter()
            .filter(|(txn, _)| !self.aborted.contains(txn))
            .map(|(_, pos)| pos)
            .collect()
    }
}

/// Multi-transaction in-memory buffer with an aborted-txn filter.
#[derive(Debug, Default)]
pub struct TransactionBuffer {
    buffers: HashMap<TxnId, TxnBuffer>,
    aborted: AbortedTxnProcessor,
}

impl TransactionBuffer {
    fn buffer_mut(&mut self, txn_id: &TxnId) -> &mut TxnBuffer {
        self.buffers.entry(*txn_id).or_insert_with(|| TxnBuffer::new(*txn_id))
    }

    pub fn append_entry(&mut self, txn_id: &TxnId, sequence_id: i64, position: EntryPosition) -> TxnResult<()> {
        self.buffer_mut(txn_id).append_entry(sequence_id, position)
    }

    pub fn committing_txn(&mut self, txn_id: &TxnId) {
        self.buffer_mut(txn_id).committing_txn();
    }

    pub fn commit_txn(&mut self, txn_id: &TxnId, committed_ledger: i64, committed_entry: i64) {
        self.buffer_mut(txn_id).commit_txn(committed_ledger, committed_entry);
    }

    /// Abort a transaction's buffer and record it in the aborted set.
    pub fn abort_txn(&mut self, txn_id: &TxnId) -> TxnResult<()> {
        self.buffer_mut(txn_id).abort_txn()?;
        self.aborted.put_aborted_txn(*txn_id);
        Ok(())
    }

    pub fn read_entries(&self, txn_id: &TxnId, num: usize, start_sequence_id: i64) -> Vec<EntryPosition> {
        self.buffers
            .get(txn_id)
            .map(|b| b.read_entries(num, start_sequence_id))
            .unwrap_or_default()
    }

    pub fn last_sequence_id(&self, txn_id: &TxnId) -> Option<i64> {
        self.buffers.get(txn_id).and_then(|b| b.last_sequence_id())
    }

    pub fn is_aborted(&self, txn_id: TxnId) -> bool {
        self.aborted.check_aborted_transaction(txn_id)
    }
}

// ── PendingAckHandle ─────────────────────────────────────────────────────────

/// Dedup key for individual acks: `(ledger_id, entry_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PendingAckPosition {
    pub ledger_id: i64,
    pub entry_id: i64,
}

/// Pending individual acks, keyed by message position.
#[derive(Debug, Default)]
pub struct PendingAckHandle {
    individual_acks: HashMap<PendingAckPosition, TxnId>,
}

impl PendingAckHandle {
    /// Record a pending ack at `position` for `txn_id`. A position already held
    /// by any transaction conflicts (dedup by `(ledger, entry)`).
    pub fn individual_ack(&mut self, txn_id: TxnId, position: PendingAckPosition) -> TxnResult<()> {
        if let Some(existing) = self.individual_acks.get(&position) {
            return Err(TxnError::TransactionConflict {
                position,
                existing: *existing,
            });
        }
        self.individual_acks.insert(position, txn_id);
        Ok(())
    }

    /// Apply the acks pending under `txn_id` (they leave the pending map).
    pub fn commit_txn(&mut self, txn_id: &TxnId) {
        self.individual_acks.retain(|_, t| t != txn_id);
    }

    /// Release the acks pending under `txn_id` (removed; messages redeliverable).
    pub fn abort_txn(&mut self, txn_id: &TxnId) {
        self.individual_acks.retain(|_, t| t != txn_id);
    }

    pub fn pending_count(&self) -> usize {
        self.individual_acks.len()
    }
}
