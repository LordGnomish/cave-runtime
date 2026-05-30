// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's write-ahead log
// (src/backend/access/transam/{xlog.c,xloginsert.c}, src/common/pg_crc32c*).
//
// Faithful behaviours asserted:
//   * the insert pointer (LSN / XLogRecPtr) is a monotonically increasing byte
//     offset that advances by each serialized record's total length
//   * every record stores the previous record's LSN (xl_prev) forming a chain
//   * records carry a CRC-32C (Castagnoli) over their header+payload, matching
//     the standard check value for the "123456789" vector
//   * replay walks the records in LSN order
//   * a corrupted payload is detected by CRC verification

use cave_rdbms::storage::wal::{crc32c, Wal};

#[test]
fn crc32c_matches_standard_vector() {
    // The canonical CRC-32C (Castagnoli) check value for ASCII "123456789".
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    assert_eq!(crc32c(b""), 0);
}

#[test]
fn lsn_is_monotonic_byte_offset_with_prev_chain() {
    let mut wal = Wal::new();
    let start = wal.insert_lsn();
    let l1 = wal.append(100, 1, 0, b"first".to_vec());
    let l2 = wal.append(100, 1, 0, b"second-record".to_vec());
    let l3 = wal.append(101, 2, 0, b"x".to_vec());

    assert_eq!(l1, start, "first record sits at the starting LSN");
    assert!(l2 > l1 && l3 > l2, "LSN must strictly increase");

    let recs = wal.records();
    assert_eq!(recs.len(), 3);
    // LSN advances by the previous record's serialized length
    assert_eq!(recs[1].lsn - recs[0].lsn, recs[0].total_len());
    assert_eq!(recs[2].lsn - recs[1].lsn, recs[1].total_len());
    // xl_prev chain
    assert_eq!(recs[0].prev, start);
    assert_eq!(recs[1].prev, recs[0].lsn);
    assert_eq!(recs[2].prev, recs[1].lsn);
    // insert pointer now past the last record
    assert_eq!(wal.insert_lsn(), recs[2].lsn + recs[2].total_len());
}

#[test]
fn replay_visits_records_in_lsn_order() {
    let mut wal = Wal::new();
    wal.append(1, 1, 0, b"a".to_vec());
    wal.append(2, 1, 0, b"bb".to_vec());
    wal.append(3, 1, 0, b"ccc".to_vec());

    let mut seen: Vec<(u32, usize)> = Vec::new();
    let mut last_lsn = 0u64;
    wal.replay(|r| {
        assert!(r.lsn >= last_lsn, "replay out of LSN order");
        last_lsn = r.lsn;
        seen.push((r.xid, r.data.len()));
    });
    assert_eq!(seen, vec![(1, 1), (2, 2), (3, 3)]);
}

#[test]
fn intact_log_verifies_and_corruption_is_detected() {
    let mut wal = Wal::new();
    wal.append(1, 1, 0, b"hello".to_vec());
    wal.append(1, 1, 0, b"world".to_vec());
    assert!(wal.verify(), "freshly written log must verify");

    // Flip a payload byte without updating the stored CRC.
    wal.records_mut()[0].data[0] ^= 0xFF;
    assert!(!wal.verify(), "CRC must catch the corrupted payload");
}
