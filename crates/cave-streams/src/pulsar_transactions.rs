// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Pulsar Transactions — deep-port of Apache Pulsar v4.2.0
//! `pulsar-transaction/coordinator/` + `pulsar-broker/.../transaction/`.
//!
//! Upstream layout we honour:
//!
//! * `TxnID` (most-sig + least-sig u64) — `pulsar-client-api`'s `TxnID.java`
//! * `TxnStatus` state machine — `proto/TxnStatus.java` +
//!   `coordinator/util/TransactionUtil.canTransitionTo`
//! * `TxnMeta` — `coordinator/TxnMeta.java` + `impl/TxnMetaImpl.java`
//! * `InMemTransactionMetadataStore` — `coordinator/impl/InMemTransactionMetadataStore.java`
//! * `TransactionBuffer` (per-topic dedup + commit log) —
//!   `broker/transaction/buffer/TransactionBuffer.java`
//! * `AbortedTxnProcessor` — `broker/transaction/buffer/AbortedTxnProcessor.java`
//! * `PendingAckHandle` — `broker/transaction/pendingack/impl/PendingAckHandleImpl.java`
//! * `TransactionCoordinator` orchestration + `TransactionTimeoutTracker`
//!
//! The port follows the upstream semantics exactly:
//!   * `OPEN → {COMMITTING, ABORTING}` (no jump to terminal states)
//!   * `COMMITTING → {COMMITTING, COMMITTED}` (re-entrant; idempotent commit)
//!   * `ABORTING → {ABORTING, ABORTED}`   (re-entrant; idempotent abort)
//!   * `COMMITTED` and `ABORTED` are terminal and only loop to themselves.
//!   * `addProducedPartitions` + `addAckedPartitions` fail unless `OPEN`.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ─── TxnID ────────────────────────────────────────────────────────────────────

/// Pulsar transaction identifier — a 128-bit pair of (most-sig, least-sig).
///
/// Mirrors `pulsar-client-api/.../TxnID.java`.  The most-significant half is
/// the `TransactionCoordinatorID` (TC id) and the least-significant half is
/// the per-TC monotonic counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxnId {
    pub most_sig_bits: u64,
    pub least_sig_bits: u64,
}

impl TxnId {
    pub fn new(most_sig_bits: u64, least_sig_bits: u64) -> Self {
        Self {
            most_sig_bits,
            least_sig_bits,
        }
    }

    /// `TransactionCoordinatorID` — the upper 64 bits.
    pub fn tc_id(&self) -> u64 {
        self.most_sig_bits
    }

    /// Local sequence number assigned by the owning TC.
    pub fn local_id(&self) -> u64 {
        self.least_sig_bits
    }
}

impl std::fmt::Display for TxnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({},{})", self.most_sig_bits, self.least_sig_bits)
    }
}

// ─── TxnStatus + state machine ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnStatus {
    Open,
    Committing,
    Committed,
    Aborting,
    Aborted,
}

/// Upstream parity:
/// `pulsar-transaction/coordinator/src/main/java/.../util/TransactionUtil.java`.
pub fn can_transition_to(current: TxnStatus, new_status: TxnStatus) -> bool {
    use TxnStatus::*;
    match current {
        // From OPEN you can stay open, abort, or commit (any non-terminal
        // jump is rejected — the terminal Committed/Aborted require a
        // matching Committing/Aborting predecessor).
        Open => new_status != Committed && new_status != Aborted,
        // COMMITTING is re-entrant + drains to COMMITTED.
        Committing => matches!(new_status, Committing | Committed),
        // COMMITTED is terminal.
        Committed => matches!(new_status, Committed),
        // ABORTING is re-entrant + drains to ABORTED.
        Aborting => matches!(new_status, Aborting | Aborted),
        // ABORTED is terminal.
        Aborted => matches!(new_status, Aborted),
    }
}

// ─── TransactionSubscription ─────────────────────────────────────────────────

/// Mirrors `TransactionSubscription { topic, subscription }`.
/// Ordering is `(topic, subscription)` lexicographic — matches upstream
/// `Comparable.compareTo` used by `Collections.sort` on `ackedPartitions()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

// ─── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorError {
    /// `coordinator/exceptions/CoordinatorException.InvalidTxnStatusException`.
    InvalidTxnStatus {
        txn_id: TxnId,
        expected: TxnStatus,
        actual: TxnStatus,
    },
    /// `coordinator/exceptions/CoordinatorException.TransactionNotFoundException`.
    TransactionNotFound(TxnId),
    /// `broker/transaction/exception/TransactionBufferException.UnexpectedStatus`.
    UnexpectedStatus { txn_id: TxnId, status: TxnStatus },
    BlankOwner,
}

impl std::fmt::Display for CoordinatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoordinatorError::InvalidTxnStatus {
                txn_id,
                expected,
                actual,
            } => write!(
                f,
                "txn {} in status {:?}, expected {:?}",
                txn_id, actual, expected
            ),
            CoordinatorError::TransactionNotFound(t) => write!(f, "txn {} not found", t),
            CoordinatorError::UnexpectedStatus { txn_id, status } => {
                write!(f, "txn {} unexpected status {:?}", txn_id, status)
            }
            CoordinatorError::BlankOwner => write!(f, "owner can't be blank"),
        }
    }
}

impl std::error::Error for CoordinatorError {}

pub type CoordinatorResult<T> = Result<T, CoordinatorError>;

// ─── TxnMeta ─────────────────────────────────────────────────────────────────

/// Mirrors `TxnMetaImpl`. Uses `BTreeSet` for produced/acked partitions so
/// `produced_partitions()` and `acked_partitions()` return entries in upstream
/// `Collections.sort`-equivalent order.
#[derive(Debug, Clone)]
pub struct TxnMeta {
    pub id: TxnId,
    pub status: TxnStatus,
    pub produced_partitions: BTreeSet<String>,
    pub acked_partitions: BTreeSet<TransactionSubscription>,
    pub open_timestamp_ms: u64,
    pub timeout_at_ms: u64,
    pub owner: Option<String>,
}

impl TxnMeta {
    fn new(id: TxnId, open_ts_ms: u64, timeout_at_ms: u64, owner: Option<String>) -> Self {
        Self {
            id,
            status: TxnStatus::Open,
            produced_partitions: BTreeSet::new(),
            acked_partitions: BTreeSet::new(),
            open_timestamp_ms: open_ts_ms,
            timeout_at_ms,
            owner,
        }
    }

    pub fn produced_partitions(&self) -> Vec<String> {
        self.produced_partitions.iter().cloned().collect()
    }

    pub fn acked_partitions(&self) -> Vec<TransactionSubscription> {
        self.acked_partitions.iter().cloned().collect()
    }

    /// Upstream `TxnMetaImpl.addProducedPartitions` — only valid when OPEN.
    pub fn add_produced_partitions(
        &mut self,
        partitions: impl IntoIterator<Item = String>,
    ) -> CoordinatorResult<()> {
        self.check_status(TxnStatus::Open)?;
        self.produced_partitions.extend(partitions);
        Ok(())
    }

    /// Upstream `TxnMetaImpl.addAckedPartitions` — only valid when OPEN.
    pub fn add_acked_partitions(
        &mut self,
        subs: impl IntoIterator<Item = TransactionSubscription>,
    ) -> CoordinatorResult<()> {
        self.check_status(TxnStatus::Open)?;
        self.acked_partitions.extend(subs);
        Ok(())
    }

    /// Upstream `TxnMetaImpl.updateTxnStatus` — uses the
    /// `TransactionUtil.canTransitionTo` state machine and requires the
    /// caller to declare the `expected` status (compare-and-set).
    pub fn update_txn_status(
        &mut self,
        new_status: TxnStatus,
        expected: TxnStatus,
    ) -> CoordinatorResult<()> {
        self.check_status(expected)?;
        if !can_transition_to(self.status, new_status) {
            return Err(CoordinatorError::InvalidTxnStatus {
                txn_id: self.id,
                expected: new_status,
                actual: self.status,
            });
        }
        self.status = new_status;
        Ok(())
    }

    fn check_status(&self, expected: TxnStatus) -> CoordinatorResult<()> {
        if self.status != expected {
            Err(CoordinatorError::InvalidTxnStatus {
                txn_id: self.id,
                expected,
                actual: self.status,
            })
        } else {
            Ok(())
        }
    }
}

// ─── In-memory TransactionMetadataStore ──────────────────────────────────────

/// Upstream parity: `InMemTransactionMetadataStore` — `localID` counter +
/// `ConcurrentHashMap` of `TxnID → TxnMeta`.  We use `Mutex<…>` instead of
/// `ConcurrentHashMap` because the upstream uses fine-grained synchronisation
/// (`synchronized` blocks per-method), which is equivalent to a single big
/// lock as long as transactions don't span the map+meta boundary.
pub struct InMemTransactionMetadataStore {
    tc_id: u64,
    local_id_counter: Mutex<u64>,
    transactions: Mutex<HashMap<TxnId, TxnMeta>>,
    created_count: Mutex<u64>,
    committed_count: Mutex<u64>,
    aborted_count: Mutex<u64>,
    timeout_count: Mutex<u64>,
}

impl InMemTransactionMetadataStore {
    pub fn new(tc_id: u64) -> Self {
        Self {
            tc_id,
            local_id_counter: Mutex::new(0),
            transactions: Mutex::new(HashMap::new()),
            created_count: Mutex::new(0),
            committed_count: Mutex::new(0),
            aborted_count: Mutex::new(0),
            timeout_count: Mutex::new(0),
        }
    }

    pub fn tc_id(&self) -> u64 {
        self.tc_id
    }

    /// Upstream `newTransaction(long timeoutInMills, String owner)`.
    pub fn new_transaction(
        &self,
        timeout_ms: u64,
        owner: Option<String>,
    ) -> CoordinatorResult<TxnId> {
        if let Some(o) = owner.as_ref() {
            if o.is_empty() || o.trim().is_empty() {
                return Err(CoordinatorError::BlankOwner);
            }
        }
        let local = {
            let mut c = self.local_id_counter.lock().unwrap();
            let v = *c;
            *c += 1;
            v
        };
        let id = TxnId::new(self.tc_id, local);
        let now_ms = now_ms();
        let meta = TxnMeta::new(id, now_ms, now_ms.saturating_add(timeout_ms), owner);
        self.transactions.lock().unwrap().insert(id, meta);
        *self.created_count.lock().unwrap() += 1;
        Ok(id)
    }

    pub fn get_txn_meta(&self, txn_id: TxnId) -> CoordinatorResult<TxnMeta> {
        self.transactions
            .lock()
            .unwrap()
            .get(&txn_id)
            .cloned()
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))
    }

    /// Upstream `addProducedPartitionToTxn`.
    pub fn add_produced_partition_to_txn(
        &self,
        txn_id: TxnId,
        partitions: Vec<String>,
    ) -> CoordinatorResult<()> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .get_mut(&txn_id)
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))?;
        meta.add_produced_partitions(partitions)
    }

    /// Upstream `addAckedPartitionToTxn`.
    pub fn add_acked_partition_to_txn(
        &self,
        txn_id: TxnId,
        subs: Vec<TransactionSubscription>,
    ) -> CoordinatorResult<()> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .get_mut(&txn_id)
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))?;
        meta.add_acked_partitions(subs)
    }

    /// Upstream `updateTxnStatus(txnid, newStatus, expectedStatus, isTimeout)`.
    pub fn update_txn_status(
        &self,
        txn_id: TxnId,
        new_status: TxnStatus,
        expected: TxnStatus,
        is_timeout: bool,
    ) -> CoordinatorResult<()> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .get_mut(&txn_id)
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))?;
        meta.update_txn_status(new_status, expected)?;
        // Increment timeout counter when a timeout sweep causes the txn to
        // enter ABORTING (one increment per timed-out txn).  Upstream
        // increments on `expected == ABORTING && is_timeout` (the second
        // call); we choose the equivalent edge on `new == ABORTING` so
        // callers that drive the full abort path don't double-count.
        if is_timeout && new_status == TxnStatus::Aborting {
            *self.timeout_count.lock().unwrap() += 1;
        }
        match new_status {
            TxnStatus::Committed => *self.committed_count.lock().unwrap() += 1,
            TxnStatus::Aborted => *self.aborted_count.lock().unwrap() += 1,
            _ => {}
        }
        Ok(())
    }

    pub fn active_count(&self) -> usize {
        self.transactions.lock().unwrap().len()
    }

    pub fn stats(&self) -> TransactionStats {
        TransactionStats {
            tc_id: self.tc_id,
            actives: self.active_count() as u64,
            created: *self.created_count.lock().unwrap(),
            committed: *self.committed_count.lock().unwrap(),
            aborted: *self.aborted_count.lock().unwrap(),
            timed_out: *self.timeout_count.lock().unwrap(),
        }
    }

    /// Upstream `getSlowTransactions(timeout)` — return txns whose
    /// open_timestamp is older than `now - threshold_ms`.
    pub fn slow_transactions(&self, threshold_ms: u64) -> Vec<TxnMeta> {
        let now = now_ms();
        self.transactions
            .lock()
            .unwrap()
            .values()
            .filter(|m| now.saturating_sub(m.open_timestamp_ms) >= threshold_ms)
            .cloned()
            .collect()
    }

    /// Upstream `closeAsync` — drops the in-memory store.
    pub fn close(&self) {
        self.transactions.lock().unwrap().clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionStats {
    pub tc_id: u64,
    pub actives: u64,
    pub created: u64,
    pub committed: u64,
    pub aborted: u64,
    pub timed_out: u64,
}

// ─── TransactionBuffer ───────────────────────────────────────────────────────

/// Per-topic transaction buffer.  Holds the in-flight transaction entries
/// keyed by sequence number until either `commit_txn` (entries become part
/// of the persistent stream) or `abort_txn` (entries are tombstoned and added
/// to the `AbortedTxnProcessor` set).
///
/// Mirrors `broker/transaction/buffer/{TransactionBuffer,TransactionMeta}.java`
/// + `TopicTransactionBuffer` impl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPosition {
    pub ledger_id: u64,
    pub entry_id: u64,
}

impl EntryPosition {
    pub fn new(ledger_id: u64, entry_id: u64) -> Self {
        Self {
            ledger_id,
            entry_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionBufferEntry {
    pub sequence_id: u64,
    pub position: EntryPosition,
    pub batch_size: u32,
}

#[derive(Debug, Clone)]
pub struct TransactionBufferMeta {
    pub id: TxnId,
    pub status: TxnStatus,
    pub entries: BTreeMap<u64, TransactionBufferEntry>,
    pub committed_at: Option<EntryPosition>,
}

impl TransactionBufferMeta {
    fn new(id: TxnId) -> Self {
        Self {
            id,
            status: TxnStatus::Open,
            entries: BTreeMap::new(),
            committed_at: None,
        }
    }

    pub fn num_entries(&self) -> usize {
        self.entries.len()
    }

    pub fn num_messages(&self) -> u64 {
        self.entries.values().map(|e| e.batch_size as u64).sum()
    }

    pub fn last_sequence_id(&self) -> Option<u64> {
        self.entries.keys().next_back().copied()
    }
}

pub struct TransactionBuffer {
    topic: String,
    transactions: Mutex<HashMap<TxnId, TransactionBufferMeta>>,
    aborted: Mutex<BTreeSet<TxnId>>,
}

impl TransactionBuffer {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            transactions: Mutex::new(HashMap::new()),
            aborted: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn append_entry(
        &self,
        txn_id: TxnId,
        sequence_id: u64,
        position: EntryPosition,
        batch_size: u32,
    ) -> CoordinatorResult<EntryPosition> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .entry(txn_id)
            .or_insert_with(|| TransactionBufferMeta::new(txn_id));
        if meta.status != TxnStatus::Open {
            return Err(CoordinatorError::UnexpectedStatus {
                txn_id,
                status: meta.status,
            });
        }
        meta.entries.insert(
            sequence_id,
            TransactionBufferEntry {
                sequence_id,
                position: position.clone(),
                batch_size,
            },
        );
        Ok(position)
    }

    pub fn committing_txn(&self, txn_id: TxnId) -> CoordinatorResult<TransactionBufferMeta> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .get_mut(&txn_id)
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))?;
        // Mirror `TransactionUtil.canTransitionTo` for the buffer-side.
        if !can_transition_to(meta.status, TxnStatus::Committing) {
            return Err(CoordinatorError::InvalidTxnStatus {
                txn_id,
                expected: TxnStatus::Committing,
                actual: meta.status,
            });
        }
        meta.status = TxnStatus::Committing;
        Ok(meta.clone())
    }

    pub fn commit_txn(
        &self,
        txn_id: TxnId,
        committed_at: EntryPosition,
    ) -> CoordinatorResult<TransactionBufferMeta> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .get_mut(&txn_id)
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))?;
        if !can_transition_to(meta.status, TxnStatus::Committed) {
            return Err(CoordinatorError::InvalidTxnStatus {
                txn_id,
                expected: TxnStatus::Committed,
                actual: meta.status,
            });
        }
        meta.status = TxnStatus::Committed;
        meta.committed_at = Some(committed_at);
        Ok(meta.clone())
    }

    pub fn abort_txn(&self, txn_id: TxnId) -> CoordinatorResult<TransactionBufferMeta> {
        let mut map = self.transactions.lock().unwrap();
        let meta = map
            .get_mut(&txn_id)
            .ok_or(CoordinatorError::TransactionNotFound(txn_id))?;
        // OPEN → ABORTING and ABORTING → ABORTED are both legal.
        let target = match meta.status {
            TxnStatus::Open | TxnStatus::Aborting => TxnStatus::Aborted,
            other => {
                return Err(CoordinatorError::InvalidTxnStatus {
                    txn_id,
                    expected: TxnStatus::Aborted,
                    actual: other,
                });
            }
        };
        // Tombstone path goes via ABORTING first.
        if meta.status == TxnStatus::Open {
            meta.status = TxnStatus::Aborting;
        }
        meta.status = target;
        self.aborted.lock().unwrap().insert(txn_id);
        Ok(meta.clone())
    }

    pub fn is_aborted(&self, txn_id: TxnId) -> bool {
        self.aborted.lock().unwrap().contains(&txn_id)
    }

    pub fn aborted_count(&self) -> usize {
        self.aborted.lock().unwrap().len()
    }

    pub fn read_entries(&self, txn_id: TxnId, start_sequence_id: u64, num: usize) -> Vec<EntryPosition> {
        let map = self.transactions.lock().unwrap();
        let Some(meta) = map.get(&txn_id) else {
            return Vec::new();
        };
        meta.entries
            .range(start_sequence_id..)
            .take(num)
            .map(|(_, e)| e.position.clone())
            .collect()
    }

    pub fn meta(&self, txn_id: TxnId) -> Option<TransactionBufferMeta> {
        self.transactions.lock().unwrap().get(&txn_id).cloned()
    }
}

// ─── AbortedTxnProcessor ─────────────────────────────────────────────────────

/// Mirrors `broker/transaction/buffer/AbortedTxnProcessor.java` (simplified
/// to the snapshot+filter contract — full snapshot persistence is out of scope
/// for the in-memory broker).
pub struct AbortedTxnProcessor {
    aborted: Mutex<BTreeSet<TxnId>>,
}

impl Default for AbortedTxnProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl AbortedTxnProcessor {
    pub fn new() -> Self {
        Self {
            aborted: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn put_aborted_txn(&self, txn_id: TxnId) {
        self.aborted.lock().unwrap().insert(txn_id);
    }

    pub fn contains(&self, txn_id: TxnId) -> bool {
        self.aborted.lock().unwrap().contains(&txn_id)
    }

    /// Used by the dispatcher to skip aborted-txn entries on read.
    pub fn filter_aborted(&self, entries: Vec<(TxnId, EntryPosition)>) -> Vec<EntryPosition> {
        let set = self.aborted.lock().unwrap();
        entries
            .into_iter()
            .filter(|(tx, _)| !set.contains(tx))
            .map(|(_, p)| p)
            .collect()
    }

    pub fn clear(&self) {
        self.aborted.lock().unwrap().clear();
    }

    pub fn len(&self) -> usize {
        self.aborted.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.aborted.lock().unwrap().is_empty()
    }
}

// ─── PendingAckHandle ────────────────────────────────────────────────────────

/// Per-subscription pending-ack store.  When a consumer acknowledges within
/// a transaction (`ackMessageId @ txn_id`) the ack is held in this handle
/// until the transaction commits (then propagated to the cursor) or aborts
/// (then released back to the subscription, redeliverable).
///
/// Mirrors `pulsar-broker/.../transaction/pendingack/impl/PendingAckHandleImpl.java`.
pub struct PendingAckHandle {
    subscription: TransactionSubscription,
    pending: Mutex<HashMap<TxnId, BTreeSet<EntryPositionKey>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryPositionKey(pub u64, pub u64);

impl From<EntryPosition> for EntryPositionKey {
    fn from(p: EntryPosition) -> Self {
        EntryPositionKey(p.ledger_id, p.entry_id)
    }
}

impl PendingAckHandle {
    pub fn new(subscription: TransactionSubscription) -> Self {
        Self {
            subscription,
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn subscription(&self) -> &TransactionSubscription {
        &self.subscription
    }

    /// Stage an ack for `(txn_id, position)`.  Returns the new pending count
    /// for the transaction.
    pub fn individual_ack(&self, txn_id: TxnId, position: EntryPosition) -> usize {
        let mut map = self.pending.lock().unwrap();
        let set = map.entry(txn_id).or_default();
        set.insert(position.into());
        set.len()
    }

    /// Commit the pending acks for `txn_id` — returns the positions that
    /// should now be propagated to the durable subscription cursor.
    pub fn commit(&self, txn_id: TxnId) -> Vec<EntryPosition> {
        let mut map = self.pending.lock().unwrap();
        map.remove(&txn_id)
            .map(|set| {
                set.into_iter()
                    .map(|k| EntryPosition::new(k.0, k.1))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Abort the pending acks for `txn_id` — returns the positions that
    /// should be released back to the subscription for redelivery.
    pub fn abort(&self, txn_id: TxnId) -> Vec<EntryPosition> {
        // Same shape as commit — caller decides what to do with the result.
        // Kept separate so call-sites are explicit + future divergence is easy.
        let mut map = self.pending.lock().unwrap();
        map.remove(&txn_id)
            .map(|set| {
                set.into_iter()
                    .map(|k| EntryPosition::new(k.0, k.1))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn pending_for(&self, txn_id: TxnId) -> usize {
        self.pending
            .lock()
            .unwrap()
            .get(&txn_id)
            .map(|s| s.len())
            .unwrap_or(0)
    }
}

// ─── TransactionTimeoutTracker ───────────────────────────────────────────────

/// Mirrors `broker/transaction/timeout/TransactionTimeoutTrackerImpl.java`.
/// Min-heap keyed by `deadline_ms` — `pop_expired(now)` drains every txn
/// whose deadline has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeadlineEntry {
    deadline_ms: u64,
    txn_id: TxnId,
}

impl Ord for DeadlineEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap on deadline; tie-break on txn_id for determinism.
        other
            .deadline_ms
            .cmp(&self.deadline_ms)
            .then_with(|| other.txn_id.cmp(&self.txn_id))
    }
}

impl PartialOrd for DeadlineEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct TransactionTimeoutTracker {
    heap: Mutex<BinaryHeap<DeadlineEntry>>,
}

impl Default for TransactionTimeoutTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionTimeoutTracker {
    pub fn new() -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::new()),
        }
    }

    pub fn track(&self, txn_id: TxnId, deadline_ms: u64) {
        self.heap.lock().unwrap().push(DeadlineEntry {
            deadline_ms,
            txn_id,
        });
    }

    /// Pop every entry whose deadline ≤ `now`.  Returned in deadline order.
    pub fn pop_expired(&self, now_ms: u64) -> Vec<TxnId> {
        let mut out = Vec::new();
        let mut heap = self.heap.lock().unwrap();
        while let Some(top) = heap.peek() {
            if top.deadline_ms <= now_ms {
                let e = heap.pop().expect("peeked above");
                out.push(e.txn_id);
            } else {
                break;
            }
        }
        out
    }

    pub fn pending(&self) -> usize {
        self.heap.lock().unwrap().len()
    }
}

// ─── TransactionCoordinator orchestrator ─────────────────────────────────────

/// Glues the metadata store, the timeout tracker, and (via callbacks) the
/// per-partition `TransactionBuffer`s + per-subscription `PendingAckHandle`s.
pub struct TransactionCoordinator {
    store: InMemTransactionMetadataStore,
    timeout: TransactionTimeoutTracker,
}

impl TransactionCoordinator {
    pub fn new(tc_id: u64) -> Self {
        Self {
            store: InMemTransactionMetadataStore::new(tc_id),
            timeout: TransactionTimeoutTracker::new(),
        }
    }

    pub fn tc_id(&self) -> u64 {
        self.store.tc_id()
    }

    pub fn store(&self) -> &InMemTransactionMetadataStore {
        &self.store
    }

    pub fn new_transaction(
        &self,
        timeout_ms: u64,
        owner: Option<String>,
    ) -> CoordinatorResult<TxnId> {
        let id = self.store.new_transaction(timeout_ms, owner)?;
        let deadline = self.store.get_txn_meta(id)?.timeout_at_ms;
        self.timeout.track(id, deadline);
        Ok(id)
    }

    pub fn add_partition(&self, txn_id: TxnId, partition: String) -> CoordinatorResult<()> {
        self.store.add_produced_partition_to_txn(txn_id, vec![partition])
    }

    pub fn add_subscription(
        &self,
        txn_id: TxnId,
        subscription: TransactionSubscription,
    ) -> CoordinatorResult<()> {
        self.store
            .add_acked_partition_to_txn(txn_id, vec![subscription])
    }

    /// Begin a commit — `OPEN → COMMITTING`.  Idempotent if already committing.
    pub fn begin_commit(&self, txn_id: TxnId) -> CoordinatorResult<()> {
        // Allow re-entry on Committing.
        let cur = self.store.get_txn_meta(txn_id)?.status;
        let expected = match cur {
            TxnStatus::Open => TxnStatus::Open,
            TxnStatus::Committing => TxnStatus::Committing,
            other => {
                return Err(CoordinatorError::InvalidTxnStatus {
                    txn_id,
                    expected: TxnStatus::Open,
                    actual: other,
                });
            }
        };
        self.store
            .update_txn_status(txn_id, TxnStatus::Committing, expected, false)
    }

    /// Finalise a commit — `COMMITTING → COMMITTED`.
    pub fn end_commit(&self, txn_id: TxnId) -> CoordinatorResult<()> {
        self.store.update_txn_status(
            txn_id,
            TxnStatus::Committed,
            TxnStatus::Committing,
            false,
        )
    }

    /// Begin an abort — `OPEN → ABORTING` (or stay in `ABORTING`).
    pub fn begin_abort(&self, txn_id: TxnId, is_timeout: bool) -> CoordinatorResult<()> {
        let cur = self.store.get_txn_meta(txn_id)?.status;
        let expected = match cur {
            TxnStatus::Open => TxnStatus::Open,
            TxnStatus::Aborting => TxnStatus::Aborting,
            other => {
                return Err(CoordinatorError::InvalidTxnStatus {
                    txn_id,
                    expected: TxnStatus::Open,
                    actual: other,
                });
            }
        };
        self.store
            .update_txn_status(txn_id, TxnStatus::Aborting, expected, is_timeout)
    }

    /// Finalise an abort — `ABORTING → ABORTED`.
    pub fn end_abort(&self, txn_id: TxnId) -> CoordinatorResult<()> {
        self.store
            .update_txn_status(txn_id, TxnStatus::Aborted, TxnStatus::Aborting, false)
    }

    /// Process timeouts at `now_ms`.  Returns the set of txn ids that were
    /// transitioned to `ABORTING` by the timeout sweep (caller drives the
    /// subsequent `ABORTING → ABORTED` once buffer-side cleanup completes).
    pub fn process_timeouts(&self, now_ms: u64) -> Vec<TxnId> {
        let mut aborted = Vec::new();
        for txn_id in self.timeout.pop_expired(now_ms) {
            // Only OPEN txns actually time out; everything else is a tracker
            // leftover from a re-arm and is silently dropped.
            if let Ok(meta) = self.store.get_txn_meta(txn_id) {
                if meta.status == TxnStatus::Open {
                    let _ = self.store.update_txn_status(
                        txn_id,
                        TxnStatus::Aborting,
                        TxnStatus::Open,
                        true,
                    );
                    aborted.push(txn_id);
                }
            }
        }
        aborted
    }

    pub fn stats(&self) -> TransactionStats {
        self.store.stats()
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn open_meta() -> TxnMeta {
        TxnMeta::new(TxnId::new(7, 42), 1_000, 10_000, Some("alice".into()))
    }

    #[test]
    fn txn_id_components() {
        let id = TxnId::new(0x1122334455667788, 0xAABBCCDDEEFF0011);
        assert_eq!(id.tc_id(), 0x1122334455667788);
        assert_eq!(id.local_id(), 0xAABBCCDDEEFF0011);
        let s = format!("{}", id);
        assert!(s.starts_with("(1234605616436508552,"));
        assert!(s.ends_with(&format!("{})", 0xAABBCCDDEEFF0011_u64)));
    }

    #[test]
    fn open_cannot_jump_directly_to_committed_or_aborted() {
        assert!(!can_transition_to(TxnStatus::Open, TxnStatus::Committed));
        assert!(!can_transition_to(TxnStatus::Open, TxnStatus::Aborted));
        assert!(can_transition_to(TxnStatus::Open, TxnStatus::Committing));
        assert!(can_transition_to(TxnStatus::Open, TxnStatus::Aborting));
        assert!(can_transition_to(TxnStatus::Open, TxnStatus::Open));
    }

    #[test]
    fn committing_drains_to_committed_and_is_reentrant() {
        assert!(can_transition_to(TxnStatus::Committing, TxnStatus::Committed));
        assert!(can_transition_to(TxnStatus::Committing, TxnStatus::Committing));
        for terminal in [TxnStatus::Open, TxnStatus::Aborting, TxnStatus::Aborted] {
            assert!(!can_transition_to(TxnStatus::Committing, terminal));
        }
    }

    #[test]
    fn aborting_drains_to_aborted_and_is_reentrant() {
        assert!(can_transition_to(TxnStatus::Aborting, TxnStatus::Aborted));
        assert!(can_transition_to(TxnStatus::Aborting, TxnStatus::Aborting));
        for invalid in [TxnStatus::Open, TxnStatus::Committing, TxnStatus::Committed] {
            assert!(!can_transition_to(TxnStatus::Aborting, invalid));
        }
    }

    #[test]
    fn committed_and_aborted_are_terminal_loops_only() {
        assert!(can_transition_to(TxnStatus::Committed, TxnStatus::Committed));
        assert!(can_transition_to(TxnStatus::Aborted, TxnStatus::Aborted));
        for other in [
            TxnStatus::Open,
            TxnStatus::Committing,
            TxnStatus::Aborting,
            TxnStatus::Aborted,
        ] {
            if other != TxnStatus::Aborted {
                assert!(!can_transition_to(TxnStatus::Committed, other));
            }
        }
        for other in [
            TxnStatus::Open,
            TxnStatus::Committing,
            TxnStatus::Committed,
            TxnStatus::Aborting,
        ] {
            if other != TxnStatus::Committed {
                assert!(!can_transition_to(TxnStatus::Aborted, other));
            }
        }
    }

    #[test]
    fn add_produced_partitions_requires_open_status() {
        let mut meta = open_meta();
        meta.add_produced_partitions(["t/a".into(), "t/b".into()])
            .unwrap();
        assert_eq!(meta.produced_partitions(), vec!["t/a", "t/b"]);
        meta.update_txn_status(TxnStatus::Committing, TxnStatus::Open)
            .unwrap();
        let err = meta
            .add_produced_partitions(["t/c".into()])
            .expect_err("should reject");
        assert!(matches!(err, CoordinatorError::InvalidTxnStatus { .. }));
    }

    #[test]
    fn add_acked_partitions_sorted_and_dedup() {
        let mut meta = open_meta();
        meta.add_acked_partitions(vec![
            TransactionSubscription::new("t/b", "s2"),
            TransactionSubscription::new("t/a", "s1"),
            TransactionSubscription::new("t/a", "s1"),
        ])
        .unwrap();
        let ordered = meta.acked_partitions();
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].topic, "t/a");
        assert_eq!(ordered[1].topic, "t/b");
    }

    #[test]
    fn update_txn_status_enforces_expected() {
        let mut meta = open_meta();
        let err = meta
            .update_txn_status(TxnStatus::Committed, TxnStatus::Committing)
            .expect_err("expected mismatch");
        assert!(matches!(err, CoordinatorError::InvalidTxnStatus { .. }));
        meta.update_txn_status(TxnStatus::Committing, TxnStatus::Open)
            .unwrap();
        meta.update_txn_status(TxnStatus::Committed, TxnStatus::Committing)
            .unwrap();
        assert_eq!(meta.status, TxnStatus::Committed);
    }

    #[test]
    fn in_mem_store_allocates_monotonic_local_ids() {
        let store = InMemTransactionMetadataStore::new(99);
        let a = store.new_transaction(1_000, None).unwrap();
        let b = store.new_transaction(1_000, None).unwrap();
        assert_eq!(a.tc_id(), 99);
        assert_eq!(b.tc_id(), 99);
        assert_eq!(b.local_id(), a.local_id() + 1);
    }

    #[test]
    fn blank_owner_rejected() {
        let store = InMemTransactionMetadataStore::new(99);
        let err = store
            .new_transaction(1_000, Some("   ".into()))
            .expect_err("blank owner");
        assert!(matches!(err, CoordinatorError::BlankOwner));
    }

    #[test]
    fn missing_txn_returns_not_found() {
        let store = InMemTransactionMetadataStore::new(99);
        let err = store
            .get_txn_meta(TxnId::new(1, 1))
            .expect_err("not found");
        assert!(matches!(err, CoordinatorError::TransactionNotFound(_)));
    }

    #[test]
    fn stats_track_counts() {
        let store = InMemTransactionMetadataStore::new(7);
        let id = store.new_transaction(1_000, None).unwrap();
        store
            .update_txn_status(id, TxnStatus::Committing, TxnStatus::Open, false)
            .unwrap();
        store
            .update_txn_status(id, TxnStatus::Committed, TxnStatus::Committing, false)
            .unwrap();
        let s = store.stats();
        assert_eq!(s.created, 1);
        assert_eq!(s.committed, 1);
        assert_eq!(s.aborted, 0);
    }

    #[test]
    fn buffer_append_then_commit() {
        let buf = TransactionBuffer::new("t/topic-0");
        let id = TxnId::new(1, 1);
        buf.append_entry(id, 0, EntryPosition::new(10, 0), 5).unwrap();
        buf.append_entry(id, 1, EntryPosition::new(10, 1), 3).unwrap();
        let m = buf.meta(id).unwrap();
        assert_eq!(m.num_entries(), 2);
        assert_eq!(m.num_messages(), 8);
        assert_eq!(m.last_sequence_id(), Some(1));
        buf.committing_txn(id).unwrap();
        let committed = buf.commit_txn(id, EntryPosition::new(11, 0)).unwrap();
        assert_eq!(committed.status, TxnStatus::Committed);
        assert_eq!(committed.committed_at.unwrap(), EntryPosition::new(11, 0));
    }

    #[test]
    fn buffer_abort_marks_aborted_set() {
        let buf = TransactionBuffer::new("t/topic-1");
        let id = TxnId::new(1, 2);
        buf.append_entry(id, 0, EntryPosition::new(20, 0), 1).unwrap();
        buf.abort_txn(id).unwrap();
        assert!(buf.is_aborted(id));
        assert_eq!(buf.aborted_count(), 1);
    }

    #[test]
    fn buffer_append_after_commit_rejected() {
        let buf = TransactionBuffer::new("t/topic-2");
        let id = TxnId::new(1, 3);
        buf.append_entry(id, 0, EntryPosition::new(30, 0), 1).unwrap();
        buf.committing_txn(id).unwrap();
        buf.commit_txn(id, EntryPosition::new(31, 0)).unwrap();
        let err = buf
            .append_entry(id, 1, EntryPosition::new(31, 1), 1)
            .expect_err("append after commit");
        assert!(matches!(err, CoordinatorError::UnexpectedStatus { .. }));
    }

    #[test]
    fn aborted_processor_filters_entries() {
        let proc_ = AbortedTxnProcessor::new();
        let abort = TxnId::new(5, 1);
        let live = TxnId::new(5, 2);
        proc_.put_aborted_txn(abort);
        let kept = proc_.filter_aborted(vec![
            (live, EntryPosition::new(1, 0)),
            (abort, EntryPosition::new(1, 1)),
            (live, EntryPosition::new(1, 2)),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0], EntryPosition::new(1, 0));
        assert_eq!(kept[1], EntryPosition::new(1, 2));
    }

    #[test]
    fn pending_ack_commit_propagates_positions() {
        let h = PendingAckHandle::new(TransactionSubscription::new("t/x", "s"));
        let id = TxnId::new(9, 1);
        h.individual_ack(id, EntryPosition::new(1, 1));
        h.individual_ack(id, EntryPosition::new(1, 2));
        h.individual_ack(id, EntryPosition::new(1, 1)); // dedup
        assert_eq!(h.pending_for(id), 2);
        let committed = h.commit(id);
        assert_eq!(committed.len(), 2);
        assert_eq!(h.pending_for(id), 0);
    }

    #[test]
    fn pending_ack_abort_returns_positions_for_redeliver() {
        let h = PendingAckHandle::new(TransactionSubscription::new("t/x", "s"));
        let id = TxnId::new(9, 2);
        h.individual_ack(id, EntryPosition::new(2, 0));
        let released = h.abort(id);
        assert_eq!(released.len(), 1);
        assert_eq!(h.pending_for(id), 0);
    }

    #[test]
    fn timeout_tracker_pops_in_deadline_order() {
        let t = TransactionTimeoutTracker::new();
        t.track(TxnId::new(1, 3), 30);
        t.track(TxnId::new(1, 1), 10);
        t.track(TxnId::new(1, 2), 20);
        let expired = t.pop_expired(20);
        assert_eq!(expired.len(), 2);
        assert_eq!(expired[0].local_id(), 1);
        assert_eq!(expired[1].local_id(), 2);
        assert_eq!(t.pending(), 1);
    }

    #[test]
    fn coordinator_full_commit_path() {
        let tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(60_000, Some("alice".into())).unwrap();
        tc.add_partition(id, "t/p0".into()).unwrap();
        tc.add_subscription(id, TransactionSubscription::new("t/p0", "s1"))
            .unwrap();
        tc.begin_commit(id).unwrap();
        tc.end_commit(id).unwrap();
        assert_eq!(tc.store().get_txn_meta(id).unwrap().status, TxnStatus::Committed);
        assert_eq!(tc.stats().committed, 1);
    }

    #[test]
    fn coordinator_full_abort_path() {
        let tc = TransactionCoordinator::new(2);
        let id = tc.new_transaction(60_000, None).unwrap();
        tc.begin_abort(id, false).unwrap();
        tc.end_abort(id).unwrap();
        assert_eq!(tc.store().get_txn_meta(id).unwrap().status, TxnStatus::Aborted);
        assert_eq!(tc.stats().aborted, 1);
    }

    #[test]
    fn coordinator_commit_is_idempotent_in_committing() {
        let tc = TransactionCoordinator::new(3);
        let id = tc.new_transaction(60_000, None).unwrap();
        tc.begin_commit(id).unwrap();
        tc.begin_commit(id).unwrap(); // second call OK (Committing → Committing)
        tc.end_commit(id).unwrap();
        // Idempotent end_commit on Committed should ERR per upstream
        // (Committed → Committing not allowed).
        assert!(tc.end_commit(id).is_err());
    }

    #[test]
    fn process_timeouts_aborts_only_open_txns() {
        let tc = TransactionCoordinator::new(4);
        let id_a = tc.new_transaction(0, None).unwrap();
        let _id_b = tc.new_transaction(0, None).unwrap();
        // Commit one of them first.
        tc.begin_commit(id_a).unwrap();
        tc.end_commit(id_a).unwrap();
        // Now fire timeout sweep — only the still-OPEN txn should abort.
        let aborted = tc.process_timeouts(u64::MAX);
        assert_eq!(aborted.len(), 1);
        assert_ne!(aborted[0], id_a);
        assert_eq!(tc.stats().timed_out, 1);
    }

    #[test]
    fn close_clears_in_mem_store() {
        let store = InMemTransactionMetadataStore::new(7);
        store.new_transaction(1, None).unwrap();
        assert_eq!(store.active_count(), 1);
        store.close();
        assert_eq!(store.active_count(), 0);
    }

    #[test]
    fn read_entries_returns_in_sequence_order() {
        let buf = TransactionBuffer::new("t");
        let id = TxnId::new(1, 1);
        buf.append_entry(id, 5, EntryPosition::new(0, 5), 1).unwrap();
        buf.append_entry(id, 3, EntryPosition::new(0, 3), 1).unwrap();
        buf.append_entry(id, 9, EntryPosition::new(0, 9), 1).unwrap();
        let positions = buf.read_entries(id, 4, 10);
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0], EntryPosition::new(0, 5));
        assert_eq!(positions[1], EntryPosition::new(0, 9));
    }

    #[test]
    fn slow_transactions_picks_long_running() {
        let store = InMemTransactionMetadataStore::new(8);
        let _id = store.new_transaction(60_000, None).unwrap();
        let slow = store.slow_transactions(0);
        assert_eq!(slow.len(), 1);
        let none = store.slow_transactions(60_000_000);
        assert!(none.is_empty());
    }
}
