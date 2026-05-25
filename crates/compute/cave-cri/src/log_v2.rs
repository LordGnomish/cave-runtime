// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Container log v2 — kubelet-style tagged line format.
//!
//! The CRI specifies a single line format that the kubelet parses out of
//! container log files (kubernetes/cri-api `LogTag` and `ParseCRILog`):
//!
//! ```text
//! 2024-04-26T12:00:00.123456789Z stdout F hello world
//! ```
//!
//! Fields:
//! - RFC3339Nano timestamp (UTC).
//! - Stream tag (`stdout` or `stderr`).
//! - Log tag — `F` for a full line, `P` for a partial line that was split
//!   because it exceeded the runtime's per-line buffer (16 KiB in
//!   containerd) and is continued by the next entry.
//! - Message body — UTF-8, trailing newline stripped.
//!
//! This module:
//! - Encodes/decodes the tagged-line format.
//! - Reads filtered tails honouring `LogOptions { follow, since_time,
//!   until_time, tail_lines, limit_bytes }`.
//! - Composes with `crate::logs::rotate` so rotated logs are streamed in
//!   chronological order.

use crate::error::{CriError, CriResult};
use crate::logs;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Stream tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        }
    }

    pub fn parse(s: &str) -> Option<Stream> {
        match s {
            "stdout" => Some(Stream::Stdout),
            "stderr" => Some(Stream::Stderr),
            _ => None,
        }
    }
}

/// Log tag — full or partial line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogTag {
    /// `F` — full line.
    Full,
    /// `P` — partial line; the next entry continues this one.
    Partial,
}

impl LogTag {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogTag::Full => "F",
            LogTag::Partial => "P",
        }
    }

    pub fn parse(s: &str) -> Option<LogTag> {
        match s {
            "F" => Some(LogTag::Full),
            "P" => Some(LogTag::Partial),
            _ => None,
        }
    }
}

/// One parsed line in CRI tagged format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriLogEntry {
    pub timestamp: DateTime<Utc>,
    pub stream: Stream,
    pub tag: LogTag,
    pub message: String,
}

/// Filter options for reading container logs.
///
/// Mirrors `runtime.v1.ContainerLogOptions`:
/// - `tail_lines`  — return at most N entries from the tail.
/// - `since_time`  — drop entries older than this.
/// - `until_time`  — drop entries newer than this (cave extension; not in CRI).
/// - `limit_bytes` — return at most N bytes total (truncate from the head).
/// - `follow`      — block waiting for new entries (handled by the caller).
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    pub tail_lines: Option<usize>,
    pub since_time: Option<DateTime<Utc>>,
    pub until_time: Option<DateTime<Utc>>,
    pub limit_bytes: Option<usize>,
    pub follow: bool,
}

/// Maximum bytes per encoded log line before the runtime splits with `P` tag.
/// Matches containerd's default (16 KiB).
pub const MAX_LINE_BYTES: usize = 16 * 1024;

/// Encode a single line to the tagged-line wire format.
pub fn encode_line(timestamp: DateTime<Utc>, stream: Stream, tag: LogTag, message: &str) -> String {
    format!(
        "{} {} {} {}",
        timestamp.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        stream.as_str(),
        tag.as_str(),
        message
    )
}

/// Parse a single tagged line. Returns `Err` if the prefix is malformed.
pub fn parse_line(line: &str) -> CriResult<CriLogEntry> {
    // Format: <ts> <stream> <tag> <message...>
    let mut parts = line.splitn(4, ' ');
    let ts = parts
        .next()
        .ok_or_else(|| CriError::Runtime("missing timestamp in CRI log line".into()))?;
    let stream_str = parts
        .next()
        .ok_or_else(|| CriError::Runtime("missing stream in CRI log line".into()))?;
    let tag_str = parts
        .next()
        .ok_or_else(|| CriError::Runtime("missing tag in CRI log line".into()))?;
    let message = parts.next().unwrap_or("").to_string();

    let timestamp = DateTime::parse_from_rfc3339(ts)
        .map_err(|e| CriError::Runtime(format!("invalid CRI log timestamp {:?}: {}", ts, e)))?
        .with_timezone(&Utc);
    let stream = Stream::parse(stream_str)
        .ok_or_else(|| CriError::Runtime(format!("invalid CRI log stream {:?}", stream_str)))?;
    let tag = LogTag::parse(tag_str)
        .ok_or_else(|| CriError::Runtime(format!("invalid CRI log tag {:?}", tag_str)))?;

    Ok(CriLogEntry {
        timestamp,
        stream,
        tag,
        message,
    })
}

/// Append a CRI-formatted log line to `path`, splitting into multiple
/// tagged entries if the message exceeds `MAX_LINE_BYTES`.
///
/// All but the final entry get the `P` (partial) tag; the final one is
/// `F` (full). Rotation is applied via `crate::logs::rotate` if the
/// file is over `max_size_bytes`.
pub fn write_log_line(
    path: &Path,
    stream: Stream,
    message: &str,
    timestamp: DateTime<Utc>,
    max_size_bytes: u64,
    max_files: u32,
) -> CriResult<()> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() >= max_size_bytes {
            logs::rotate(path, max_files)?;
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(CriError::Io)?;

    let bytes = message.as_bytes();
    if bytes.len() <= MAX_LINE_BYTES {
        let line = encode_line(timestamp, stream, LogTag::Full, message);
        writeln!(file, "{}", line).map_err(CriError::Io)?;
        return Ok(());
    }

    // Split into MAX_LINE_BYTES chunks at UTF-8 boundaries.
    let mut start = 0;
    while start < bytes.len() {
        let mut end = (start + MAX_LINE_BYTES).min(bytes.len());
        // Walk back to a UTF-8 boundary.
        while end < bytes.len() && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
        let chunk = std::str::from_utf8(&bytes[start..end])
            .map_err(|e| CriError::Runtime(format!("invalid utf8 in log message: {}", e)))?;
        let tag = if end == bytes.len() {
            LogTag::Full
        } else {
            LogTag::Partial
        };
        let line = encode_line(timestamp, stream, tag, chunk);
        writeln!(file, "{}", line).map_err(CriError::Io)?;
        start = end;
    }
    Ok(())
}

/// Read the tagged log lines from a single file with no filtering.
pub fn read_file(path: &Path) -> CriResult<Vec<CriLogEntry>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let file = std::fs::File::open(path).map_err(CriError::Io)?;
    let reader = std::io::BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        // Best-effort: skip lines that do not parse cleanly.
        if let Ok(entry) = parse_line(&line) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Read all rotated and active logs for `path` in chronological order.
fn read_rotated_chain(path: &Path) -> CriResult<Vec<CriLogEntry>> {
    let stem = path.to_string_lossy().into_owned();
    let mut all = Vec::new();
    // Walk .N from highest down to .1 (oldest first), then the active file.
    // We probe up to .10; rotate's max_files is configurable but always small.
    let mut rotated_paths: Vec<PathBuf> = Vec::new();
    for i in 1..=10u32 {
        let p = PathBuf::from(format!("{}.{}", stem, i));
        if p.exists() {
            rotated_paths.push(p);
        } else {
            break;
        }
    }
    for p in rotated_paths.iter().rev() {
        all.extend(read_file(p)?);
    }
    all.extend(read_file(path)?);
    Ok(all)
}

/// Apply `LogOptions` filters to an in-memory entry list.
fn apply_filters(mut entries: Vec<CriLogEntry>, opts: &LogOptions) -> Vec<CriLogEntry> {
    if let Some(since) = opts.since_time {
        entries.retain(|e| e.timestamp >= since);
    }
    if let Some(until) = opts.until_time {
        entries.retain(|e| e.timestamp <= until);
    }
    if let Some(n) = opts.tail_lines {
        let len = entries.len();
        if n < len {
            entries = entries.split_off(len - n);
        }
    }
    if let Some(limit) = opts.limit_bytes {
        // Truncate from the head: keep the *latest* bytes.
        let mut total: usize = entries.iter().map(|e| e.message.len()).sum();
        while total > limit && !entries.is_empty() {
            total -= entries.first().map(|e| e.message.len()).unwrap_or(0);
            entries.remove(0);
        }
    }
    entries
}

/// Read tagged logs honouring `LogOptions`. `follow` is *not* implemented
/// here — the route layer is responsible for streaming in that mode.
pub fn read_logs(path: &Path, opts: &LogOptions) -> CriResult<Vec<CriLogEntry>> {
    let entries = read_rotated_chain(path)?;
    Ok(apply_filters(entries, opts))
}

/// Stitch consecutive `Partial` entries back into single logical lines for
/// presentation (kubelet does this when serving `kubectl logs`).
pub fn stitch_partials(entries: Vec<CriLogEntry>) -> Vec<CriLogEntry> {
    let mut out: Vec<CriLogEntry> = Vec::with_capacity(entries.len());
    let mut buf: Option<CriLogEntry> = None;
    for e in entries {
        match buf.take() {
            None => {
                if e.tag == LogTag::Full {
                    out.push(e);
                } else {
                    buf = Some(e);
                }
            }
            Some(mut prev) => {
                if prev.stream == e.stream {
                    prev.message.push_str(&e.message);
                    prev.timestamp = e.timestamp;
                    prev.tag = e.tag;
                    if e.tag == LogTag::Full {
                        out.push(prev);
                    } else {
                        buf = Some(prev);
                    }
                } else {
                    // Stream switched mid-partial — emit prev as-is.
                    out.push(prev);
                    if e.tag == LogTag::Full {
                        out.push(e);
                    } else {
                        buf = Some(e);
                    }
                }
            }
        }
    }
    if let Some(b) = buf {
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    fn ts(unix: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(unix, 0).unwrap()
    }

    // ── Stream and LogTag ─────────────────────────────────────────────────────

    #[test]
    fn stream_parse_known() {
        assert_eq!(Stream::parse("stdout"), Some(Stream::Stdout));
        assert_eq!(Stream::parse("stderr"), Some(Stream::Stderr));
        assert_eq!(Stream::parse("other"), None);
    }

    #[test]
    fn log_tag_parse_known() {
        assert_eq!(LogTag::parse("F"), Some(LogTag::Full));
        assert_eq!(LogTag::parse("P"), Some(LogTag::Partial));
        assert_eq!(LogTag::parse("X"), None);
    }

    // ── encode/parse roundtrip ────────────────────────────────────────────────

    #[test]
    fn encode_then_parse_roundtrip() {
        let when = Utc.with_ymd_and_hms(2024, 4, 26, 12, 0, 0).unwrap();
        let line = encode_line(when, Stream::Stdout, LogTag::Full, "hello world");
        let parsed = parse_line(&line).unwrap();
        assert_eq!(parsed.timestamp, when);
        assert_eq!(parsed.stream, Stream::Stdout);
        assert_eq!(parsed.tag, LogTag::Full);
        assert_eq!(parsed.message, "hello world");
    }

    #[test]
    fn encode_uses_rfc3339nano() {
        let when = Utc.timestamp_nanos(1_700_000_000_123_456_789);
        let line = encode_line(when, Stream::Stderr, LogTag::Partial, "x");
        // Should contain nanosecond precision and the Z suffix.
        assert!(
            line.contains(".123456789Z"),
            "missing nano precision: {}",
            line
        );
        assert!(line.contains(" stderr P "));
    }

    #[test]
    fn parse_line_with_spaces_in_message() {
        let line = "2024-04-26T12:00:00.000000000Z stdout F hello there many spaces";
        let p = parse_line(line).unwrap();
        assert_eq!(p.message, "hello there many spaces");
    }

    #[test]
    fn parse_line_missing_message_yields_empty_string() {
        let line = "2024-04-26T12:00:00.000000000Z stdout F";
        let p = parse_line(line).unwrap();
        assert_eq!(p.message, "");
    }

    #[test]
    fn parse_line_invalid_timestamp_errors() {
        assert!(parse_line("notadate stdout F hi").is_err());
    }

    #[test]
    fn parse_line_invalid_stream_errors() {
        assert!(parse_line("2024-04-26T12:00:00.000000000Z weird F hi").is_err());
    }

    #[test]
    fn parse_line_invalid_tag_errors() {
        assert!(parse_line("2024-04-26T12:00:00.000000000Z stdout X hi").is_err());
    }

    // ── write_log_line / read_file ────────────────────────────────────────────

    #[test]
    fn write_then_read_single_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        write_log_line(
            &path,
            Stream::Stdout,
            "first",
            ts(1_700_000_000),
            u64::MAX,
            5,
        )
        .unwrap();
        let entries = read_file(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "first");
        assert_eq!(entries[0].tag, LogTag::Full);
    }

    #[test]
    fn write_long_message_splits_with_partial_tag() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        let big = "x".repeat(MAX_LINE_BYTES * 2 + 7);
        write_log_line(&path, Stream::Stdout, &big, ts(1), u64::MAX, 5).unwrap();
        let entries = read_file(&path).unwrap();
        assert!(entries.len() >= 3);
        // All but the last must be Partial; last is Full.
        for e in &entries[..entries.len() - 1] {
            assert_eq!(e.tag, LogTag::Partial);
        }
        assert_eq!(entries.last().unwrap().tag, LogTag::Full);
        // Recombined message preserves length.
        let total: usize = entries.iter().map(|e| e.message.len()).sum();
        assert_eq!(total, big.len());
    }

    #[test]
    fn write_to_existing_file_appends() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        write_log_line(&path, Stream::Stdout, "a", ts(1), u64::MAX, 5).unwrap();
        write_log_line(&path, Stream::Stderr, "b", ts(2), u64::MAX, 5).unwrap();
        let entries = read_file(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "a");
        assert_eq!(entries[1].stream, Stream::Stderr);
    }

    #[test]
    fn read_file_nonexistent_returns_empty() {
        let dir = tempdir().unwrap();
        let entries = read_file(&dir.path().join("nope.log")).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn write_triggers_rotation_when_over_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        // Pre-create a file at 100 bytes.
        std::fs::write(&path, vec![b'x'; 100]).unwrap();
        write_log_line(&path, Stream::Stdout, "after", ts(1), 50, 3).unwrap();
        // Active file should be small (just the new line); .1 should exist.
        assert!(dir.path().join("c.log.1").exists());
    }

    // ── filters ───────────────────────────────────────────────────────────────

    fn seed_log(path: &Path, count: usize) {
        for i in 0..count {
            write_log_line(
                path,
                Stream::Stdout,
                &format!("line-{}", i),
                ts(1_700_000_000 + i as i64),
                u64::MAX,
                5,
            )
            .unwrap();
        }
    }

    #[test]
    fn tail_lines_filter_keeps_last_n() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        seed_log(&path, 10);
        let opts = LogOptions {
            tail_lines: Some(3),
            ..Default::default()
        };
        let entries = read_logs(&path, &opts).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "line-7");
        assert_eq!(entries[2].message, "line-9");
    }

    #[test]
    fn since_time_filter_drops_older_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        seed_log(&path, 5);
        let opts = LogOptions {
            since_time: Some(ts(1_700_000_002)),
            ..Default::default()
        };
        let entries = read_logs(&path, &opts).unwrap();
        // Drops line-0 and line-1.
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "line-2");
    }

    #[test]
    fn until_time_filter_drops_newer_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        seed_log(&path, 5);
        let opts = LogOptions {
            until_time: Some(ts(1_700_000_002)),
            ..Default::default()
        };
        let entries = read_logs(&path, &opts).unwrap();
        // Keeps line-0..line-2 inclusive.
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.last().unwrap().message, "line-2");
    }

    #[test]
    fn limit_bytes_filter_drops_oldest_until_under_budget() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        // Each "line-N" message is 6–7 bytes.
        seed_log(&path, 10);
        let opts = LogOptions {
            limit_bytes: Some(20),
            ..Default::default()
        };
        let entries = read_logs(&path, &opts).unwrap();
        let total: usize = entries.iter().map(|e| e.message.len()).sum();
        assert!(total <= 20);
        // We should have kept the most recent ones.
        assert!(entries.last().unwrap().message.ends_with('9'));
    }

    #[test]
    fn combined_since_and_tail_intersect() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        seed_log(&path, 10);
        let opts = LogOptions {
            since_time: Some(ts(1_700_000_005)),
            tail_lines: Some(2),
            ..Default::default()
        };
        let entries = read_logs(&path, &opts).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "line-8");
        assert_eq!(entries[1].message, "line-9");
    }

    #[test]
    fn options_defaults_return_all() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        seed_log(&path, 3);
        let entries = read_logs(&path, &LogOptions::default()).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn read_logs_includes_rotated_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        // Write to .1 (rotated) and active.
        write_log_line(
            &PathBuf::from(format!("{}.1", path.display())),
            Stream::Stdout,
            "old",
            ts(1),
            u64::MAX,
            5,
        )
        .unwrap();
        write_log_line(&path, Stream::Stdout, "new", ts(2), u64::MAX, 5).unwrap();
        let entries = read_logs(&path, &LogOptions::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "old"); // chronological
        assert_eq!(entries[1].message, "new");
    }

    // ── stitch_partials ───────────────────────────────────────────────────────

    #[test]
    fn stitch_combines_partial_then_full() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        let big = "y".repeat(MAX_LINE_BYTES + 5);
        write_log_line(&path, Stream::Stdout, &big, ts(1), u64::MAX, 5).unwrap();
        let raw = read_file(&path).unwrap();
        assert!(raw.len() >= 2);
        let stitched = stitch_partials(raw);
        assert_eq!(stitched.len(), 1);
        assert_eq!(stitched[0].message.len(), big.len());
        assert_eq!(stitched[0].tag, LogTag::Full);
    }

    #[test]
    fn stitch_emits_partial_when_stream_switches() {
        let entries = vec![
            CriLogEntry {
                timestamp: ts(1),
                stream: Stream::Stdout,
                tag: LogTag::Partial,
                message: "abc".into(),
            },
            CriLogEntry {
                timestamp: ts(2),
                stream: Stream::Stderr,
                tag: LogTag::Full,
                message: "ERR".into(),
            },
        ];
        let out = stitch_partials(entries);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].stream, Stream::Stdout);
        assert_eq!(out[0].message, "abc");
        assert_eq!(out[1].stream, Stream::Stderr);
        assert_eq!(out[1].message, "ERR");
    }

    #[test]
    fn stitch_passthrough_for_full_lines() {
        let entries = vec![
            CriLogEntry {
                timestamp: ts(1),
                stream: Stream::Stdout,
                tag: LogTag::Full,
                message: "a".into(),
            },
            CriLogEntry {
                timestamp: ts(2),
                stream: Stream::Stdout,
                tag: LogTag::Full,
                message: "b".into(),
            },
        ];
        let out = stitch_partials(entries.clone());
        assert_eq!(out.len(), 2);
        assert_eq!(out, entries);
    }

    // ── unicode boundary safety ───────────────────────────────────────────────

    #[test]
    fn write_long_unicode_message_stays_valid_utf8() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.log");
        // 4-byte UTF-8 char repeated until we cross the split threshold.
        let s = "🌊".repeat(MAX_LINE_BYTES); // each is 4 bytes → 64 KiB
        write_log_line(&path, Stream::Stdout, &s, ts(1), u64::MAX, 5).unwrap();
        let raw = read_file(&path).unwrap();
        let stitched = stitch_partials(raw);
        assert_eq!(stitched.len(), 1);
        assert_eq!(stitched[0].message, s);
    }
}
