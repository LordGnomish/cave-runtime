// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Compactor — executes the Loki compaction loop at chunk granularity.
//!
//! Loki's compactor (`pkg/compactor`) periodically merges the many small chunks
//! a stream accumulates into fewer, larger chunks; de-duplicates entries that
//! arrived more than once (Loki ingest is at-least-once); and drops entries
//! that have aged past the per-tenant / per-stream retention period.
//!
//! cave-logs already had the *planning* half — [`crate::multitenant::plan_compaction`]
//! decides which chunks to merge — and retention *counting* via
//! [`crate::multitenant::dry_run_retention`]. This module supplies the missing
//! *execution* half: decode → concatenate per `(tenant, stream_fp)` group →
//! retention-filter → sort → dedupe → re-encode into one chunk per group.
//!
//! It operates purely on [`Chunk`] values plus the chunk codec, so it is
//! single-process and side-effect free — the multi-node table-shipper sync that
//! Loki layers on top is out of scope (single-process cave-logs; see the
//! manifest scope-cuts).

use std::collections::BTreeMap;

use crate::chunk::{decode_chunk, encode_chunk};
use crate::models::{Chunk, Codec, LogEntry, TimestampNs};

/// Disable retention by passing this as the cutoff to [`compact`].
pub const NO_RETENTION: TimestampNs = TimestampNs::MIN;

/// Outcome counters for one compaction run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionStats {
    /// Number of chunks fed in.
    pub input_chunks: usize,
    /// Number of merged chunks produced.
    pub output_chunks: usize,
    /// Total entries decoded from the input chunks.
    pub entries_in: usize,
    /// Total entries written to the output chunks.
    pub entries_out: usize,
    /// Identical `(ts, line)` pairs removed.
    pub entries_deduped: usize,
    /// Entries dropped because they aged past the retention cutoff.
    pub entries_expired: usize,
}

/// Sort entries by `(ts, line)` and collapse exact `(ts, line)` duplicates.
/// Distinct lines that share a timestamp are preserved. Returns the number of
/// duplicates removed. Mirrors [`crate::store::LogStore::dedup_entries`].
fn sort_and_dedupe(entries: &mut Vec<LogEntry>) -> usize {
    let before = entries.len();
    entries.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.line.cmp(&b.line)));
    // De-dupe on the (ts, line) key — Loki's de-dupe identity.
    entries.dedup_by(|a, b| a.ts == b.ts && a.line == b.line);
    before - entries.len()
}

/// Compact a set of chunks.
///
/// Chunks are grouped by `(tenant, stream_fp)`; each group is merged into a
/// single re-encoded chunk. Entries with `ts < retention_cutoff_ns` are dropped
/// (pass [`NO_RETENTION`] to disable). The output chunk for each group is
/// encoded with `out_codec`. Output chunks are returned ordered by
/// `(tenant, stream_fp)`, so re-running [`compact`] on its own output is a
/// no-op (idempotent).
///
/// A group whose entries are all expired produces no output chunk.
pub fn compact(
    chunks: &[Chunk],
    retention_cutoff_ns: TimestampNs,
    out_codec: Codec,
) -> anyhow::Result<(Vec<Chunk>, CompactionStats)> {
    let mut stats = CompactionStats {
        input_chunks: chunks.len(),
        ..Default::default()
    };

    // Group decoded entries by (tenant, stream_fp). BTreeMap gives a
    // deterministic output ordering for idempotency.
    let mut groups: BTreeMap<(String, u64), Vec<LogEntry>> = BTreeMap::new();
    for chunk in chunks {
        let decoded = decode_chunk(chunk)?;
        groups
            .entry((chunk.tenant.clone(), chunk.stream_fp))
            .or_default()
            .extend(decoded);
    }

    let mut out = Vec::new();
    for ((tenant, stream_fp), mut entries) in groups {
        stats.entries_in += entries.len();

        // Retention: drop entries older than the cutoff.
        let before_retention = entries.len();
        entries.retain(|e| e.ts >= retention_cutoff_ns);
        stats.entries_expired += before_retention - entries.len();

        // Sort + dedupe the merged entry set.
        stats.entries_deduped += sort_and_dedupe(&mut entries);

        // A group emptied by retention emits no chunk.
        if entries.is_empty() {
            continue;
        }

        // Entries are sorted by (ts, line), so first/last give the ts range.
        let min_ts = entries.first().map(|e| e.ts).unwrap_or(0);
        let max_ts = entries.last().map(|e| e.ts).unwrap_or(0);
        let uncompressed_size: u64 = entries.iter().map(|e| e.size_bytes() as u64).sum();
        let num_entries = entries.len() as u64;
        let data = encode_chunk(&entries, out_codec)?;

        stats.entries_out += entries.len();
        out.push(Chunk {
            stream_fp,
            tenant,
            min_ts,
            max_ts,
            num_entries,
            codec: out_codec,
            data,
            uncompressed_size,
        });
    }

    stats.output_chunks = out.len();
    Ok((out, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(tenant: &str, fp: u64, e: &[LogEntry]) -> Chunk {
        let codec = Codec::Snappy;
        Chunk {
            stream_fp: fp,
            tenant: tenant.to_string(),
            min_ts: e.iter().map(|x| x.ts).min().unwrap_or(0),
            max_ts: e.iter().map(|x| x.ts).max().unwrap_or(0),
            num_entries: e.len() as u64,
            codec,
            data: encode_chunk(e, codec).unwrap(),
            uncompressed_size: e.iter().map(|x| x.size_bytes() as u64).sum(),
        }
    }

    #[test]
    fn unit_merge_and_order() {
        let c1 = mk("t", 1, &[LogEntry::new(300, "c"), LogEntry::new(100, "a")]);
        let c2 = mk("t", 1, &[LogEntry::new(200, "b")]);
        let (out, stats) = compact(&[c1, c2], NO_RETENTION, Codec::Snappy).unwrap();
        assert_eq!(out.len(), 1);
        let lines: Vec<String> = decode_chunk(&out[0])
            .unwrap()
            .into_iter()
            .map(|e| e.line)
            .collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
        assert_eq!(stats.entries_out, 3);
    }

    #[test]
    fn unit_dedup_count() {
        let c = mk(
            "t",
            1,
            &[
                LogEntry::new(1, "x"),
                LogEntry::new(1, "x"),
                LogEntry::new(1, "y"),
            ],
        );
        let (_out, stats) = compact(&[c], NO_RETENTION, Codec::Snappy).unwrap();
        assert_eq!(stats.entries_deduped, 1);
    }
}
