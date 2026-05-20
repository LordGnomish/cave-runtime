// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Container log management — JSON line format + rotation.
//!
//! Each log line is a JSON object matching Docker/containerd's format:
//!   {"log":"message\n","stream":"stdout","time":"2024-01-01T00:00:00Z"}
//!
//! Rotation: when the active log exceeds max_size_bytes, it is renamed to
//! <name>.1.log, existing rotated files are shifted (.1→.2, etc.), and files
//! beyond max_files are deleted.

use crate::error::{CriError, CriResult};
use crate::models::ContainerLogEntry;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB
pub const DEFAULT_MAX_FILES: u32 = 5;

/// Append a log line in JSON format to the container log file.
/// Rotates if the file exceeds max_size_bytes.
pub fn write_log_entry(
    log_path: &Path,
    stream: &str,
    message: &str,
    timestamp: DateTime<Utc>,
    max_size_bytes: u64,
    max_files: u32,
) -> CriResult<()> {
    maybe_rotate(log_path, max_size_bytes, max_files)?;

    let entry = json!({
        "log": format!("{}\n", message),
        "stream": stream,
        "time": timestamp.to_rfc3339(),
    });

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| CriError::Io(e))?;

    writeln!(file, "{}", entry).map_err(|e| CriError::Io(e))?;

    Ok(())
}

/// Read log entries from a container log file, optionally tailing the last N.
pub fn read_log_entries(log_path: &Path, tail: Option<usize>) -> CriResult<Vec<ContainerLogEntry>> {
    let mut entries = Vec::new();

    if !log_path.exists() {
        return Ok(entries);
    }

    let file = std::fs::File::open(log_path).map_err(CriError::Io)?;
    let reader = std::io::BufReader::new(file);

    for line in reader.lines().flatten() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            let timestamp = v["time"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            let stream = v["stream"].as_str().unwrap_or("stdout").to_string();
            let message = v["log"]
                .as_str()
                .unwrap_or("")
                .trim_end_matches('\n')
                .to_string();
            entries.push(ContainerLogEntry {
                timestamp,
                stream,
                message,
            });
        } else {
            entries.push(ContainerLogEntry {
                timestamp: Utc::now(),
                stream: "stdout".into(),
                message: line,
            });
        }
    }

    if let Some(n) = tail {
        let len = entries.len();
        if n < len {
            entries = entries[len - n..].to_vec();
        }
    }

    Ok(entries)
}

/// Read the last N lines across all rotated log files (active + .1 + .2 …).
pub fn read_all_rotated(log_path: &Path, tail: Option<usize>) -> CriResult<Vec<ContainerLogEntry>> {
    let mut all = Vec::new();

    // Active log
    all.extend(read_log_entries(log_path, None)?);

    // Rotated logs (oldest first so timestamps stay monotonic)
    let stem = log_path.to_string_lossy().into_owned();
    // Check .5, .4, … .1 in reverse, then reverse so oldest is first
    let mut rotated = Vec::new();
    for i in 1..=10u32 {
        let rotated_path = PathBuf::from(format!("{}.{}", stem, i));
        if rotated_path.exists() {
            rotated.push(read_log_entries(&rotated_path, None)?);
        } else {
            break;
        }
    }
    rotated.reverse();
    let mut prepend = Vec::new();
    for chunk in rotated {
        prepend.extend(chunk);
    }
    prepend.extend(all);
    let mut result = prepend;
    if let Some(n) = tail {
        let len = result.len();
        if n < len {
            result = result[len - n..].to_vec();
        }
    }
    Ok(result)
}

/// Rotate logs if active file is too large.
fn maybe_rotate(log_path: &Path, max_size_bytes: u64, max_files: u32) -> CriResult<()> {
    let size = match std::fs::metadata(log_path) {
        Ok(m) => m.len(),
        Err(_) => return Ok(()), // File doesn't exist yet — no rotation needed
    };

    if size < max_size_bytes {
        return Ok(());
    }

    rotate(log_path, max_files)
}

/// Rotate: active → .1, .1 → .2, …, delete beyond max_files.
pub fn rotate(log_path: &Path, max_files: u32) -> CriResult<()> {
    let stem = log_path.to_string_lossy().into_owned();

    // Delete the oldest file if it exists
    let oldest = PathBuf::from(format!("{}.{}", stem, max_files));
    if oldest.exists() {
        std::fs::remove_file(&oldest).map_err(CriError::Io)?;
    }

    // Shift .N-1 → .N, …, .1 → .2
    for i in (1..max_files).rev() {
        let from = PathBuf::from(format!("{}.{}", stem, i));
        let to = PathBuf::from(format!("{}.{}", stem, i + 1));
        if from.exists() {
            std::fs::rename(&from, &to).map_err(CriError::Io)?;
        }
    }

    // Rename active → .1
    if log_path.exists() {
        let archived = PathBuf::from(format!("{}.1", stem));
        std::fs::rename(log_path, &archived).map_err(CriError::Io)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    // ── write + read roundtrip ─────────────────────────────────────────────────

    #[test]
    fn write_and_read_single_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stdout",
            "hello world",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let entries = read_log_entries(&path, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "hello world");
        assert_eq!(entries[0].stream, "stdout");
    }

    #[test]
    fn write_and_read_stderr_entry() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stderr",
            "error occurred",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let entries = read_log_entries(&path, None).unwrap();
        assert_eq!(entries[0].stream, "stderr");
    }

    #[test]
    fn write_multiple_entries_preserves_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        for i in 0..5 {
            write_log_entry(
                &path,
                "stdout",
                &format!("line {}", i),
                ts,
                DEFAULT_MAX_SIZE,
                DEFAULT_MAX_FILES,
            )
            .unwrap();
        }

        let entries = read_log_entries(&path, None).unwrap();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].message, "line 0");
        assert_eq!(entries[4].message, "line 4");
    }

    #[test]
    fn read_nonexistent_file_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.log");
        let entries = read_log_entries(&path, None).unwrap();
        assert!(entries.is_empty());
    }

    // ── JSON format compliance ─────────────────────────────────────────────────

    #[test]
    fn log_file_is_valid_json_per_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stdout",
            "test message",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        for line in content.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(v["log"].is_string());
            assert!(v["stream"].is_string());
            assert!(v["time"].is_string());
        }
    }

    #[test]
    fn log_entry_message_ends_with_newline_in_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stdout",
            "no-newline",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert!(v["log"].as_str().unwrap().ends_with('\n'));
    }

    #[test]
    fn read_strips_trailing_newline_from_message() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stdout",
            "clean",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let entries = read_log_entries(&path, None).unwrap();
        assert!(!entries[0].message.ends_with('\n'));
    }

    #[test]
    fn timestamp_is_preserved_in_rfc3339() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = chrono::DateTime::parse_from_rfc3339("2024-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        write_log_entry(
            &path,
            "stdout",
            "ts-test",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let entries = read_log_entries(&path, None).unwrap();
        assert_eq!(entries[0].timestamp.timestamp(), ts.timestamp());
    }

    // ── tail ──────────────────────────────────────────────────────────────────

    #[test]
    fn tail_limits_returned_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        for i in 0..10 {
            write_log_entry(
                &path,
                "stdout",
                &format!("line {}", i),
                ts,
                DEFAULT_MAX_SIZE,
                DEFAULT_MAX_FILES,
            )
            .unwrap();
        }

        let entries = read_log_entries(&path, Some(3)).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "line 7");
        assert_eq!(entries[2].message, "line 9");
    }

    #[test]
    fn tail_larger_than_count_returns_all() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        for i in 0..3 {
            write_log_entry(
                &path,
                "stdout",
                &format!("line {}", i),
                ts,
                DEFAULT_MAX_SIZE,
                DEFAULT_MAX_FILES,
            )
            .unwrap();
        }

        let entries = read_log_entries(&path, Some(100)).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn tail_zero_returns_nothing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stdout",
            "line",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        let entries = read_log_entries(&path, Some(0)).unwrap();
        assert!(entries.is_empty());
    }

    // ── rotation ──────────────────────────────────────────────────────────────

    #[test]
    fn rotate_creates_archived_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        std::fs::write(&path, "log data").unwrap();

        rotate(&path, 5).unwrap();

        assert!(!path.exists(), "active log should be gone after rotate");
        let archived = dir.path().join("container.log.1");
        assert!(archived.exists(), "archived log should exist");
    }

    #[test]
    fn rotate_shifts_existing_numbered_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        std::fs::write(&path, "active").unwrap();
        std::fs::write(dir.path().join("container.log.1"), "old-1").unwrap();

        rotate(&path, 5).unwrap();

        assert!(dir.path().join("container.log.1").exists());
        assert!(dir.path().join("container.log.2").exists());
        let content = std::fs::read_to_string(dir.path().join("container.log.2")).unwrap();
        assert_eq!(content, "old-1");
    }

    #[test]
    fn rotate_deletes_oldest_beyond_max_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        std::fs::write(&path, "active").unwrap();
        // Fill up to max
        for i in 1..=5 {
            std::fs::write(
                dir.path().join(format!("container.log.{}", i)),
                format!("old-{}", i),
            )
            .unwrap();
        }

        rotate(&path, 5).unwrap();

        // .5 should be deleted, others shifted
        assert!(!dir.path().join("container.log.6").exists());
    }

    #[test]
    fn rotate_on_nonexistent_file_ok() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.log");
        // Should not error
        rotate(&path, 5).unwrap();
    }

    #[test]
    fn auto_rotate_triggers_when_size_exceeded() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        // Write 5 bytes as "existing" content
        std::fs::write(&path, "xxxxx").unwrap();

        // max_size_bytes = 4 → rotation should trigger
        let ts = Utc::now();
        write_log_entry(&path, "stdout", "trigger rotation", ts, 4, 5).unwrap();

        // After rotation the old content should be in .1
        let archived = dir.path().join("container.log.1");
        assert!(archived.exists());
    }

    #[test]
    fn no_rotation_when_under_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        let ts = Utc::now();

        write_log_entry(
            &path,
            "stdout",
            "small",
            ts,
            DEFAULT_MAX_SIZE,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        // .1 should not exist
        let archived = dir.path().join("container.log.1");
        assert!(!archived.exists());
    }

    // ── non-JSON lines gracefully parsed ──────────────────────────────────────

    #[test]
    fn non_json_lines_are_treated_as_raw_stdout() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("container.log");
        std::fs::write(&path, "plain text line\n").unwrap();

        let entries = read_log_entries(&path, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "plain text line");
        assert_eq!(entries[0].stream, "stdout");
    }
}
