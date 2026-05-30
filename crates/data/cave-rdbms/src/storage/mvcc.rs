// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MVCC tuple visibility.
//!
//! Port of PostgreSQL's multi-version concurrency control core:
//!   * `src/backend/access/heap/heapam_visibility.c` — `HeapTupleSatisfiesMVCC`
//!   * `src/backend/storage/ipc/procarray.c` — `XidInMVCCSnapshot`, snapshot build
//!   * the commit-log (`clog`) transaction status
//!
//! Every heap tuple version stores `xmin` (the inserting transaction) and
//! `xmax` (the deleting/locking transaction, or [`INVALID_XID`] while live). A
//! version is visible to a [`Snapshot`] when its inserter committed and was
//! already visible at snapshot time, and its deletion is not yet visible.

use std::collections::HashMap;

/// `TransactionId` (`xid`).
pub type Xid = u32;

/// `InvalidTransactionId` — an unset `xmax` means the tuple is live.
pub const INVALID_XID: Xid = 0;

/// CLOG transaction outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XactStatus {
    InProgress,
    Committed,
    Aborted,
}

/// Commit log: maps a transaction id to its final status. Unknown xids are
/// treated as still in progress (`TransactionIdIsInProgress`).
#[derive(Default)]
pub struct Clog {
    status: HashMap<Xid, XactStatus>,
}

impl Clog {
    pub fn new() -> Self {
        Clog {
            status: HashMap::new(),
        }
    }

    pub fn commit(&mut self, xid: Xid) {
        self.status.insert(xid, XactStatus::Committed);
    }

    pub fn abort(&mut self, xid: Xid) {
        self.status.insert(xid, XactStatus::Aborted);
    }

    pub fn status(&self, xid: Xid) -> XactStatus {
        self.status
            .get(&xid)
            .copied()
            .unwrap_or(XactStatus::InProgress)
    }
}

/// An MVCC snapshot (`SnapshotData`): the transaction-visibility horizon taken
/// at a point in time.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// oldest xid still running when the snapshot was taken
    pub xmin: Xid,
    /// first xid not yet assigned (exclusive upper bound)
    pub xmax: Xid,
    /// xids that were in progress at snapshot time
    pub xip: Vec<Xid>,
}

impl Snapshot {
    /// `GetSnapshotData`: build a snapshot from the next unassigned xid and the
    /// set of currently-running xids.
    pub fn take(next_xid: Xid, active: &[Xid]) -> Self {
        let xmin = active.iter().copied().min().unwrap_or(next_xid);
        Snapshot {
            xmin,
            xmax: next_xid,
            xip: active.to_vec(),
        }
    }

    /// `XidInMVCCSnapshot`: was `xid` still in progress as of this snapshot?
    /// `xid >= xmax` had not started; `xid < xmin` had already completed;
    /// otherwise it is in progress iff it appears in the in-flight list.
    pub fn xid_in_progress(&self, xid: Xid) -> bool {
        if xid >= self.xmax {
            return true;
        }
        if xid < self.xmin {
            return false;
        }
        self.xip.contains(&xid)
    }
}

/// A heap tuple version header (the subset MVCC needs).
#[derive(Debug, Clone)]
pub struct HeapTuple {
    /// inserting transaction (`t_xmin`)
    pub xmin: Xid,
    /// deleting/locking transaction (`t_xmax`), or [`INVALID_XID`]
    pub xmax: Xid,
    /// opaque tuple payload
    pub data: Vec<u8>,
}

/// `HeapTupleSatisfiesMVCC`: is `tuple` visible to `snap` given commit log
/// `clog`?
pub fn satisfies_mvcc(tuple: &HeapTuple, snap: &Snapshot, clog: &Clog) -> bool {
    // ── Inserter must be committed and visible to the snapshot. ──
    match clog.status(tuple.xmin) {
        XactStatus::Aborted | XactStatus::InProgress => return false,
        XactStatus::Committed => {}
    }
    if snap.xid_in_progress(tuple.xmin) {
        // committed, but was still running when the snapshot was taken
        return false;
    }

    // ── The tuple is alive unless its deletion is committed *and* visible. ──
    if tuple.xmax == INVALID_XID {
        return true;
    }
    match clog.status(tuple.xmax) {
        // delete committed → row dead only if that delete is visible now
        XactStatus::Committed => snap.xid_in_progress(tuple.xmax),
        // delete rolled back or still running → row remains alive
        XactStatus::Aborted | XactStatus::InProgress => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_xid_is_in_progress() {
        let clog = Clog::new();
        assert_eq!(clog.status(99), XactStatus::InProgress);
    }

    #[test]
    fn empty_active_set_makes_xmin_equal_xmax() {
        let snap = Snapshot::take(7, &[]);
        assert_eq!(snap.xmin, 7);
        assert_eq!(snap.xmax, 7);
        // everything below the horizon is complete
        assert!(!snap.xid_in_progress(6));
        assert!(snap.xid_in_progress(7));
    }
}
