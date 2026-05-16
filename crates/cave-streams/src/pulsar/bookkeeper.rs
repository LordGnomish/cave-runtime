// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/bookkeeper@1e88110d78c4a26f9c5749c78c90bab3e4e0a0ae (release-4.17.1)
//         bookkeeper-server/src/main/java/org/apache/bookkeeper/bookie/Bookie.java
//         bookkeeper-server/src/main/java/org/apache/bookkeeper/client/LedgerHandle.java
//         bookkeeper-server/src/main/java/org/apache/bookkeeper/client/LedgerMetadata.java
//         apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008 (v4.2.0)
//         pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/PersistentTopic.java
//
//! BookKeeper-style segmented persistent storage.
//!
//! A *ledger* is an ordered sequence of write-once *segments*; each segment
//! lives in its own backing file. An *entry* is `(ledger_id, entry_id,
//! payload)` with monotonically increasing `entry_id` *within* the ledger.
//! Writes go to `Wq` bookies and return success once `Aq` of them ack the
//! write; if fewer than `Aq` bookies are healthy the write fails fast with
//! [`BookKeeperError::NotEnoughBookies`].
//!
//! Cave Streams runs the bookies in-process (a `Vec<Bookie>` with simulated
//! quorum), and durability is provided by a single fsync per ledger on the
//! backing files. Ledger metadata is persisted through the
//! [`LedgerMetadataStore`] trait: an in-memory implementation is provided
//! for tests, and the production wire-up (under `/streams/pulsar/ledgers/{id}`
//! in `cave_etcd`) is left for the integration layer.
//!
//! Fencing is permanent — once [`SegmentedLedger::fence`] is called no
//! further writes succeed and readers see exactly the committed prefix.
//! Cursors advance by `read_entries(from, max)` across segment boundaries.

use crate::error::{StreamsError, StreamsResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Identifier types — `u64` matches BookKeeper's `LedgerId` / `EntryId`.
pub type LedgerId = u64;
/// Per-ledger monotonically increasing entry sequence number.
pub type EntryId = u64;

/// Errors that can occur during ledger operations.
#[derive(Debug, thiserror::Error)]
pub enum BookKeeperError {
    /// Fewer than `ack_quorum` bookies are reachable; the write quorum
    /// cannot be satisfied.  Mirrors `BKNotEnoughBookiesException`.
    #[error("not enough bookies: required Aq={required}, available={available}")]
    NotEnoughBookies { required: u32, available: u32 },

    /// The ledger has been fenced.  Mirrors `BKLedgerFencedException`.
    #[error("ledger {0} is fenced")]
    Fenced(LedgerId),

    /// Entry not present in the ledger (either before LAC or never written).
    #[error("entry {entry_id} not found in ledger {ledger_id}")]
    EntryNotFound { ledger_id: LedgerId, entry_id: EntryId },

    /// Ledger metadata not present in the metadata store.
    #[error("ledger {0} metadata missing")]
    LedgerNotFound(LedgerId),

    /// Underlying I/O error from the backing file.
    #[error("io error: {0}")]
    Io(String),
}

impl From<BookKeeperError> for StreamsError {
    fn from(e: BookKeeperError) -> Self {
        StreamsError::Internal(e.to_string())
    }
}

impl From<std::io::Error> for BookKeeperError {
    fn from(e: std::io::Error) -> Self {
        BookKeeperError::Io(e.to_string())
    }
}

pub type BkResult<T> = Result<T, BookKeeperError>;

// ─── Bookie ───────────────────────────────────────────────────────────────

/// An in-process bookie: a single node in the simulated ensemble.
///
/// Each bookie holds an in-memory `entry_id → payload` map per ledger;
/// durability is provided by the [`SegmentedLedger`]'s own backing file.
/// This split mirrors BookKeeper's separation of the network-side `Bookie`
/// from the on-disk `EntryLog`.
#[derive(Debug, Clone)]
pub struct Bookie {
    pub id: String,
    /// Whether the bookie is reachable for write quorum.
    pub healthy: bool,
    store: Arc<Mutex<HashMap<(LedgerId, EntryId), Vec<u8>>>>,
}

impl Bookie {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            healthy: true,
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Mark the bookie as unreachable.  Subsequent writes that target it
    /// count as `Wq - 1` healthy bookies for quorum purposes.
    pub fn set_healthy(&mut self, healthy: bool) {
        self.healthy = healthy;
    }

    fn add_entry(&self, ledger_id: LedgerId, entry_id: EntryId, payload: &[u8]) -> bool {
        if !self.healthy {
            return false;
        }
        let mut store = self.store.lock().unwrap();
        store.insert((ledger_id, entry_id), payload.to_vec());
        true
    }

    #[allow(dead_code)]
    fn read_entry(&self, ledger_id: LedgerId, entry_id: EntryId) -> Option<Vec<u8>> {
        let store = self.store.lock().unwrap();
        store.get(&(ledger_id, entry_id)).cloned()
    }
}

// ─── Segment ──────────────────────────────────────────────────────────────

/// One write-once segment of a ledger, backed by a single file.
///
/// Segments roll over at [`SegmentedLedger::roll_segment_at`] entries (the
/// default segment size).  Each segment file uses a length-prefixed entry
/// layout: `u64 entry_id || u32 len || bytes payload`.
#[derive(Debug)]
struct Segment {
    /// First `entry_id` in this segment (inclusive).
    first_entry: EntryId,
    /// Last `entry_id` in this segment (inclusive); `None` while the
    /// segment is open for writing.
    last_entry: Option<EntryId>,
    /// Backing file path.
    path: PathBuf,
    /// Open file handle for writes; `None` once the segment is sealed.
    file: Option<Mutex<File>>,
    /// In-memory index `entry_id → byte_offset` for random reads.
    index: HashMap<EntryId, u64>,
}

impl Segment {
    fn create(dir: &Path, first_entry: EntryId) -> BkResult<Self> {
        let path = dir.join(format!("segment-{:020}.log", first_entry));
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .append(false)
            .truncate(true)
            .open(&path)?;
        Ok(Self {
            first_entry,
            last_entry: None,
            path,
            file: Some(Mutex::new(file)),
            index: HashMap::new(),
        })
    }

    /// Open an existing segment file and replay its entries into the
    /// in-memory index.  Used during ledger recovery.
    fn open_existing(path: PathBuf, first_entry: EntryId) -> BkResult<Self> {
        let mut file = OpenOptions::new()
            .create(false)
            .read(true)
            .write(true)
            .append(false)
            .open(&path)?;
        let mut index = HashMap::new();
        let mut last_entry: Option<EntryId> = None;
        let mut pos: u64 = 0;
        file.seek(SeekFrom::Start(0))?;
        loop {
            let mut hdr = [0u8; 12];
            match file.read_exact(&mut hdr) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let entry_id = u64::from_be_bytes(hdr[0..8].try_into().unwrap());
            let len = u32::from_be_bytes(hdr[8..12].try_into().unwrap()) as usize;
            let mut payload = vec![0u8; len];
            file.read_exact(&mut payload)?;
            index.insert(entry_id, pos);
            last_entry = Some(entry_id);
            pos += 12 + len as u64;
        }
        Ok(Self {
            first_entry,
            last_entry,
            path,
            file: Some(Mutex::new(file)),
            index,
        })
    }

    fn append(&mut self, entry_id: EntryId, payload: &[u8]) -> BkResult<()> {
        let file_mtx = self
            .file
            .as_mut()
            .ok_or_else(|| BookKeeperError::Io("segment sealed".into()))?;
        let mut f = file_mtx.lock().unwrap();
        let offset = f.seek(SeekFrom::End(0))?;
        f.write_all(&entry_id.to_be_bytes())?;
        f.write_all(&(payload.len() as u32).to_be_bytes())?;
        f.write_all(payload)?;
        f.flush()?;
        self.index.insert(entry_id, offset);
        self.last_entry = Some(entry_id);
        Ok(())
    }

    fn read(&self, entry_id: EntryId) -> BkResult<Option<Vec<u8>>> {
        let Some(&offset) = self.index.get(&entry_id) else {
            return Ok(None);
        };
        let Some(file_mtx) = self.file.as_ref() else {
            // Sealed and dropped: re-open.
            let mut f = OpenOptions::new().read(true).open(&self.path)?;
            return read_at(&mut f, offset).map(Some);
        };
        let mut f = file_mtx.lock().unwrap();
        f.seek(SeekFrom::Start(offset))?;
        let mut hdr = [0u8; 12];
        f.read_exact(&mut hdr)?;
        let len = u32::from_be_bytes(hdr[8..12].try_into().unwrap()) as usize;
        let mut payload = vec![0u8; len];
        f.read_exact(&mut payload)?;
        Ok(Some(payload))
    }

    /// Persist outstanding writes; called per [`SegmentedLedger::sync`].
    fn fsync(&self) -> BkResult<()> {
        if let Some(file_mtx) = self.file.as_ref() {
            let f = file_mtx.lock().unwrap();
            f.sync_all()?;
        }
        Ok(())
    }
}

fn read_at(file: &mut File, offset: u64) -> BkResult<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut hdr = [0u8; 12];
    file.read_exact(&mut hdr)?;
    let len = u32::from_be_bytes(hdr[8..12].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; len];
    file.read_exact(&mut payload)?;
    Ok(payload)
}

// ─── Ledger metadata ──────────────────────────────────────────────────────

/// Persistent ledger metadata: ensemble and quorum sizing, fenced bit,
/// last-add-confirmed pointer, and segment boundaries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerMetadata {
    pub ledger_id: LedgerId,
    /// Bookie IDs in the ensemble; ordered.
    pub ensemble: Vec<String>,
    /// Write quorum: how many bookies a write is sent to.
    pub write_quorum: u32,
    /// Ack quorum: how many acks are required for write success.
    pub ack_quorum: u32,
    /// Whether the ledger has been fenced (immutable thereafter).
    pub fenced: bool,
    /// Last entry id that is confirmed durable (the LAC).  `None` means
    /// the ledger is empty.
    pub last_add_confirmed: Option<EntryId>,
    /// First entry of each segment (sorted ascending); always includes 0.
    pub segments: Vec<EntryId>,
}

/// Pluggable metadata persistence — backed by `cave_etcd` in production,
/// or by [`InMemoryMetadataStore`] in tests.
pub trait LedgerMetadataStore: Send + Sync {
    fn get(&self, ledger_id: LedgerId) -> BkResult<Option<LedgerMetadata>>;
    fn put(&self, meta: &LedgerMetadata) -> BkResult<()>;
    fn delete(&self, ledger_id: LedgerId) -> BkResult<()>;
    fn list(&self) -> BkResult<Vec<LedgerMetadata>>;
}

/// Default in-memory metadata store used by tests and single-node runs.
#[derive(Debug, Default, Clone)]
pub struct InMemoryMetadataStore {
    inner: Arc<Mutex<HashMap<LedgerId, LedgerMetadata>>>,
}

impl InMemoryMetadataStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LedgerMetadataStore for InMemoryMetadataStore {
    fn get(&self, ledger_id: LedgerId) -> BkResult<Option<LedgerMetadata>> {
        Ok(self.inner.lock().unwrap().get(&ledger_id).cloned())
    }
    fn put(&self, meta: &LedgerMetadata) -> BkResult<()> {
        self.inner
            .lock()
            .unwrap()
            .insert(meta.ledger_id, meta.clone());
        Ok(())
    }
    fn delete(&self, ledger_id: LedgerId) -> BkResult<()> {
        self.inner.lock().unwrap().remove(&ledger_id);
        Ok(())
    }
    fn list(&self) -> BkResult<Vec<LedgerMetadata>> {
        Ok(self.inner.lock().unwrap().values().cloned().collect())
    }
}

// ─── SegmentedLedger ──────────────────────────────────────────────────────

/// Default number of entries per segment before rollover.
pub const DEFAULT_SEGMENT_SIZE: u64 = 1024;

/// Top-level ledger handle: owns the ensemble, the live segments, and
/// the metadata cache.
pub struct SegmentedLedger {
    // NOTE: cannot derive Debug because `dyn LedgerMetadataStore` is
    // !Debug; we provide a manual impl below.
    ledger_id: LedgerId,
    dir: PathBuf,
    bookies: Arc<Mutex<Vec<Bookie>>>,
    /// All segments, oldest first; only the last one is open for writes
    /// unless the ledger is fenced.
    segments: Mutex<Vec<Segment>>,
    /// Cached metadata; the source of truth lives in `store`.
    meta: Mutex<LedgerMetadata>,
    store: Arc<dyn LedgerMetadataStore>,
    roll_at: u64,
}

impl std::fmt::Debug for SegmentedLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentedLedger")
            .field("ledger_id", &self.ledger_id)
            .field("dir", &self.dir)
            .field("meta", &self.meta)
            .field("roll_at", &self.roll_at)
            .finish()
    }
}

impl SegmentedLedger {
    /// Create a brand-new ledger.  `bookies.len()` must equal the
    /// ensemble size; `write_quorum` <= ensemble; `ack_quorum` <=
    /// `write_quorum`.
    pub fn create(
        ledger_id: LedgerId,
        dir: impl Into<PathBuf>,
        bookies: Vec<Bookie>,
        write_quorum: u32,
        ack_quorum: u32,
        store: Arc<dyn LedgerMetadataStore>,
    ) -> BkResult<Self> {
        assert!(
            ack_quorum > 0 && ack_quorum <= write_quorum,
            "Aq must satisfy 0 < Aq <= Wq"
        );
        assert!(
            write_quorum as usize <= bookies.len(),
            "Wq must be <= |ensemble|"
        );
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let ensemble = bookies.iter().map(|b| b.id.clone()).collect::<Vec<_>>();
        let meta = LedgerMetadata {
            ledger_id,
            ensemble,
            write_quorum,
            ack_quorum,
            fenced: false,
            last_add_confirmed: None,
            segments: vec![0],
        };
        store.put(&meta)?;
        let first_segment = Segment::create(&dir, 0)?;
        Ok(Self {
            ledger_id,
            dir,
            bookies: Arc::new(Mutex::new(bookies)),
            segments: Mutex::new(vec![first_segment]),
            meta: Mutex::new(meta),
            store,
            roll_at: DEFAULT_SEGMENT_SIZE,
        })
    }

    /// Re-open an existing ledger after a crash.  Replays each on-disk
    /// segment to rebuild the index and restores the LAC.
    pub fn open(
        ledger_id: LedgerId,
        dir: impl Into<PathBuf>,
        bookies: Vec<Bookie>,
        store: Arc<dyn LedgerMetadataStore>,
    ) -> BkResult<Self> {
        let dir = dir.into();
        let meta = store
            .get(ledger_id)?
            .ok_or(BookKeeperError::LedgerNotFound(ledger_id))?;
        let mut segments = Vec::with_capacity(meta.segments.len());
        for &first in &meta.segments {
            let path = dir.join(format!("segment-{:020}.log", first));
            if path.exists() {
                segments.push(Segment::open_existing(path, first)?);
            } else {
                // Segment slot exists in metadata but no file — possible
                // only for the empty initial segment.  Create it.
                segments.push(Segment::create(&dir, first)?);
            }
        }
        // Re-hydrate bookie store from on-disk entries so reads via the
        // bookie path after recovery still resolve.
        for seg in &segments {
            for (&eid, _) in seg.index.iter() {
                if let Some(payload) = seg.read(eid)? {
                    for b in bookies.iter().take(meta.write_quorum as usize) {
                        b.add_entry(ledger_id, eid, &payload);
                    }
                }
            }
        }
        Ok(Self {
            ledger_id,
            dir,
            bookies: Arc::new(Mutex::new(bookies)),
            segments: Mutex::new(segments),
            meta: Mutex::new(meta),
            store,
            roll_at: DEFAULT_SEGMENT_SIZE,
        })
    }

    /// Override the per-segment roll threshold (tests use a small value).
    pub fn roll_segment_at(&mut self, n: u64) {
        assert!(n > 0, "roll threshold must be positive");
        self.roll_at = n;
    }

    pub fn ledger_id(&self) -> LedgerId {
        self.ledger_id
    }

    pub fn metadata(&self) -> LedgerMetadata {
        self.meta.lock().unwrap().clone()
    }

    pub fn last_add_confirmed(&self) -> Option<EntryId> {
        self.meta.lock().unwrap().last_add_confirmed
    }

    pub fn is_fenced(&self) -> bool {
        self.meta.lock().unwrap().fenced
    }

    fn next_entry_id(&self) -> EntryId {
        match self.meta.lock().unwrap().last_add_confirmed {
            Some(lac) => lac + 1,
            None => 0,
        }
    }

    /// Append a payload to the ledger.  Returns the assigned `entry_id`.
    ///
    /// Quorum semantics: the entry is sent to the first `Wq` bookies in
    /// the ensemble; success once `Aq` of them ack.  On failure the LAC
    /// does not advance and no on-disk write is performed.
    pub fn append(&self, payload: &[u8]) -> BkResult<EntryId> {
        if self.is_fenced() {
            return Err(BookKeeperError::Fenced(self.ledger_id));
        }
        let entry_id = self.next_entry_id();
        let (wq, aq) = {
            let m = self.meta.lock().unwrap();
            (m.write_quorum, m.ack_quorum)
        };
        let bookies = self.bookies.lock().unwrap();
        let mut acks = 0u32;
        let mut healthy = 0u32;
        for b in bookies.iter().take(wq as usize) {
            if b.healthy {
                healthy += 1;
            }
            if b.add_entry(self.ledger_id, entry_id, payload) {
                acks += 1;
            }
        }
        if acks < aq {
            return Err(BookKeeperError::NotEnoughBookies {
                required: aq,
                available: healthy,
            });
        }
        drop(bookies);
        // Persist on disk via current segment.
        let mut segs = self.segments.lock().unwrap();
        {
            let cur = segs.last_mut().expect("at least one segment");
            cur.append(entry_id, payload)?;
            cur.fsync()?;
        }
        // Roll segment if we hit the threshold.
        let cur_first = segs.last().unwrap().first_entry;
        let cur_count = entry_id - cur_first + 1;
        if cur_count >= self.roll_at {
            let new_first = entry_id + 1;
            let new_seg = Segment::create(&self.dir, new_first)?;
            segs.push(new_seg);
            let mut m = self.meta.lock().unwrap();
            m.segments.push(new_first);
            self.store.put(&m)?;
        }
        // Update metadata LAC.
        {
            let mut m = self.meta.lock().unwrap();
            m.last_add_confirmed = Some(entry_id);
            self.store.put(&m)?;
        }
        Ok(entry_id)
    }

    /// Read a single entry.  Returns `EntryNotFound` if the entry has not
    /// been written or lies past the LAC.
    pub fn read(&self, entry_id: EntryId) -> BkResult<Vec<u8>> {
        let lac = self.last_add_confirmed();
        if lac.is_none() || entry_id > lac.unwrap() {
            return Err(BookKeeperError::EntryNotFound {
                ledger_id: self.ledger_id,
                entry_id,
            });
        }
        let segs = self.segments.lock().unwrap();
        for seg in segs.iter() {
            if entry_id >= seg.first_entry
                && entry_id <= seg.last_entry.unwrap_or(EntryId::MAX)
            {
                if let Some(payload) = seg.read(entry_id)? {
                    return Ok(payload);
                }
            }
        }
        Err(BookKeeperError::EntryNotFound {
            ledger_id: self.ledger_id,
            entry_id,
        })
    }

    /// Read up to `max` entries starting at `from` (inclusive).
    pub fn read_entries(&self, from: EntryId, max: usize) -> BkResult<Vec<(EntryId, Vec<u8>)>> {
        let Some(lac) = self.last_add_confirmed() else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(max);
        let mut eid = from;
        while eid <= lac && out.len() < max {
            match self.read(eid) {
                Ok(p) => out.push((eid, p)),
                Err(BookKeeperError::EntryNotFound { .. }) => break,
                Err(e) => return Err(e),
            }
            eid += 1;
        }
        Ok(out)
    }

    /// Fence the ledger.  No further appends succeed; readers see the
    /// exact committed prefix.  Idempotent.
    pub fn fence(&self) -> BkResult<()> {
        let mut m = self.meta.lock().unwrap();
        if m.fenced {
            return Ok(());
        }
        m.fenced = true;
        self.store.put(&m)?;
        Ok(())
    }

    /// Number of segments currently allocated.
    pub fn segment_count(&self) -> usize {
        self.segments.lock().unwrap().len()
    }
}

// ─── LedgerCursor ─────────────────────────────────────────────────────────

/// Reading cursor — advances through entries by batches.  Two cursors on
/// the same ledger are independent.
pub struct LedgerCursor {
    next: EntryId,
}

impl Default for LedgerCursor {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerCursor {
    pub fn new() -> Self {
        Self { next: 0 }
    }

    /// Position the cursor at a specific entry id.
    pub fn seek(&mut self, entry_id: EntryId) {
        self.next = entry_id;
    }

    pub fn position(&self) -> EntryId {
        self.next
    }

    /// Pull the next batch of up to `max` entries; advance the cursor by
    /// however many entries were returned.
    pub fn read_next(
        &mut self,
        ledger: &SegmentedLedger,
        max: usize,
    ) -> BkResult<Vec<(EntryId, Vec<u8>)>> {
        let batch = ledger.read_entries(self.next, max)?;
        if let Some(&(last, _)) = batch.last() {
            self.next = last + 1;
        }
        Ok(batch)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Build a fresh ledger with a 3-bookie ensemble in a unique tempdir.
    fn fresh(
        ledger_id: LedgerId,
        suffix: &str,
        wq: u32,
        aq: u32,
    ) -> (SegmentedLedger, Arc<InMemoryMetadataStore>, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-{}-{}-{}",
            ledger_id,
            suffix,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let bookies = vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")];
        let store = Arc::new(InMemoryMetadataStore::new());
        let ledger = SegmentedLedger::create(
            ledger_id,
            &dir,
            bookies,
            wq,
            aq,
            store.clone() as Arc<dyn LedgerMetadataStore>,
        )
        .unwrap();
        (ledger, store, dir)
    }

    #[test]
    fn append_assigns_monotonic_entry_ids() {
        // cite: bookkeeper 4.17.1 LedgerHandle.addEntry — monotonic entryIds
        let (l, _s, _d) = fresh(1, "mono", 3, 2);
        assert_eq!(l.append(b"a").unwrap(), 0);
        assert_eq!(l.append(b"b").unwrap(), 1);
        assert_eq!(l.append(b"c").unwrap(), 2);
        assert_eq!(l.last_add_confirmed(), Some(2));
    }

    #[test]
    fn read_round_trips_payloads() {
        // cite: bookkeeper 4.17.1 LedgerHandle.readEntries
        let (l, _s, _d) = fresh(2, "rt", 3, 2);
        l.append(b"hello").unwrap();
        l.append(b"world").unwrap();
        assert_eq!(l.read(0).unwrap(), b"hello".to_vec());
        assert_eq!(l.read(1).unwrap(), b"world".to_vec());
    }

    #[test]
    fn read_entries_batch() {
        // cite: bookkeeper 4.17.1 LedgerHandle.readEntries(first,last)
        let (l, _s, _d) = fresh(3, "batch", 3, 2);
        for i in 0..5 {
            l.append(format!("e{}", i).as_bytes()).unwrap();
        }
        let batch = l.read_entries(1, 3).unwrap();
        assert_eq!(batch.len(), 3);
        assert_eq!(batch[0], (1, b"e1".to_vec()));
        assert_eq!(batch[2], (3, b"e3".to_vec()));
    }

    #[test]
    fn read_past_lac_errors() {
        // cite: bookkeeper 4.17.1 LedgerHandle.readEntry — past LAC = OperationFailed
        let (l, _s, _d) = fresh(4, "past", 3, 2);
        l.append(b"only").unwrap();
        let err = l.read(99).unwrap_err();
        assert!(matches!(err, BookKeeperError::EntryNotFound { .. }));
    }

    #[test]
    fn write_quorum_three_ack_two_succeeds_with_two_healthy() {
        // cite: bookkeeper 4.17.1 ack-quorum Aq=2 → ok with 2/3 bookies
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-5-aq2-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut bookies = vec![
            Bookie::new("bk1"),
            Bookie::new("bk2"),
            Bookie::new("bk3"),
        ];
        bookies[2].set_healthy(false);
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        let l = SegmentedLedger::create(5, &dir, bookies, 3, 2, store).unwrap();
        assert_eq!(l.append(b"x").unwrap(), 0);
    }

    #[test]
    fn ack_quorum_failure_when_too_many_unhealthy() {
        // cite: bookkeeper 4.17.1 BKNotEnoughBookiesException
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-6-neb-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut bookies = vec![
            Bookie::new("bk1"),
            Bookie::new("bk2"),
            Bookie::new("bk3"),
        ];
        bookies[1].set_healthy(false);
        bookies[2].set_healthy(false);
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        let l = SegmentedLedger::create(6, &dir, bookies, 3, 2, store).unwrap();
        let err = l.append(b"x").unwrap_err();
        assert!(matches!(err, BookKeeperError::NotEnoughBookies { required: 2, .. }));
    }

    #[test]
    fn fence_blocks_further_writes() {
        // cite: bookkeeper 4.17.1 BKLedgerFencedException
        let (l, _s, _d) = fresh(7, "fence", 3, 2);
        l.append(b"x").unwrap();
        l.fence().unwrap();
        assert!(l.is_fenced());
        let err = l.append(b"y").unwrap_err();
        assert!(matches!(err, BookKeeperError::Fenced(7)));
    }

    #[test]
    fn fence_is_idempotent() {
        // cite: bookkeeper 4.17.1 fence operation must be idempotent
        let (l, _s, _d) = fresh(8, "fence-idem", 3, 2);
        l.fence().unwrap();
        l.fence().unwrap();
        assert!(l.is_fenced());
    }

    #[test]
    fn fenced_ledger_reads_committed_prefix() {
        // cite: bookkeeper 4.17.1 fenced ledger preserves committed entries
        let (l, _s, _d) = fresh(9, "fence-rd", 3, 2);
        l.append(b"committed-0").unwrap();
        l.append(b"committed-1").unwrap();
        l.fence().unwrap();
        assert_eq!(l.read(0).unwrap(), b"committed-0".to_vec());
        assert_eq!(l.read(1).unwrap(), b"committed-1".to_vec());
        assert!(l.read(2).is_err());
    }

    #[test]
    fn segment_rolls_at_threshold() {
        // cite: pulsar 4.2.0 ManagedLedgerImpl ledger-rollover-target
        let (mut l, _s, _d) = fresh(10, "roll", 3, 2);
        l.roll_segment_at(2);
        assert_eq!(l.segment_count(), 1);
        l.append(b"e0").unwrap();
        l.append(b"e1").unwrap();
        // After 2 entries → roll
        assert_eq!(l.segment_count(), 2);
        l.append(b"e2").unwrap();
        // 3rd entry in new segment
        assert_eq!(l.segment_count(), 2);
    }

    #[test]
    fn segment_boundary_does_not_corrupt_entry_id() {
        // cite: bookkeeper 4.17.1 entry-id is contiguous across ensembles
        let (mut l, _s, _d) = fresh(11, "bnd", 3, 2);
        l.roll_segment_at(3);
        for i in 0..7 {
            assert_eq!(l.append(format!("p{}", i).as_bytes()).unwrap(), i);
        }
        assert_eq!(l.last_add_confirmed(), Some(6));
        for i in 0..7 {
            assert_eq!(l.read(i).unwrap(), format!("p{}", i).as_bytes());
        }
    }

    #[test]
    fn cursor_advances_across_segment_boundary() {
        // cite: bookkeeper 4.17.1 ReadHandle cursor crosses segments
        let (mut l, _s, _d) = fresh(12, "cur", 3, 2);
        l.roll_segment_at(2);
        for i in 0..5 {
            l.append(format!("v{}", i).as_bytes()).unwrap();
        }
        let mut c = LedgerCursor::new();
        let b1 = c.read_next(&l, 2).unwrap();
        assert_eq!(b1.len(), 2);
        assert_eq!(b1[0].0, 0);
        let b2 = c.read_next(&l, 2).unwrap();
        assert_eq!(b2.len(), 2);
        assert_eq!(b2[0].0, 2);
        let b3 = c.read_next(&l, 2).unwrap();
        assert_eq!(b3.len(), 1);
        assert_eq!(b3[0].0, 4);
        let b4 = c.read_next(&l, 2).unwrap();
        assert!(b4.is_empty());
    }

    #[test]
    fn cursor_seek_resets_position() {
        // cite: bookkeeper 4.17.1 ReadHandle seek
        let (l, _s, _d) = fresh(13, "seek", 3, 2);
        for i in 0..3 {
            l.append(format!("s{}", i).as_bytes()).unwrap();
        }
        let mut c = LedgerCursor::new();
        c.read_next(&l, 10).unwrap();
        assert_eq!(c.position(), 3);
        c.seek(1);
        assert_eq!(c.position(), 1);
        let b = c.read_next(&l, 1).unwrap();
        assert_eq!(b[0], (1, b"s1".to_vec()));
    }

    #[test]
    fn cursor_on_empty_ledger_returns_nothing() {
        // cite: bookkeeper 4.17.1 ReadHandle empty ledger
        let (l, _s, _d) = fresh(14, "empty-cur", 3, 2);
        let mut c = LedgerCursor::new();
        assert!(c.read_next(&l, 10).unwrap().is_empty());
        assert_eq!(c.position(), 0);
    }

    #[test]
    fn ledger_metadata_persists_in_store() {
        // cite: bookkeeper 4.17.1 LedgerMetadata written to metadata store
        let (l, store, _d) = fresh(15, "meta", 3, 2);
        l.append(b"x").unwrap();
        let m = store.get(15).unwrap().unwrap();
        assert_eq!(m.ledger_id, 15);
        assert_eq!(m.last_add_confirmed, Some(0));
        assert_eq!(m.ensemble, vec!["bk1", "bk2", "bk3"]);
        assert_eq!(m.write_quorum, 3);
        assert_eq!(m.ack_quorum, 2);
        assert!(!m.fenced);
    }

    #[test]
    fn metadata_lac_advances_with_each_append() {
        // cite: bookkeeper 4.17.1 LAC update path
        let (l, store, _d) = fresh(16, "lac", 3, 2);
        for i in 0..4 {
            l.append(format!("e{}", i).as_bytes()).unwrap();
            let m = store.get(16).unwrap().unwrap();
            assert_eq!(m.last_add_confirmed, Some(i));
        }
    }

    #[test]
    fn metadata_records_segments_after_rollover() {
        // cite: bookkeeper 4.17.1 LedgerMetadata.ensembles map
        let (mut l, store, _d) = fresh(17, "seg-meta", 3, 2);
        l.roll_segment_at(2);
        for i in 0..5 {
            l.append(format!("e{}", i).as_bytes()).unwrap();
        }
        let m = store.get(17).unwrap().unwrap();
        // Initial segment at 0, +rollover after 2 = at 2, +rollover after 4 = at 4.
        assert!(m.segments.contains(&0));
        assert!(m.segments.contains(&2));
        assert!(m.segments.contains(&4));
    }

    #[test]
    fn recover_after_crash_replays_entries() {
        // cite: bookkeeper 4.17.1 Bookie startup replays journal/entry log
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-18-recover-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let bookies = vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")];
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        {
            let l = SegmentedLedger::create(18, &dir, bookies.clone(), 3, 2, store.clone()).unwrap();
            l.append(b"persist-0").unwrap();
            l.append(b"persist-1").unwrap();
            l.append(b"persist-2").unwrap();
        }
        // Drop the ledger and re-open from disk.
        let recovered =
            SegmentedLedger::open(18, &dir, bookies, store.clone()).unwrap();
        assert_eq!(recovered.last_add_confirmed(), Some(2));
        assert_eq!(recovered.read(0).unwrap(), b"persist-0".to_vec());
        assert_eq!(recovered.read(2).unwrap(), b"persist-2".to_vec());
    }

    #[test]
    fn recover_preserves_segment_boundaries() {
        // cite: bookkeeper 4.17.1 recovery reads each segment file in order
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-19-recover-segs-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let bookies = vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")];
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        {
            let mut l = SegmentedLedger::create(19, &dir, bookies.clone(), 3, 2, store.clone()).unwrap();
            l.roll_segment_at(2);
            for i in 0..5 {
                l.append(format!("p{}", i).as_bytes()).unwrap();
            }
            assert_eq!(l.segment_count(), 3);
        }
        let recovered = SegmentedLedger::open(19, &dir, bookies, store.clone()).unwrap();
        assert_eq!(recovered.segment_count(), 3);
        for i in 0..5 {
            assert_eq!(recovered.read(i).unwrap(), format!("p{}", i).as_bytes());
        }
    }

    #[test]
    fn recover_fenced_ledger_remains_fenced() {
        // cite: bookkeeper 4.17.1 fenced bit is durable
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-20-fenced-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let bookies = vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")];
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        {
            let l = SegmentedLedger::create(20, &dir, bookies.clone(), 3, 2, store.clone()).unwrap();
            l.append(b"a").unwrap();
            l.fence().unwrap();
        }
        let recovered = SegmentedLedger::open(20, &dir, bookies, store.clone()).unwrap();
        assert!(recovered.is_fenced());
        let err = recovered.append(b"b").unwrap_err();
        assert!(matches!(err, BookKeeperError::Fenced(_)));
    }

    #[test]
    fn empty_ledger_lac_is_none() {
        // cite: bookkeeper 4.17.1 empty-ledger LAC = -1 (None in Rust API)
        let (l, _s, _d) = fresh(21, "empty", 3, 2);
        assert_eq!(l.last_add_confirmed(), None);
        let err = l.read(0).unwrap_err();
        assert!(matches!(err, BookKeeperError::EntryNotFound { .. }));
    }

    #[test]
    fn metadata_store_delete_clears_entry() {
        // cite: bookkeeper 4.17.1 LedgerManager.removeLedgerMetadata
        let (l, store, _d) = fresh(22, "del", 3, 2);
        l.append(b"x").unwrap();
        assert!(store.get(22).unwrap().is_some());
        store.delete(22).unwrap();
        assert!(store.get(22).unwrap().is_none());
    }

    #[test]
    fn metadata_store_lists_all_ledgers() {
        // cite: bookkeeper 4.17.1 LedgerManager.asyncProcessLedgers
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        for i in 0..3 {
            let dir = std::env::temp_dir().join(format!(
                "cave-streams-bk-list-{}-{}",
                i,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            SegmentedLedger::create(
                100 + i,
                &dir,
                vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")],
                3,
                2,
                store.clone(),
            )
            .unwrap();
        }
        let all = store.list().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn ack_quorum_one_succeeds_with_minimum_acks() {
        // cite: bookkeeper 4.17.1 Aq=1 single-ack ledger
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-23-aq1-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut bookies = vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")];
        bookies[1].set_healthy(false);
        bookies[2].set_healthy(false);
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        let l = SegmentedLedger::create(23, &dir, bookies, 3, 1, store).unwrap();
        assert_eq!(l.append(b"a").unwrap(), 0);
    }

    #[test]
    fn ledger_id_round_trips() {
        // cite: bookkeeper 4.17.1 LedgerHandle.getId
        let (l, _s, _d) = fresh(24, "id", 3, 2);
        assert_eq!(l.ledger_id(), 24);
    }

    #[test]
    fn read_entries_bounds_at_lac() {
        // cite: bookkeeper 4.17.1 LedgerHandle.readEntries clamps to LAC
        let (l, _s, _d) = fresh(25, "bound", 3, 2);
        for i in 0..3 {
            l.append(format!("e{}", i).as_bytes()).unwrap();
        }
        let b = l.read_entries(0, 100).unwrap();
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn read_entries_max_zero_returns_empty() {
        // cite: bookkeeper 4.17.1 readEntries(0) is well-defined
        let (l, _s, _d) = fresh(26, "max0", 3, 2);
        l.append(b"x").unwrap();
        assert!(l.read_entries(0, 0).unwrap().is_empty());
    }

    #[test]
    fn metadata_records_fenced_bit() {
        // cite: bookkeeper 4.17.1 metadata.fenced is persisted
        let (l, store, _d) = fresh(27, "fenced-meta", 3, 2);
        l.fence().unwrap();
        let m = store.get(27).unwrap().unwrap();
        assert!(m.fenced);
    }

    #[test]
    fn open_missing_metadata_errors() {
        // cite: bookkeeper 4.17.1 BKNoSuchLedgerExistsException
        let dir = std::env::temp_dir().join(format!(
            "cave-streams-bk-28-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store: Arc<dyn LedgerMetadataStore> = Arc::new(InMemoryMetadataStore::new());
        let err = SegmentedLedger::open(
            28,
            &dir,
            vec![Bookie::new("bk1"), Bookie::new("bk2"), Bookie::new("bk3")],
            store,
        )
        .unwrap_err();
        assert!(matches!(err, BookKeeperError::LedgerNotFound(28)));
    }
}
