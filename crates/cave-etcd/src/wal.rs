// SPDX-License-Identifier: AGPL-3.0-or-later
//! Write-Ahead Log — durable append-only record stream.
//!
//! Mirrors etcd v3.6's `server/storage/wal/` package: every mutating
//! operation (Put / Delete / Txn / Compact / LeaseGrant / LeaseRevoke)
//! is recorded here BEFORE it is applied to the MVCC store, so a crash
//! between apply and snapshot can be recovered by replaying the WAL.
//!
//! On-disk record framing:
//! ```text
//!   ┌────────────┬────────────┬──────────────────────┐
//!   │ 4B LE len  │ 4B LE crc  │  N bytes payload     │
//!   └────────────┴────────────┴──────────────────────┘
//! ```
//! where `len = N` (payload size in bytes) and `crc` is the IEEE CRC-32
//! of the payload bytes. Payload is a JSON-encoded [`WalRecord`].
//!
//! Faithful to etcd in shape, simpler in scope:
//!
//! * etcd uses a protobuf-encoded `walpb.Record { type, crc, data }`;
//!   we use JSON so the cave-etcd file stays self-describing under a
//!   text editor / `jq` for forensics during the single-node MVP.
//! * etcd cuts a new file at 64 MiB and names them
//!   `<seq:016x>-<index:016x>.wal`; the MVP keeps a single `wal.log`
//!   file. Rotation is a follow-up tracked in
//!   `docs/parity/cave-etcd-port-2026-05-12.md` §Known gaps.
//! * etcd records `metadataType` / `entryType` / `stateType` /
//!   `crcType` / `snapshotType`; we map them to the [`WalRecord`]
//!   enum variants below.
//!
//! ## Crash recovery
//!
//! On `Wal::open`, the loader scans the file from offset 0. For each
//! record, it reads the 8-byte header, then the payload, then validates
//! the CRC. If the file ends MID-frame (incomplete header or
//! length-prefix promises more bytes than remain), the loader
//! TRUNCATES to the last valid record boundary and continues — this
//! matches etcd's behaviour when the OS killed `fsync` partway through.
//! If the CRC of an otherwise-complete record fails, the loader
//! returns [`WalError::Corrupt`] — those are NOT silently dropped,
//! because a mid-record bit flip indicates real disk damage.
//!
//! ## Durability
//!
//! Each [`Wal::append`] writes + flushes + fsyncs. Callers that need
//! batching can compose at a higher layer; the WAL itself does not
//! buffer past a single record so a power loss at any byte boundary
//! either keeps or loses the most recent append, but never a prior
//! one. This is the same guarantee etcd's `wal.Save` provides.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Header is `[4B LE length][4B LE crc32]`.
const RECORD_HEADER_LEN: usize = 8;

/// Hard cap on a single payload — defends against a corrupted length
/// header that would otherwise allocate megabytes of garbage. Mirrors
/// etcd's `maxRecordSize = 8 MiB` in `server/storage/wal/decoder.go`.
const MAX_PAYLOAD_LEN: u32 = 8 * 1024 * 1024;

/// File name relative to the WAL directory. Single-file MVP; rotation
/// is tracked as a follow-up.
const WAL_FILENAME: &str = "wal.log";

#[derive(Debug, thiserror::Error)]
pub enum WalError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("payload too large: {0} bytes > {} limit", MAX_PAYLOAD_LEN)]
    PayloadTooLarge(u32),
    #[error("record CRC mismatch at offset {offset}: expected {expected:#010x}, got {got:#010x}")]
    Corrupt {
        offset: u64,
        expected: u32,
        got: u32,
    },
    #[error("encode: {0}")]
    Encode(String),
    #[error("decode: {0}")]
    Decode(String),
}

/// One entry in the WAL. The variant names mirror etcd's `walpb.Record.Type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WalRecord {
    /// Cluster identity — written once at WAL creation, mirrors etcd's
    /// `metadataType` record.
    Metadata {
        cluster_id: u64,
        node_id: u64,
    },
    /// Per-mutation entry — bulk of the log. Index is monotonically
    /// increasing across the entire lifetime of the WAL.
    Entry(EntryRecord),
    /// Raft hard-state checkpoint. In the single-node MVP this is
    /// written opportunistically; multi-node Raft (Paket C) will drive
    /// the cadence.
    State {
        commit_index: u64,
        term: u64,
        voted_for: Option<u64>,
    },
    /// Marker pointing at a snapshot file. After a snapshot is durably
    /// on disk, all entries with `index <= snapshot_index` can be
    /// truncated. Mirrors etcd's `snapshotType`.
    Snapshot {
        snapshot_index: u64,
        snapshot_term: u64,
    },
}

/// One log entry. `index` is the per-WAL monotonic position; `term` is
/// the raft term (always 1 in the single-node MVP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryRecord {
    pub index: u64,
    pub term: u64,
    pub op: WalOp,
}

/// Mutating operations recorded by the store before applying. Subset
/// chosen to cover every public mutation surface in `routes.rs` that a
/// crash could otherwise lose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalOp {
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
        lease: Option<i64>,
    },
    Delete {
        key: Vec<u8>,
        range_end: Option<Vec<u8>>,
    },
    Txn {
        ops: Vec<WalOp>,
    },
    Compact {
        revision: i64,
    },
    LeaseGrant {
        lease_id: i64,
        ttl_seconds: i64,
    },
    LeaseRevoke {
        lease_id: i64,
    },
}

/// Durable WAL handle. Single-file MVP; one append per write.
#[derive(Debug)]
pub struct Wal {
    dir: PathBuf,
    file: File,
    /// Last index assigned to an [`EntryRecord`]. 0 means "no entries
    /// yet"; the next entry will be assigned 1.
    last_entry_index: u64,
    /// Number of records persisted (any variant). Used by tests and the
    /// `len()` API; not part of etcd's WAL surface.
    record_count: u64,
}

impl Wal {
    /// Open or create the WAL directory. Replays any existing records
    /// to discover `last_entry_index` so the next [`append_entry`] uses
    /// a monotonic index even after restart.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, WalError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(WAL_FILENAME);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        // Replay to discover state. We also truncate any trailing
        // partial record so subsequent appends land at a clean
        // boundary.
        let (last_entry_index, record_count, valid_end) =
            scan_for_valid_end(&path)?;
        if valid_end < file.metadata()?.len() {
            file.set_len(valid_end)?;
        }
        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            dir,
            file,
            last_entry_index,
            record_count,
        })
    }

    /// Append a non-entry record (Metadata / State / Snapshot). For
    /// log entries use [`append_entry`].
    pub fn append(&mut self, record: &WalRecord) -> Result<(), WalError> {
        write_record(&mut self.file, record)?;
        self.file.flush()?;
        self.file.sync_data()?;
        self.record_count += 1;
        Ok(())
    }

    /// Append a log entry. The provided `op` is assigned the next
    /// sequential index and persisted. Returns the assigned index.
    pub fn append_entry(&mut self, term: u64, op: WalOp) -> Result<u64, WalError> {
        let index = self.last_entry_index + 1;
        let record = WalRecord::Entry(EntryRecord { index, term, op });
        write_record(&mut self.file, &record)?;
        self.file.flush()?;
        self.file.sync_data()?;
        self.last_entry_index = index;
        self.record_count += 1;
        Ok(index)
    }

    /// Index that will be assigned to the next entry. Useful for
    /// callers that need to know in advance what to record alongside
    /// the WAL append (e.g. an in-memory raft log mirror).
    pub fn next_entry_index(&self) -> u64 {
        self.last_entry_index + 1
    }

    /// Greatest entry index currently persisted. 0 if no entries.
    pub fn last_entry_index(&self) -> u64 {
        self.last_entry_index
    }

    /// Total number of records (every variant) currently persisted.
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Stream every record from the start of the WAL in append order.
    /// Useful for crash recovery: callers replay the iterator into the
    /// MVCC store before serving traffic.
    pub fn replay(&self) -> Result<Vec<WalRecord>, WalError> {
        replay_file(&self.dir.join(WAL_FILENAME))
    }

    /// Discard every record up to and including the entry at `index`.
    /// Mirrors etcd's `wal.ReleaseLockTo` / segment file removal once a
    /// snapshot covers `index`. In the single-file MVP this rewrites
    /// the file with only the records strictly after `index` (plus any
    /// non-Entry records, which are always retained — Metadata and the
    /// latest State must survive truncation).
    pub fn truncate_through(&mut self, index: u64) -> Result<(), WalError> {
        let path = self.dir.join(WAL_FILENAME);
        let kept = replay_file(&path)?
            .into_iter()
            .filter(|r| match r {
                WalRecord::Entry(e) => e.index > index,
                _ => true,
            })
            .collect::<Vec<_>>();
        // Rewrite atomically: write to a sibling, fsync, rename.
        let tmp = self.dir.join(format!("{WAL_FILENAME}.tmp"));
        let mut out = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        for r in &kept {
            write_record(&mut out, r)?;
        }
        out.flush()?;
        out.sync_data()?;
        drop(out);
        std::fs::rename(&tmp, &path)?;

        // Reopen the file handle so subsequent appends see the
        // rewritten state, and recompute counters.
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        let (last_entry_index, record_count, _) = scan_for_valid_end(&path)?;
        self.file = file;
        self.file.seek(SeekFrom::End(0))?;
        self.last_entry_index = last_entry_index;
        self.record_count = record_count;
        Ok(())
    }

    /// Path of the underlying log file. Exposed for tests + diagnostics.
    pub fn path(&self) -> PathBuf {
        self.dir.join(WAL_FILENAME)
    }
}

fn write_record<W: Write>(out: &mut W, record: &WalRecord) -> Result<(), WalError> {
    let payload = serde_json::to_vec(record).map_err(|e| WalError::Encode(e.to_string()))?;
    if payload.len() > MAX_PAYLOAD_LEN as usize {
        return Err(WalError::PayloadTooLarge(payload.len() as u32));
    }
    let len = payload.len() as u32;
    let crc = crc32fast::hash(&payload);
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&crc.to_le_bytes())?;
    out.write_all(&payload)?;
    Ok(())
}

fn replay_file(path: &Path) -> Result<Vec<WalRecord>, WalError> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(WalError::Io(e)),
    };
    let mut r = BufReader::new(f);
    let mut out = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let mut header = [0u8; RECORD_HEADER_LEN];
        match r.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(WalError::Io(e)),
        }
        let len = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let crc_expected = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if len > MAX_PAYLOAD_LEN {
            // Length header has been corrupted (or is from a future
            // version with a larger cap). Treat as the end-of-valid
            // boundary so partial-write recovery still works.
            break;
        }
        let mut payload = vec![0u8; len as usize];
        match r.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(WalError::Io(e)),
        }
        let crc_got = crc32fast::hash(&payload);
        if crc_got != crc_expected {
            return Err(WalError::Corrupt {
                offset,
                expected: crc_expected,
                got: crc_got,
            });
        }
        let record: WalRecord = serde_json::from_slice(&payload)
            .map_err(|e| WalError::Decode(format!("at offset {offset}: {e}")))?;
        out.push(record);
        offset += (RECORD_HEADER_LEN as u64) + (len as u64);
    }
    Ok(out)
}

/// Scan the WAL file and return:
///   - the greatest `EntryRecord::index` seen,
///   - the total record count,
///   - the byte offset just past the last fully-valid record (so a
///     trailing partial frame can be truncated on open).
fn scan_for_valid_end(path: &Path) -> Result<(u64, u64, u64), WalError> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((0, 0, 0)),
        Err(e) => return Err(WalError::Io(e)),
    };
    let mut r = BufReader::new(f);
    let mut last_entry_index = 0u64;
    let mut record_count = 0u64;
    let mut last_good_end = 0u64;
    loop {
        let mut header = [0u8; RECORD_HEADER_LEN];
        match r.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(WalError::Io(e)),
        }
        let len = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let crc_expected = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if len > MAX_PAYLOAD_LEN {
            break;
        }
        let mut payload = vec![0u8; len as usize];
        match r.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(WalError::Io(e)),
        }
        let crc_got = crc32fast::hash(&payload);
        if crc_got != crc_expected {
            return Err(WalError::Corrupt {
                offset: last_good_end,
                expected: crc_expected,
                got: crc_got,
            });
        }
        if let Ok(WalRecord::Entry(e)) = serde_json::from_slice::<WalRecord>(&payload) {
            if e.index > last_entry_index {
                last_entry_index = e.index;
            }
        }
        record_count += 1;
        last_good_end += (RECORD_HEADER_LEN as u64) + (len as u64);
    }
    Ok((last_entry_index, record_count, last_good_end))
}

/// Replay a WAL into an in-memory [`KvStore`], reconstructing the
/// equivalent state of every recorded mutation. This is the boot path
/// for any caller that wants WAL-backed durability: open the WAL,
/// hand it to `replay_into_store`, then start serving traffic.
///
/// Mirrors etcd's `server/etcdserver/server.go::recoverFromWAL` shape
/// — non-Entry records (Metadata/State/Snapshot) are observed but do
/// not mutate the store directly; the caller can read them by calling
/// [`Wal::replay`] separately if it needs the cluster identity or the
/// committed-index checkpoint.
pub fn replay_into_store(wal: &Wal, store: &crate::store::KvStore) {
    let records = match wal.replay() {
        Ok(r) => r,
        Err(_) => return,
    };
    for record in records {
        if let WalRecord::Entry(e) = record {
            apply_op(store, &e.op);
        }
    }
}

fn apply_op(store: &crate::store::KvStore, op: &WalOp) {
    use crate::models::{
        CompactionRequest, DeleteRangeRequest, LeaseGrantRequest, PutRequest,
    };
    match op {
        WalOp::Put { key, value, lease } => {
            let req = PutRequest {
                key: String::from_utf8_lossy(key).into_owned(),
                value: String::from_utf8_lossy(value).into_owned(),
                lease: *lease,
                prev_kv: false,
            };
            let _ = store.put(&req);
        }
        WalOp::Delete { key, range_end } => {
            let req = DeleteRangeRequest {
                key: String::from_utf8_lossy(key).into_owned(),
                range_end: range_end
                    .as_ref()
                    .map(|r| String::from_utf8_lossy(r).into_owned()),
                prev_kv: false,
            };
            let _ = store.delete_range(&req);
        }
        WalOp::Txn { ops } => {
            for op in ops {
                apply_op(store, op);
            }
        }
        WalOp::Compact { revision } => {
            let req = CompactionRequest {
                revision: *revision as u64,
                physical: false,
            };
            let _ = store.compaction(&req);
        }
        WalOp::LeaseGrant {
            lease_id,
            ttl_seconds,
        } => {
            let req = LeaseGrantRequest {
                ttl: *ttl_seconds,
                id: Some(*lease_id),
            };
            let _ = store.lease_grant(&req);
        }
        WalOp::LeaseRevoke { lease_id } => {
            let _ = store.lease_revoke(*lease_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn put(key: &str, value: &str) -> WalOp {
        WalOp::Put {
            key: key.as_bytes().to_vec(),
            value: value.as_bytes().to_vec(),
            lease: None,
        }
    }

    #[test]
    fn open_creates_empty_wal_on_fresh_dir() {
        let dir = tempdir().unwrap();
        let w = Wal::open(dir.path()).unwrap();
        assert_eq!(w.last_entry_index(), 0);
        assert_eq!(w.record_count(), 0);
        assert_eq!(w.next_entry_index(), 1);
        assert!(w.replay().unwrap().is_empty());
    }

    #[test]
    fn append_entry_assigns_monotonic_index() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        assert_eq!(w.append_entry(1, put("a", "1")).unwrap(), 1);
        assert_eq!(w.append_entry(1, put("b", "2")).unwrap(), 2);
        assert_eq!(w.append_entry(1, put("c", "3")).unwrap(), 3);
        assert_eq!(w.last_entry_index(), 3);
        assert_eq!(w.next_entry_index(), 4);
    }

    #[test]
    fn replay_returns_records_in_append_order() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        w.append_entry(1, put("a", "1")).unwrap();
        w.append_entry(1, put("b", "2")).unwrap();
        let recs = w.replay().unwrap();
        assert_eq!(recs.len(), 2);
        match &recs[0] {
            WalRecord::Entry(e) => {
                assert_eq!(e.index, 1);
                match &e.op {
                    WalOp::Put { key, .. } => assert_eq!(key, b"a"),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn restart_recovers_last_entry_index() {
        let dir = tempdir().unwrap();
        {
            let mut w = Wal::open(dir.path()).unwrap();
            w.append_entry(1, put("a", "1")).unwrap();
            w.append_entry(1, put("b", "2")).unwrap();
            w.append_entry(1, put("c", "3")).unwrap();
        }
        let w2 = Wal::open(dir.path()).unwrap();
        assert_eq!(w2.last_entry_index(), 3);
        assert_eq!(w2.next_entry_index(), 4);
        assert_eq!(w2.replay().unwrap().len(), 3);
    }

    #[test]
    fn metadata_and_state_records_round_trip() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        w.append(&WalRecord::Metadata {
            cluster_id: 42,
            node_id: 7,
        })
        .unwrap();
        w.append(&WalRecord::State {
            commit_index: 100,
            term: 3,
            voted_for: Some(7),
        })
        .unwrap();
        w.append_entry(3, put("k", "v")).unwrap();
        let recs = w.replay().unwrap();
        assert_eq!(recs.len(), 3);
        assert!(matches!(&recs[0], WalRecord::Metadata { cluster_id: 42, .. }));
        assert!(matches!(&recs[1], WalRecord::State { commit_index: 100, .. }));
        assert!(matches!(&recs[2], WalRecord::Entry(_)));
    }

    #[test]
    fn corrupted_payload_returns_crc_error() {
        let dir = tempdir().unwrap();
        {
            let mut w = Wal::open(dir.path()).unwrap();
            w.append_entry(1, put("a", "1")).unwrap();
        }
        // Flip a byte in the payload (after the 8-byte header).
        let path = dir.path().join(WAL_FILENAME);
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();
        let err = Wal::open(dir.path()).unwrap_err();
        assert!(matches!(err, WalError::Corrupt { .. }));
    }

    #[test]
    fn trailing_partial_header_is_truncated_on_open() {
        let dir = tempdir().unwrap();
        {
            let mut w = Wal::open(dir.path()).unwrap();
            w.append_entry(1, put("a", "1")).unwrap();
            w.append_entry(1, put("b", "2")).unwrap();
        }
        // Simulate a crash mid-header: append 3 garbage bytes.
        let path = dir.path().join(WAL_FILENAME);
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[0u8, 0u8, 0u8]).unwrap();
        f.sync_all().unwrap();
        drop(f);

        let w = Wal::open(dir.path()).unwrap();
        // Both prior records still readable, last_entry_index still 2.
        assert_eq!(w.last_entry_index(), 2);
        assert_eq!(w.replay().unwrap().len(), 2);
        // And the trailing garbage was trimmed off, so the next append
        // lands at the right offset (no zero-byte hole).
        let len_after_open = std::fs::metadata(&path).unwrap().len();
        let mut w2 = w;
        w2.append_entry(1, put("c", "3")).unwrap();
        let len_after_append = std::fs::metadata(w2.path()).unwrap().len();
        assert!(len_after_append > len_after_open);
        let recs = w2.replay().unwrap();
        assert_eq!(recs.len(), 3);
    }

    #[test]
    fn trailing_partial_payload_is_truncated_on_open() {
        let dir = tempdir().unwrap();
        {
            let mut w = Wal::open(dir.path()).unwrap();
            w.append_entry(1, put("a", "1")).unwrap();
        }
        // Synthesize a header that claims 1000 bytes but follow with
        // only 10. The scanner should treat that as "partial write,
        // truncate here".
        let path = dir.path().join(WAL_FILENAME);
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&1000u32.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap();
        f.write_all(&[0u8; 10]).unwrap();
        f.sync_all().unwrap();
        drop(f);

        let w = Wal::open(dir.path()).unwrap();
        assert_eq!(w.last_entry_index(), 1);
        assert_eq!(w.replay().unwrap().len(), 1);
    }

    #[test]
    fn truncate_through_drops_entries_at_or_below_index() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        w.append(&WalRecord::Metadata {
            cluster_id: 1,
            node_id: 1,
        })
        .unwrap();
        for i in 1..=5 {
            w.append_entry(1, put(&format!("k{i}"), &format!("v{i}"))).unwrap();
        }
        w.truncate_through(3).unwrap();
        let recs = w.replay().unwrap();
        // Metadata kept (non-Entry records are retained) plus entries 4 and 5.
        assert_eq!(recs.len(), 3);
        assert!(matches!(&recs[0], WalRecord::Metadata { .. }));
        match &recs[1] {
            WalRecord::Entry(e) => assert_eq!(e.index, 4),
            _ => panic!(),
        }
        match &recs[2] {
            WalRecord::Entry(e) => assert_eq!(e.index, 5),
            _ => panic!(),
        }
        // last_entry_index re-derived after truncate.
        assert_eq!(w.last_entry_index(), 5);
    }

    #[test]
    fn append_after_truncate_continues_from_existing_max() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        for i in 1..=3 {
            w.append_entry(1, put(&format!("k{i}"), "v")).unwrap();
        }
        w.truncate_through(2).unwrap();
        // Entry 3 survived; the next append must continue from 4.
        assert_eq!(w.append_entry(1, put("k4", "v")).unwrap(), 4);
    }

    #[test]
    fn delete_and_txn_records_round_trip() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        w.append_entry(
            1,
            WalOp::Delete {
                key: b"foo".to_vec(),
                range_end: Some(b"foo\xff".to_vec()),
            },
        )
        .unwrap();
        w.append_entry(
            1,
            WalOp::Txn {
                ops: vec![put("a", "1"), put("b", "2")],
            },
        )
        .unwrap();
        let recs = w.replay().unwrap();
        assert_eq!(recs.len(), 2);
        match &recs[0] {
            WalRecord::Entry(e) => match &e.op {
                WalOp::Delete { key, range_end } => {
                    assert_eq!(key, b"foo");
                    assert_eq!(range_end.as_deref(), Some(b"foo\xff".as_slice()));
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
        match &recs[1] {
            WalRecord::Entry(e) => match &e.op {
                WalOp::Txn { ops } => assert_eq!(ops.len(), 2),
                _ => panic!(),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn lease_grant_and_revoke_round_trip() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        w.append_entry(
            1,
            WalOp::LeaseGrant {
                lease_id: 9001,
                ttl_seconds: 60,
            },
        )
        .unwrap();
        w.append_entry(1, WalOp::LeaseRevoke { lease_id: 9001 }).unwrap();
        w.append_entry(1, WalOp::Compact { revision: 42 }).unwrap();
        let recs = w.replay().unwrap();
        assert_eq!(recs.len(), 3);
        match &recs[0] {
            WalRecord::Entry(e) => {
                assert!(matches!(&e.op, WalOp::LeaseGrant { lease_id: 9001, ttl_seconds: 60 }));
            }
            _ => panic!(),
        }
        match &recs[2] {
            WalRecord::Entry(e) => {
                assert!(matches!(&e.op, WalOp::Compact { revision: 42 }));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn payload_size_limit_rejects_oversize_records() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        let huge = vec![0u8; (MAX_PAYLOAD_LEN as usize) + 1];
        let err = w
            .append_entry(
                1,
                WalOp::Put {
                    key: b"k".to_vec(),
                    value: huge,
                    lease: None,
                },
            )
            .unwrap_err();
        assert!(matches!(err, WalError::PayloadTooLarge(_)));
        // And nothing was persisted.
        assert_eq!(w.last_entry_index(), 0);
    }

    #[test]
    fn record_count_increases_per_append() {
        let dir = tempdir().unwrap();
        let mut w = Wal::open(dir.path()).unwrap();
        assert_eq!(w.record_count(), 0);
        w.append(&WalRecord::Metadata { cluster_id: 1, node_id: 1 }).unwrap();
        assert_eq!(w.record_count(), 1);
        w.append_entry(1, put("a", "1")).unwrap();
        assert_eq!(w.record_count(), 2);
        w.append_entry(1, put("b", "2")).unwrap();
        assert_eq!(w.record_count(), 3);
    }

    #[test]
    fn restart_after_truncate_recovers_correctly() {
        let dir = tempdir().unwrap();
        {
            let mut w = Wal::open(dir.path()).unwrap();
            for i in 1..=5 {
                w.append_entry(1, put(&format!("k{i}"), "v")).unwrap();
            }
            w.truncate_through(3).unwrap();
        }
        let w2 = Wal::open(dir.path()).unwrap();
        assert_eq!(w2.last_entry_index(), 5);
        let recs = w2.replay().unwrap();
        assert_eq!(recs.len(), 2);
        match &recs[0] {
            WalRecord::Entry(e) => assert_eq!(e.index, 4),
            _ => panic!(),
        }
    }

    #[test]
    fn empty_directory_does_not_panic_on_replay() {
        let dir = tempdir().unwrap();
        let w = Wal::open(dir.path()).unwrap();
        assert!(w.replay().unwrap().is_empty());
    }
}
