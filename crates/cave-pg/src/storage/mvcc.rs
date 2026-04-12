//! Multi-Version Concurrency Control (MVCC) for the cave-pg storage engine.
//!
//! Each tuple carries xmin (creating transaction) and xmax (deleting transaction).
//! A snapshot is taken at the start of each transaction and determines which
//! tuple versions are visible.
//!
//! Isolation levels supported:
//!   - READ COMMITTED  — snapshot per statement
//!   - REPEATABLE READ — snapshot per transaction
//!   - SERIALIZABLE    — serializable snapshot isolation (SSI, via conflict detection)

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::{Mutex, RwLock};

/// Transaction ID. 0 = invalid/bootstrap.
pub type Xid = u64;
pub const XID_INVALID: Xid = 0;
pub const XID_BOOTSTRAP: Xid = 1;

/// Isolation level for a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

/// The status of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XidStatus {
    InProgress,
    Committed,
    Aborted,
    SubCommitted,
}

/// A snapshot of the transaction state at a point in time.
/// A tuple version is visible if:
///   xmin committed before snapshot AND (xmax == 0 OR xmax not committed before snapshot)
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The transaction XID when this snapshot was taken.
    pub taken_by: Xid,
    /// The next unassigned XID at snapshot time.
    pub xmax: Xid,
    /// Set of XIDs that are in-progress at snapshot time.
    pub xip: HashSet<Xid>,
    /// XIDs committed before xmax that are in xip (should be empty in practice).
    pub subxip: HashSet<Xid>,
}

impl Snapshot {
    /// Create a snapshot for a given transaction.
    pub fn new(taken_by: Xid, xmax: Xid, xip: HashSet<Xid>) -> Self {
        Self { taken_by, xmax, xip, subxip: HashSet::new() }
    }

    /// Is the given XID visible in this snapshot?
    pub fn xid_visible(&self, xid: Xid, clog: &CommitLog) -> bool {
        if xid == XID_INVALID { return false; }
        if xid == XID_BOOTSTRAP { return true; }
        if xid >= self.xmax { return false; } // future transaction
        if self.xip.contains(&xid) { return false; } // still in-progress when snapshot taken
        // Check commit log
        clog.is_committed(xid)
    }

    /// Visibility check for a tuple: is xmin visible and xmax not visible?
    pub fn tuple_visible(&self, xmin: Xid, xmax: Xid, clog: &CommitLog) -> bool {
        if !self.xid_visible(xmin, clog) { return false; }
        if xmax == XID_INVALID { return true; }
        if xmax == self.taken_by { return false; } // deleted by this transaction
        !self.xid_visible(xmax, clog)
    }
}

/// The commit log — tracks which transactions have committed or aborted.
/// In a real PostgreSQL this is pg_xact (formerly pg_clog), stored on disk.
/// Here we use an in-memory map.
#[derive(Debug, Default)]
pub struct CommitLog {
    /// XID → committed status
    committed: RwLock<HashMap<Xid, bool>>,  // true = committed, false = aborted
}

impl CommitLog {
    pub fn is_committed(&self, xid: Xid) -> bool {
        if xid == XID_BOOTSTRAP { return true; }
        self.committed.read().get(&xid).copied().unwrap_or(false)
    }

    pub fn is_aborted(&self, xid: Xid) -> bool {
        if xid == XID_BOOTSTRAP { return false; }
        self.committed.read().get(&xid).map(|&v| !v).unwrap_or(false)
    }

    pub fn mark_committed(&self, xid: Xid) {
        self.committed.write().insert(xid, true);
    }

    pub fn mark_aborted(&self, xid: Xid) {
        self.committed.write().insert(xid, false);
    }

    pub fn status(&self, xid: Xid) -> XidStatus {
        match self.committed.read().get(&xid) {
            None => XidStatus::InProgress,
            Some(true) => XidStatus::Committed,
            Some(false) => XidStatus::Aborted,
        }
    }
}

/// Write-ahead log entry.
#[derive(Debug, Clone)]
pub struct WalEntry {
    pub lsn: u64,
    pub xid: Xid,
    pub operation: WalOperation,
}

#[derive(Debug, Clone)]
pub enum WalOperation {
    BeginTransaction { isolation: IsolationLevel },
    CommitTransaction,
    AbortTransaction,
    InsertTuple { schema: String, table: String, ctid: u64, data: Vec<u8> },
    UpdateTuple { schema: String, table: String, old_ctid: u64, new_ctid: u64, data: Vec<u8> },
    DeleteTuple { schema: String, table: String, ctid: u64 },
    CreateTable { schema: String, table: String, columns: Vec<u8> },
    DropTable { schema: String, table: String },
    Savepoint { name: String },
    ReleaseSavepoint { name: String },
    RollbackToSavepoint { name: String },
}

/// The write-ahead log — append-only.
#[derive(Debug, Default)]
pub struct Wal {
    entries: Mutex<Vec<WalEntry>>,
    lsn_counter: AtomicU64,
}

impl Wal {
    pub fn append(&self, xid: Xid, operation: WalOperation) -> u64 {
        let lsn = self.lsn_counter.fetch_add(1, Ordering::SeqCst);
        self.entries.lock().push(WalEntry { lsn, xid, operation });
        lsn
    }

    pub fn current_lsn(&self) -> u64 {
        self.lsn_counter.load(Ordering::SeqCst)
    }

    /// Read all entries from a given LSN (for replication / crash recovery).
    pub fn read_from(&self, start_lsn: u64) -> Vec<WalEntry> {
        self.entries.lock()
            .iter()
            .filter(|e| e.lsn >= start_lsn)
            .cloned()
            .collect()
    }
}

/// Manages all active transactions and provides snapshot isolation.
#[derive(Debug)]
pub struct TransactionManager {
    /// Next transaction ID to assign.
    xid_counter: AtomicU64,
    /// Active transactions (xid → transaction state).
    active: RwLock<HashMap<Xid, TransactionState>>,
    /// The commit log.
    pub clog: Arc<CommitLog>,
    /// The WAL.
    pub wal: Arc<Wal>,
}

/// State of a single transaction.
#[derive(Debug, Clone)]
pub struct TransactionState {
    pub xid: Xid,
    pub isolation: IsolationLevel,
    pub snapshot: Snapshot,
    /// Savepoints: stack of (name, snapshot).
    pub savepoints: Vec<(String, Snapshot)>,
    /// Tables locked by this transaction (schema.table).
    pub row_locks: HashSet<String>,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            xid_counter: AtomicU64::new(2), // 0 = invalid, 1 = bootstrap
            active: RwLock::new(HashMap::new()),
            clog: Arc::new(CommitLog::default()),
            wal: Arc::new(Wal::default()),
        }
    }

    /// Start a new transaction.
    pub fn begin(&self, isolation: IsolationLevel) -> TransactionState {
        let xid = self.xid_counter.fetch_add(1, Ordering::SeqCst);
        let snapshot = self.take_snapshot(xid);
        self.wal.append(xid, WalOperation::BeginTransaction { isolation });

        let state = TransactionState {
            xid,
            isolation,
            snapshot,
            savepoints: Vec::new(),
            row_locks: HashSet::new(),
        };
        self.active.write().insert(xid, state.clone());
        state
    }

    /// Take a snapshot of the current in-progress XIDs.
    pub fn take_snapshot(&self, for_xid: Xid) -> Snapshot {
        let active = self.active.read();
        let xip: HashSet<Xid> = active.keys().copied().collect();
        let xmax = self.xid_counter.load(Ordering::SeqCst);
        Snapshot::new(for_xid, xmax, xip)
    }

    /// Commit a transaction.
    pub fn commit(&self, xid: Xid) {
        self.wal.append(xid, WalOperation::CommitTransaction);
        self.clog.mark_committed(xid);
        self.active.write().remove(&xid);
    }

    /// Abort/rollback a transaction.
    pub fn abort(&self, xid: Xid) {
        self.wal.append(xid, WalOperation::AbortTransaction);
        self.clog.mark_aborted(xid);
        self.active.write().remove(&xid);
    }

    /// Get current snapshot for an XID (for READ COMMITTED — refreshes per statement).
    pub fn statement_snapshot(&self, xid: Xid) -> Snapshot {
        self.take_snapshot(xid)
    }

    /// Get the transaction's existing snapshot (for REPEATABLE READ / SERIALIZABLE).
    pub fn transaction_snapshot(&self, xid: Xid) -> Option<Snapshot> {
        self.active.read().get(&xid).map(|s| s.snapshot.clone())
    }

    pub fn is_active(&self, xid: Xid) -> bool {
        self.active.read().contains_key(&xid)
    }

    pub fn active_xids(&self) -> Vec<Xid> {
        self.active.read().keys().copied().collect()
    }
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// A row lock record — prevents concurrent modifications to the same row.
#[derive(Debug, Clone)]
pub struct RowLock {
    pub xid: Xid,
    pub ctid: u64,
    pub mode: LockMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    ForShare,
    ForUpdate,
    ForNoKeyUpdate,
    ForKeyShare,
}
