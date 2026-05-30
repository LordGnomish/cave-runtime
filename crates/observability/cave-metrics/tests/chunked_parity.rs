// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for the chunked remote-read streaming framing, ported from
//! prometheus/prometheus `storage/remote/chunked_test.go` (v3.12.0,
//! source_sha a0524eeca91b19eb60d2b02f8a1c0019954e3405).
//!
//! Validates `ChunkedWriter` → `ChunkedReader` roundtrips, multi-frame
//! ordering, empty-frame handling, CRC32C integrity detection, and the
//! CRC32C check value (`STREAMED_XOR_CHUNKS` transport).

use cave_metrics::ingestion::chunked::{ChunkedReader, ChunkedWriter, crc32c};

#[test]
fn single_frame_roundtrip() {
    let mut w = ChunkedWriter::new();
    w.write(b"hello world");
    let bytes = w.into_bytes();

    let mut r = ChunkedReader::new(&bytes);
    assert_eq!(r.next().unwrap(), Some(b"hello world".to_vec()));
    assert_eq!(r.next().unwrap(), None);
}

#[test]
fn multi_frame_preserves_order() {
    let mut w = ChunkedWriter::new();
    w.write(b"first");
    w.write(b"second");
    w.write(b"third");
    let bytes = w.into_bytes();

    let mut r = ChunkedReader::new(&bytes);
    let all = r.read_all().unwrap();
    assert_eq!(
        all,
        vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
    );
}

#[test]
fn empty_frame_roundtrips() {
    let mut w = ChunkedWriter::new();
    w.write(b"");
    w.write(b"after-empty");
    let bytes = w.into_bytes();

    let mut r = ChunkedReader::new(&bytes);
    assert_eq!(r.next().unwrap(), Some(Vec::new()));
    assert_eq!(r.next().unwrap(), Some(b"after-empty".to_vec()));
    assert_eq!(r.next().unwrap(), None);
}

#[test]
fn large_frame_with_varint_length_prefix() {
    // 300 bytes forces a 2-byte uvarint length prefix.
    let payload = vec![0xABu8; 300];
    let mut w = ChunkedWriter::new();
    w.write(&payload);
    let bytes = w.into_bytes();

    let mut r = ChunkedReader::new(&bytes);
    assert_eq!(r.next().unwrap(), Some(payload));
}

#[test]
fn crc_mismatch_is_detected() {
    let mut w = ChunkedWriter::new();
    w.write(b"payload-under-protection");
    let mut bytes = w.into_bytes();

    // Corrupt the last payload byte; the CRC32C check must reject it.
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;

    let mut r = ChunkedReader::new(&bytes);
    assert!(r.next().is_err(), "corrupted frame must fail CRC verification");
}

#[test]
fn truncated_stream_errors() {
    let mut w = ChunkedWriter::new();
    w.write(b"complete-frame");
    let bytes = w.into_bytes();
    // Drop the trailing payload bytes.
    let truncated = &bytes[..bytes.len() - 3];

    let mut r = ChunkedReader::new(truncated);
    assert!(r.next().is_err());
}

#[test]
fn crc32c_castagnoli_check_value() {
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
}
