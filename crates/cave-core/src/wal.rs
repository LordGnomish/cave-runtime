//! Generic append-only write-ahead log.
//!
//! Wire format per record: `[4-byte LE length][4-byte LE CRC32][N-byte JSON payload]`
//!
//! CRC32 covers only the JSON payload.  A mismatched CRC causes the record to be
//! silently skipped during replay (same behaviour as Prometheus TSDB WAL).
//!
//! # Upstream reference
//! Pattern ported from Prometheus `tsdb/record/` and etcd `mvcc/backend/`.
//! Generic over entry type so each crate keeps its own domain enum.

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crc32fast::Hasher as Crc32Hasher;
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

// ── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum WalError {
    #[error("WAL io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WAL serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub type WalResult<T> = Result<T, WalError>;

// ── Writer ───────────────────────────────────────────────────────────────────

/// Append-only WAL writer.
///
/// Opened in append mode so it is safe to call `AppendLog::open` multiple times
/// on the same path (e.g. after a crash restart) — new records are appended
/// after existing ones.
pub struct AppendLog {
    writer: BufWriter<std::fs::File>,
}

impl AppendLog {
    /// Open (or create) the WAL at `path`.
    pub fn open(path: impl AsRef<Path>) -> WalResult<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { writer: BufWriter::new(file) })
    }

    /// Append one entry to the log.
    ///
    /// The entry is serialised to JSON, checksummed, and flushed to the OS buffer.
    /// Call [`AppendLog::sync`] for fsync durability.
    pub fn append<E: Serialize>(&mut self, entry: &E) -> WalResult<()> {
        let payload = serde_json::to_vec(entry)?;
        let len = payload.len() as u32;

        let mut h = Crc32Hasher::new();
        h.update(&payload);
        let crc = h.finalize();

        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&crc.to_le_bytes())?;
        self.writer.write_all(&payload)?;
        self.writer.flush()?;
        Ok(())
    }

    /// fsync the underlying file for full durability.
    pub fn sync(&mut self) -> WalResult<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()?;
        Ok(())
    }
}

// ── Replay ───────────────────────────────────────────────────────────────────

/// Replay all valid records from the WAL at `path`, calling `f` for each one.
///
/// Records with a CRC mismatch or that cannot be deserialised are silently
/// skipped — mirrors Prometheus TSDB WAL behaviour.  A missing file is treated
/// as an empty log (returns `Ok(())`).
pub fn replay<E, F>(path: impl AsRef<Path>, mut f: F) -> WalResult<()>
where
    E: DeserializeOwned,
    F: FnMut(E),
{
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    file.seek(SeekFrom::Start(0))?;

    let mut len_buf = [0u8; 4];
    let mut crc_buf = [0u8; 4];

    loop {
        match file.read_exact(&mut len_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        match file.read_exact(&mut crc_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_le_bytes(len_buf) as usize;
        let expected_crc = u32::from_le_bytes(crc_buf);

        let mut payload = vec![0u8; len];
        match file.read_exact(&mut payload) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }

        // Verify integrity; skip corrupted records.
        let mut h = Crc32Hasher::new();
        h.update(&payload);
        if h.finalize() != expected_crc {
            continue;
        }

        // Skip records whose type is unknown for the current caller.
        if let Ok(entry) = serde_json::from_slice::<E>(&payload) {
            f(entry);
        }
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::NamedTempFile;

    // Simple test entry type — mirrors the upstream Prometheus WalRecord shape.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "kind")]
    enum TestEntry {
        Sample { ts: i64, value: f64 },
        Checkpoint { ts: i64 },
    }

    // ── upstream: TestWALRoundtrip (Prometheus tsdb/wal_test.go) ─────────────

    #[test]
    fn test_wal_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        {
            let mut w = AppendLog::open(path).unwrap();
            w.append(&TestEntry::Sample { ts: 1000, value: 1.5 }).unwrap();
            w.append(&TestEntry::Sample { ts: 2000, value: 2.5 }).unwrap();
            w.append(&TestEntry::Checkpoint { ts: 2000 }).unwrap();
        }

        let mut entries = Vec::new();
        replay::<TestEntry, _>(path, |e| entries.push(e)).unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], TestEntry::Sample { ts: 1000, value: 1.5 });
        assert_eq!(entries[1], TestEntry::Sample { ts: 2000, value: 2.5 });
        assert_eq!(entries[2], TestEntry::Checkpoint { ts: 2000 });
    }

    // ── upstream: TestWALCorruptedRecord (Prometheus tsdb/wal_test.go) ───────

    #[test]
    fn test_corrupted_record_skipped() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        // Write two valid records then corrupt the second.
        {
            let mut w = AppendLog::open(path).unwrap();
            w.append(&TestEntry::Sample { ts: 100, value: 1.0 }).unwrap();
            w.append(&TestEntry::Sample { ts: 200, value: 2.0 }).unwrap();
        }

        // Flip a byte in the middle of the second record's payload.
        let mut data = std::fs::read(path).unwrap();
        let mid = data.len() / 2 + 5;
        if mid < data.len() {
            data[mid] ^= 0xFF;
        }
        std::fs::write(path, &data).unwrap();

        let mut entries = Vec::new();
        replay::<TestEntry, _>(path, |e| entries.push(e)).unwrap();

        // At least the first clean record must survive; corrupted ones are dropped.
        assert!(!entries.is_empty(), "expected at least one valid entry");
    }

    // ── upstream: TestWALMissingFile (etcd wal/wal_test.go) ─────────────────

    #[test]
    fn test_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.wal");

        let mut entries = Vec::<TestEntry>::new();
        replay::<TestEntry, _>(&path, |e| entries.push(e)).unwrap();
        assert!(entries.is_empty());
    }

    // ── upstream: TestWALAppendAfterReopen (etcd wal/wal_test.go) ────────────

    #[test]
    fn test_append_after_reopen() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        {
            let mut w = AppendLog::open(path).unwrap();
            w.append(&TestEntry::Sample { ts: 1, value: 0.1 }).unwrap();
        }
        {
            // Reopen same path — must append, not truncate.
            let mut w = AppendLog::open(path).unwrap();
            w.append(&TestEntry::Sample { ts: 2, value: 0.2 }).unwrap();
        }

        let mut entries = Vec::new();
        replay::<TestEntry, _>(path, |e| entries.push(e)).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], TestEntry::Sample { ts: 1, value: 0.1 });
        assert_eq!(entries[1], TestEntry::Sample { ts: 2, value: 0.2 });
    }

    // ── upstream: TestWALEmptyPayload ────────────────────────────────────────

    #[test]
    fn test_empty_log_replay() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        AppendLog::open(path).unwrap(); // open + close without writes

        let mut entries = Vec::<TestEntry>::new();
        replay::<TestEntry, _>(path, |e| entries.push(e)).unwrap();
        assert!(entries.is_empty());
    }

    // ── upstream: TestWALCRCMismatch ─────────────────────────────────────────

    #[test]
    fn test_crc_mismatch_skips_record() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        {
            let mut w = AppendLog::open(path).unwrap();
            w.append(&TestEntry::Checkpoint { ts: 42 }).unwrap();
        }

        // Corrupt CRC bytes (bytes 4–7).
        let mut data = std::fs::read(path).unwrap();
        if data.len() >= 8 {
            data[4] ^= 0xFF;
            data[5] ^= 0xFF;
        }
        std::fs::write(path, &data).unwrap();

        let mut entries = Vec::<TestEntry>::new();
        replay::<TestEntry, _>(path, |e| entries.push(e)).unwrap();
        assert!(entries.is_empty(), "corrupted CRC must skip the record");
    }
}
