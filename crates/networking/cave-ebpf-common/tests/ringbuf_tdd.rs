// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — BPF ring-buffer reader (userspace model of
// BPF_MAP_TYPE_RINGBUF, the transport Beyla uses to ship events from
// kernel probes to userspace). Models reserve/commit/discard on the
// producer side and the strictly in-order consumer that cannot pass a
// record still being written (the "busy" record).

use cave_ebpf_common::ringbuf::{RingBuf, RingBufError};

#[test]
fn test_reserve_commit_consume_roundtrip() {
    let mut rb = RingBuf::new(4096);
    let t = rb.reserve(b"hello".to_vec()).unwrap();
    rb.commit(t);
    let recs = rb.consume();
    assert_eq!(recs, vec![b"hello".to_vec()]);
    // Drained: a second consume yields nothing.
    assert!(rb.consume().is_empty());
}

#[test]
fn test_discarded_record_is_skipped() {
    let mut rb = RingBuf::new(4096);
    let a = rb.reserve(b"keep".to_vec()).unwrap();
    let b = rb.reserve(b"drop".to_vec()).unwrap();
    let c = rb.reserve(b"keep2".to_vec()).unwrap();
    rb.commit(a);
    rb.discard(b);
    rb.commit(c);
    assert_eq!(rb.consume(), vec![b"keep".to_vec(), b"keep2".to_vec()]);
}

#[test]
fn test_consumer_blocks_behind_busy_record() {
    // In-order: if the front record is still reserved (busy), the
    // consumer cannot advance even though a later record is committed.
    let mut rb = RingBuf::new(4096);
    let a = rb.reserve(b"first".to_vec()).unwrap();
    let b = rb.reserve(b"second".to_vec()).unwrap();
    rb.commit(b); // commit the later one first
    assert!(rb.consume().is_empty(), "blocked behind busy front");
    rb.commit(a);
    assert_eq!(rb.consume(), vec![b"first".to_vec(), b"second".to_vec()]);
}

#[test]
fn test_reserve_fails_when_full() {
    let mut rb = RingBuf::new(64);
    // Each record carries an 8-byte header. Fill the buffer.
    let _ = rb.reserve(vec![0u8; 24]).unwrap(); // 32 bytes
    let _ = rb.reserve(vec![0u8; 24]).unwrap(); // 32 bytes -> full at 64
    assert_eq!(
        rb.reserve(vec![0u8; 1]).err(),
        Some(RingBufError::Full)
    );
}

#[test]
fn test_consume_reclaims_space_for_new_reserve() {
    let mut rb = RingBuf::new(64);
    let a = rb.reserve(vec![0u8; 24]).unwrap();
    let b = rb.reserve(vec![0u8; 24]).unwrap();
    assert!(rb.reserve(vec![0u8; 1]).is_err());
    rb.commit(a);
    rb.commit(b);
    rb.consume(); // frees both
    // Now there is room again.
    assert!(rb.reserve(vec![0u8; 24]).is_ok());
}

#[test]
fn test_payload_larger_than_capacity_rejected() {
    let mut rb = RingBuf::new(32);
    assert_eq!(
        rb.reserve(vec![0u8; 100]).err(),
        Some(RingBufError::Full)
    );
}
