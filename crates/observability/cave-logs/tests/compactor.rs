// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Compactor execution-loop tests — strict-TDD RED for the cave-logs honest
//! uplift (2026-05-30). These exercise the *execution* side of Loki's
//! compactor (`pkg/compactor`): given a set of small chunks, merge the chunks
//! of each `(tenant, stream_fp)` group into one re-encoded chunk, de-duplicate
//! entries that arrived more than once (at-least-once ingest), and drop entries
//! that have aged past the retention cutoff.
//!
//! Upstream parity: grafana/loki `pkg/compactor` (table compaction + dedupe)
//! and the per-tenant retention sweep. cave-logs already has the *planning*
//! half (`multitenant::plan_compaction`) and retention *counting*
//! (`multitenant::dry_run_retention`); these tests pin the missing *execution*
//! half — `compactor::compact`.

use cave_logs::chunk::encode_chunk;
use cave_logs::compactor::compact;
use cave_logs::models::{Chunk, Codec, LogEntry};

/// Build a sealed chunk from raw entries (mirrors `store.rs` sealing).
fn mk_chunk(tenant: &str, fp: u64, entries: &[LogEntry]) -> Chunk {
    let codec = Codec::Snappy;
    let data = encode_chunk(entries, codec).expect("encode");
    let min_ts = entries.iter().map(|e| e.timestamp_ns).min().unwrap_or(0);
    let max_ts = entries.iter().map(|e| e.timestamp_ns).max().unwrap_or(0);
    let uncompressed_size = entries.iter().map(|e| e.estimated_size()).sum();
    Chunk {
        stream_fp: fp,
        tenant: tenant.to_string(),
        min_ts,
        max_ts,
        num_entries: entries.len() as u64,
        codec,
        data,
        uncompressed_size,
    }
}

/// Decode a chunk back to entries for assertions.
fn entries_of(chunk: &Chunk) -> Vec<LogEntry> {
    cave_logs::chunk::decode_chunk(chunk).expect("decode")
}

const NO_RETENTION: i64 = i64::MIN;

#[test]
fn merge_overlapping_chunks_same_stream() {
    // Two chunks for the same stream with interleaving timestamps.
    let c1 = mk_chunk(
        "t",
        7,
        &[LogEntry::new(100, "a"), LogEntry::new(300, "c")],
    );
    let c2 = mk_chunk(
        "t",
        7,
        &[LogEntry::new(200, "b"), LogEntry::new(400, "d")],
    );

    let (out, stats) = compact(&[c1, c2], NO_RETENTION, Codec::Snappy).expect("compact");

    // One stream in, one merged chunk out.
    assert_eq!(out.len(), 1, "two chunks of one stream merge into one");
    assert_eq!(stats.input_chunks, 2);
    assert_eq!(stats.output_chunks, 1);

    let merged = entries_of(&out[0]);
    let lines: Vec<&str> = merged.iter().map(|e| e.line.as_str()).collect();
    // Sorted by timestamp ascending across both source chunks.
    assert_eq!(lines, vec!["a", "b", "c", "d"]);
    // Chunk metadata reflects the merged range.
    assert_eq!(out[0].min_ts, 100);
    assert_eq!(out[0].max_ts, 400);
    assert_eq!(out[0].num_entries, 4);
    assert_eq!(stats.entries_in, 4);
    assert_eq!(stats.entries_out, 4);
}

#[test]
fn dedupe_identical_entries() {
    // The same (timestamp, line) arrives in two chunks (at-least-once ingest).
    let c1 = mk_chunk("t", 1, &[LogEntry::new(100, "dup"), LogEntry::new(200, "x")]);
    let c2 = mk_chunk("t", 1, &[LogEntry::new(100, "dup"), LogEntry::new(300, "y")]);

    let (out, stats) = compact(&[c1, c2], NO_RETENTION, Codec::Snappy).expect("compact");

    assert_eq!(out.len(), 1);
    let merged = entries_of(&out[0]);
    let lines: Vec<&str> = merged.iter().map(|e| e.line.as_str()).collect();
    // The duplicate "dup"@100 collapses to a single entry.
    assert_eq!(lines, vec!["dup", "x", "y"]);
    assert_eq!(stats.entries_in, 4);
    assert_eq!(stats.entries_deduped, 1, "one duplicate removed");
    assert_eq!(stats.entries_out, 3);
    assert_eq!(out[0].num_entries, 3);
}

#[test]
fn dedupe_preserves_distinct_lines_same_ts() {
    // Same timestamp, different lines — both must survive.
    let c = mk_chunk(
        "t",
        1,
        &[
            LogEntry::new(100, "first"),
            LogEntry::new(100, "second"),
            LogEntry::new(100, "first"), // exact dup of the first
        ],
    );

    let (out, stats) = compact(&[c], NO_RETENTION, Codec::Snappy).expect("compact");

    let merged = entries_of(&out[0]);
    let lines: Vec<&str> = merged.iter().map(|e| e.line.as_str()).collect();
    // Distinct lines at the same ts are kept; only the exact dup is removed.
    assert_eq!(lines, vec!["first", "second"]);
    assert_eq!(stats.entries_deduped, 1);
}

#[test]
fn retention_drops_expired_chunks() {
    // Cutoff at 1000ns: entries before it are expired.
    let c = mk_chunk(
        "t",
        1,
        &[
            LogEntry::new(500, "old"),
            LogEntry::new(900, "older-but-still-old"),
            LogEntry::new(1000, "boundary-kept"),
            LogEntry::new(1500, "fresh"),
        ],
    );

    let (out, stats) = compact(&[c], 1000, Codec::Snappy).expect("compact");

    assert_eq!(out.len(), 1);
    let merged = entries_of(&out[0]);
    let lines: Vec<&str> = merged.iter().map(|e| e.line.as_str()).collect();
    // Entries with ts < 1000 dropped; ts == cutoff retained.
    assert_eq!(lines, vec!["boundary-kept", "fresh"]);
    assert_eq!(stats.entries_expired, 2);
    assert_eq!(stats.entries_out, 2);
    assert_eq!(out[0].min_ts, 1000);
}

#[test]
fn retention_emptied_stream_produces_no_chunk() {
    // Every entry is older than the cutoff → the group yields no output chunk.
    let c = mk_chunk("t", 1, &[LogEntry::new(10, "a"), LogEntry::new(20, "b")]);

    let (out, stats) = compact(&[c], 1000, Codec::Snappy).expect("compact");

    assert!(out.is_empty(), "a fully-expired stream emits no chunk");
    assert_eq!(stats.entries_expired, 2);
    assert_eq!(stats.output_chunks, 0);
}

#[test]
fn cross_stream_isolation() {
    // Different fingerprints must never be merged into one chunk.
    let a = mk_chunk("t", 1, &[LogEntry::new(100, "a1"), LogEntry::new(200, "a2")]);
    let b = mk_chunk("t", 2, &[LogEntry::new(150, "b1")]);

    let (out, stats) = compact(&[a, b], NO_RETENTION, Codec::Snappy).expect("compact");

    assert_eq!(out.len(), 2, "two distinct streams stay separate");
    assert_eq!(stats.output_chunks, 2);
    let mut fps: Vec<u64> = out.iter().map(|c| c.stream_fp).collect();
    fps.sort_unstable();
    assert_eq!(fps, vec![1, 2]);
}

#[test]
fn tenant_isolation() {
    // Same fingerprint but different tenants must not merge.
    let a = mk_chunk("tenant-a", 9, &[LogEntry::new(100, "a")]);
    let b = mk_chunk("tenant-b", 9, &[LogEntry::new(100, "b")]);

    let (out, _) = compact(&[a, b], NO_RETENTION, Codec::Snappy).expect("compact");

    assert_eq!(out.len(), 2, "same fp across tenants stays isolated");
    let mut tenants: Vec<String> = out.iter().map(|c| c.tenant.clone()).collect();
    tenants.sort();
    assert_eq!(tenants, vec!["tenant-a".to_string(), "tenant-b".to_string()]);
}

#[test]
fn compaction_is_idempotent() {
    let c1 = mk_chunk("t", 1, &[LogEntry::new(100, "a"), LogEntry::new(300, "c")]);
    let c2 = mk_chunk("t", 1, &[LogEntry::new(200, "b"), LogEntry::new(100, "a")]);

    let (once, _) = compact(&[c1, c2], NO_RETENTION, Codec::Snappy).expect("first pass");
    let (twice, stats2) = compact(&once, NO_RETENTION, Codec::Snappy).expect("second pass");

    // Re-compacting already-compacted output changes nothing.
    assert_eq!(once.len(), twice.len());
    assert_eq!(entries_of(&once[0]), entries_of(&twice[0]));
    assert_eq!(stats2.entries_deduped, 0, "no new dups on a clean re-run");
    assert_eq!(stats2.entries_expired, 0);
}

#[test]
fn empty_input_yields_empty_output() {
    let (out, stats) = compact(&[], NO_RETENTION, Codec::Snappy).expect("compact");
    assert!(out.is_empty());
    assert_eq!(stats.input_chunks, 0);
    assert_eq!(stats.output_chunks, 0);
}
