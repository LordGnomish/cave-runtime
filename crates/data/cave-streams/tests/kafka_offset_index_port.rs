// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Upstream behavioural-parity port — Apache Kafka 4.2.0
//! `org.apache.kafka.storage.internals.log.OffsetIndex` /
//! `AbstractIndex.largestLowerBoundSlotFor`.
//!
//! Each log segment keeps a sparse `.index` file mapping a subset of message
//! offsets to byte positions.  A Fetch at offset T binary-searches the index
//! for the largest indexed offset ≤ T and starts the on-disk scan there.
//! Our SegmentLog tracked entries but had no sparse offset index; this port
//! adds `OffsetIndex` with Kafka's exact lookup contract:
//!   * an empty index, or a target below the first entry, floors to
//!     `(base_offset, 0)` — the start of the segment;
//!   * otherwise it returns the entry with the largest offset ≤ target;
//!   * appends must be strictly increasing in offset (Kafka rejects
//!     out-of-order index entries).

use cave_streams::segment_log::OffsetIndex;

#[test]
fn empty_index_floors_to_base_position_zero() {
    let idx = OffsetIndex::new(50);
    assert!(idx.is_empty());
    assert_eq!(idx.lookup(0), (50, 0));
    assert_eq!(idx.lookup(999), (50, 0));
}

#[test]
fn lookup_returns_largest_offset_not_exceeding_target() {
    let mut idx = OffsetIndex::new(0);
    idx.append(10, 0).unwrap();
    idx.append(20, 100).unwrap();
    idx.append(30, 250).unwrap();

    // Exact hits.
    assert_eq!(idx.lookup(10), (10, 0));
    assert_eq!(idx.lookup(20), (20, 100));
    assert_eq!(idx.lookup(30), (30, 250));

    // Between entries → floor to the lower neighbour.
    assert_eq!(idx.lookup(25), (20, 100));
    assert_eq!(idx.lookup(29), (20, 100));

    // Past the last entry → the last entry.
    assert_eq!(idx.lookup(35), (30, 250));
    assert_eq!(idx.lookup(1_000), (30, 250));
}

#[test]
fn target_below_first_entry_floors_to_base() {
    let mut idx = OffsetIndex::new(100);
    idx.append(110, 0).unwrap();
    idx.append(140, 512).unwrap();
    // 105 is in the segment but before the first *indexed* offset → base/0.
    assert_eq!(idx.lookup(105), (100, 0));
    assert_eq!(idx.lookup(100), (100, 0));
}

#[test]
fn append_rejects_non_increasing_offsets() {
    let mut idx = OffsetIndex::new(0);
    idx.append(10, 0).unwrap();
    assert!(idx.append(10, 50).is_err(), "duplicate offset must be rejected");
    assert!(idx.append(5, 50).is_err(), "backwards offset must be rejected");
    idx.append(11, 50).unwrap();
    assert_eq!(idx.len(), 2);
}

#[test]
fn append_rejects_offset_below_base() {
    let mut idx = OffsetIndex::new(100);
    assert!(idx.append(99, 0).is_err(), "offset below base must be rejected");
    idx.append(100, 0).unwrap();
}
