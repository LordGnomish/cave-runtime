// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Consistent-index helper.
//!
//! Mirrors etcd v3.6.10 `server/etcdserver/cindex/cindex.go`. The
//! consistent index records the last raft applied index that has been
//! persisted to the backend, plus the matching term. On restart the
//! apply loop replays only entries strictly greater than the persisted
//! consistent index, so this counter is the single source of truth for
//! "what has already taken effect" and is used to deduplicate retries
//! after leader fail-over and to keep MVCC and raft state in lockstep.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsistentEntry {
    pub index: u64,
    pub term: u64,
}

/// Persistable consistent-index store.
///
/// Atomic getter/setter for the hot path (raft apply loop) plus a
/// gated mutex for the rare flush to disk. The on-disk encoding is
/// `<index:u64-be><term:u64-be>` — 16 bytes — written via a
/// rename-into-place tempfile so partial writes never tear the value.
pub struct ConsistentIndex {
    index: AtomicU64,
    term: AtomicU64,
    flush: Mutex<Option<std::path::PathBuf>>,
}

impl Default for ConsistentIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsistentIndex {
    pub fn new() -> Self {
        Self {
            index: AtomicU64::new(0),
            term: AtomicU64::new(0),
            flush: Mutex::new(None),
        }
    }

    pub fn load(&self) -> ConsistentEntry {
        ConsistentEntry {
            index: self.index.load(Ordering::SeqCst),
            term: self.term.load(Ordering::SeqCst),
        }
    }

    /// Monotone setter: refuses to go backwards. Returns the entry that
    /// is now visible (which may be the prior value if the call lost a
    /// race or attempted regression).
    pub fn set(&self, e: ConsistentEntry) -> ConsistentEntry {
        loop {
            let cur_idx = self.index.load(Ordering::SeqCst);
            if e.index <= cur_idx {
                return ConsistentEntry {
                    index: cur_idx,
                    term: self.term.load(Ordering::SeqCst),
                };
            }
            if self
                .index
                .compare_exchange(cur_idx, e.index, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                self.term.store(e.term, Ordering::SeqCst);
                return e;
            }
        }
    }

    /// Bind a persistence path. Subsequent [`flush`] calls write here.
    pub fn bind_path<P: AsRef<Path>>(&self, p: P) {
        *self.flush.lock().unwrap() = Some(p.as_ref().to_path_buf());
    }

    /// Rename-into-place persistence of the current (index, term) pair.
    pub fn flush(&self) -> std::io::Result<()> {
        let path = match self.flush.lock().unwrap().clone() {
            Some(p) => p,
            None => return Ok(()),
        };
        let e = self.load();
        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&e.index.to_be_bytes());
        buf[8..].copy_from_slice(&e.term.to_be_bytes());
        let tmp = path.with_extension("cindex.tmp");
        std::fs::write(&tmp, buf)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Recover the persisted entry. Missing file returns
    /// `Ok(Default::default())` so first-boot is a non-error.
    pub fn load_from_disk<P: AsRef<Path>>(p: P) -> std::io::Result<ConsistentEntry> {
        let p = p.as_ref();
        if !p.exists() {
            return Ok(ConsistentEntry::default());
        }
        let buf = std::fs::read(p)?;
        if buf.len() != 16 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("cindex file size {} != 16", buf.len()),
            ));
        }
        let index = u64::from_be_bytes(buf[..8].try_into().unwrap());
        let term = u64::from_be_bytes(buf[8..].try_into().unwrap());
        Ok(ConsistentEntry { index, term })
    }

    /// Restore an entry without triggering a disk flush.
    pub fn restore(&self, e: ConsistentEntry) {
        self.index.store(e.index, Ordering::SeqCst);
        self.term.store(e.term, Ordering::SeqCst);
    }

    /// Apply-gate: returns `true` if the raft entry should be applied
    /// (i.e. its index is strictly greater than the consistent index).
    /// Used by the apply loop to deduplicate after leader fail-over.
    pub fn should_apply(&self, raft_index: u64) -> bool {
        raft_index > self.index.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_is_zero() {
        let c = ConsistentIndex::new();
        assert_eq!(c.load(), ConsistentEntry::default());
    }

    #[test]
    fn set_advances_index_and_term() {
        let c = ConsistentIndex::new();
        let r = c.set(ConsistentEntry { index: 5, term: 1 });
        assert_eq!(r, ConsistentEntry { index: 5, term: 1 });
        assert_eq!(c.load(), ConsistentEntry { index: 5, term: 1 });
    }

    #[test]
    fn set_refuses_regression() {
        let c = ConsistentIndex::new();
        c.set(ConsistentEntry {
            index: 10,
            term: 2,
        });
        let r = c.set(ConsistentEntry { index: 5, term: 3 });
        // Stays at 10/2; the regressing attempt is rejected.
        assert_eq!(r.index, 10);
        assert_eq!(r.term, 2);
        assert_eq!(c.load(), ConsistentEntry { index: 10, term: 2 });
    }

    #[test]
    fn set_equal_is_noop() {
        let c = ConsistentIndex::new();
        c.set(ConsistentEntry { index: 7, term: 1 });
        let r = c.set(ConsistentEntry { index: 7, term: 9 });
        // index didn't advance, so term stays at the value that won.
        assert_eq!(r.index, 7);
        assert_eq!(r.term, 1);
    }

    #[test]
    fn should_apply_gates_replay() {
        let c = ConsistentIndex::new();
        c.set(ConsistentEntry { index: 5, term: 1 });
        assert!(!c.should_apply(3));
        assert!(!c.should_apply(5));
        assert!(c.should_apply(6));
    }

    #[test]
    fn restore_does_not_flush() {
        let c = ConsistentIndex::new();
        c.restore(ConsistentEntry {
            index: 42,
            term: 7,
        });
        assert_eq!(c.load(), ConsistentEntry { index: 42, term: 7 });
    }

    #[test]
    fn flush_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cindex.bin");

        let c = ConsistentIndex::new();
        c.bind_path(&path);
        c.set(ConsistentEntry {
            index: 0x1234,
            term: 9,
        });
        c.flush().unwrap();

        let recovered = ConsistentIndex::load_from_disk(&path).unwrap();
        assert_eq!(
            recovered,
            ConsistentEntry {
                index: 0x1234,
                term: 9
            }
        );
    }

    #[test]
    fn flush_without_bound_path_is_noop() {
        let c = ConsistentIndex::new();
        c.set(ConsistentEntry { index: 1, term: 1 });
        c.flush().unwrap();
    }

    #[test]
    fn load_from_disk_missing_is_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("never-written.bin");
        let e = ConsistentIndex::load_from_disk(&path).unwrap();
        assert_eq!(e, ConsistentEntry::default());
    }

    #[test]
    fn load_from_disk_rejects_short_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("short.bin");
        std::fs::write(&path, b"abc").unwrap();
        let err = ConsistentIndex::load_from_disk(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
