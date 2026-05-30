// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's MVCC tuple visibility
// (src/backend/access/heap/heapam_visibility.c HeapTupleSatisfiesMVCC,
//  src/backend/storage/ipc/procarray.c XidInMVCCSnapshot, the CLOG xact status).
//
// A heap tuple version carries (xmin = inserting xid, xmax = deleting xid). It
// is visible to a snapshot iff the inserter committed and was visible at
// snapshot time, AND the deletion is not yet visible. Faithful behaviours:
//   * XidInMVCCSnapshot boundary rules (>= xmax in-progress; < xmin completed)
//   * aborted / in-progress inserter → invisible
//   * committed-and-visible deleter → invisible (row deleted)
//   * in-progress / aborted / not-yet-visible deleter → row still alive
//   * a fresh snapshot sees committed work and hides concurrent work

use cave_rdbms::storage::mvcc::{satisfies_mvcc, Clog, HeapTuple, Snapshot, INVALID_XID};

fn tuple(xmin: u32, xmax: u32) -> HeapTuple {
    HeapTuple {
        xmin,
        xmax,
        data: vec![],
    }
}

#[test]
fn xid_in_mvcc_snapshot_boundaries() {
    // snapshot: xmin=10 (oldest running), xmax=20 (next xid), in-progress {12,15}
    let snap = Snapshot {
        xmin: 10,
        xmax: 20,
        xip: vec![12, 15],
    };
    assert!(snap.xid_in_progress(20), ">= xmax counts as in-progress");
    assert!(snap.xid_in_progress(25));
    assert!(!snap.xid_in_progress(9), "< xmin already completed");
    assert!(snap.xid_in_progress(12), "listed in xip");
    assert!(!snap.xid_in_progress(13), "between bounds, not listed → completed");
}

#[test]
fn committed_insert_no_delete_is_visible() {
    let mut clog = Clog::new();
    clog.commit(5);
    let snap = Snapshot {
        xmin: 10,
        xmax: 10,
        xip: vec![],
    };
    assert!(satisfies_mvcc(&tuple(5, INVALID_XID), &snap, &clog));
}

#[test]
fn aborted_or_inprogress_inserter_is_invisible() {
    let mut clog = Clog::new();
    clog.abort(5);
    // xid 7 left in-progress (unknown to clog)
    let snap = Snapshot {
        xmin: 10,
        xmax: 10,
        xip: vec![],
    };
    assert!(!satisfies_mvcc(&tuple(5, INVALID_XID), &snap, &clog));
    assert!(!satisfies_mvcc(&tuple(7, INVALID_XID), &snap, &clog));
}

#[test]
fn committed_inserter_running_at_snapshot_is_invisible() {
    let mut clog = Clog::new();
    clog.commit(12); // committed *after* the snapshot was taken
    let snap = Snapshot {
        xmin: 10,
        xmax: 20,
        xip: vec![12],
    };
    assert!(!satisfies_mvcc(&tuple(12, INVALID_XID), &snap, &clog));
}

#[test]
fn deleted_row_visibility_depends_on_deleter_status() {
    let mut clog = Clog::new();
    clog.commit(5); // inserter, visible
    let snap = Snapshot {
        xmin: 10,
        xmax: 20,
        xip: vec![15],
    };

    // deleter committed and visible (< xmin) → row deleted → invisible
    clog.commit(6);
    assert!(!satisfies_mvcc(&tuple(5, 6), &snap, &clog));

    // deleter aborted → delete rolled back → row alive
    clog.abort(7);
    assert!(satisfies_mvcc(&tuple(5, 7), &snap, &clog));

    // deleter committed but was in-progress at snapshot → delete not visible → alive
    clog.commit(15);
    assert!(satisfies_mvcc(&tuple(5, 15), &snap, &clog));

    // deleter still in-progress (unknown) → alive
    assert!(satisfies_mvcc(&tuple(5, 18), &snap, &clog));
}

#[test]
fn snapshot_take_captures_active_xids() {
    // next xid 21, active set {15, 18} → xmin=15, xmax=21
    let snap = Snapshot::take(21, &[18, 15]);
    assert_eq!(snap.xmin, 15);
    assert_eq!(snap.xmax, 21);
    assert!(snap.xid_in_progress(15) && snap.xid_in_progress(18));
    assert!(!snap.xid_in_progress(14));
}
