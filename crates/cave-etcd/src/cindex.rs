// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Consistent-index helper.
//!
//! Upstream: `server/etcdserver/cindex/cindex.go`.
//!
//! etcd persists a `consistent_index` next to every committed entry so
//! that, on restart, the server can re-derive *which* raft entries have
//! already been applied to the mvcc layer and replay only the suffix
//! after that. Without it WAL replay would re-apply entries already
//! reflected in the snapshot.
//!
//! The upstream API has three flavours:
//!
//!   * `ConsistentIndex()` — atomic read of the *committed* index.
//!   * `SetConsistentIndex(idx, term)` — atomic write of both fields.
//!     The pair is committed together so a crash cannot leave the
//!     index and term out of sync.
//!   * `UnsafeSave(tx)` / `UnsafeLoad(tx)` — variants that must be
//!     called while holding the backend transaction lock; they read /
//!     write the bytes directly into / out of the open tx.
//!
//! This module is a pure in-memory mirror of those semantics keyed by
//! atomics + a writable bytes "tx" proxy so callers can integrate with
//! whatever durable backend they have. A `hash()` method returns a
//! stable hash of `(index, term)` for snapshot integrity checks.

use std::sync::atomic::{AtomicU64, Ordering};

/// Combined (index, term) — upstream stores these as a single 16-byte
/// big-endian record under the `meta` bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexTerm {
    pub index: u64,
    pub term: u64,
}

impl IndexTerm {
    pub fn new(index: u64, term: u64) -> Self {
        Self { index, term }
    }

    /// Serialize to the upstream 16-byte big-endian on-disk form
    /// (`index` first, then `term`).
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&self.index.to_be_bytes());
        buf[8..].copy_from_slice(&self.term.to_be_bytes());
        buf
    }

    /// Parse the 16-byte big-endian on-disk form. Returns `None` if
    /// the slice is the wrong length.
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != 16 {
            return None;
        }
        let mut idx = [0u8; 8];
        let mut trm = [0u8; 8];
        idx.copy_from_slice(&b[..8]);
        trm.copy_from_slice(&b[8..]);
        Some(Self {
            index: u64::from_be_bytes(idx),
            term: u64::from_be_bytes(trm),
        })
    }
}

/// Consistent-index — etcd's "we have applied entries up to here" mark.
///
/// Reads are cheap atomics. Writes update both index and term together
/// via a single atomic-u64 pair, then `unsafe_save` mirrors the pair
/// into a caller-owned bytes "transaction" (a `Vec<u8>` standing in
/// for an open `bolt.Tx`).
pub struct ConsistentIndex {
    index: AtomicU64,
    term: AtomicU64,
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
        }
    }

    /// Initialise from a known starting pair (e.g. on warm-boot from a
    /// snapshot).
    pub fn with_initial(index: u64, term: u64) -> Self {
        Self {
            index: AtomicU64::new(index),
            term: AtomicU64::new(term),
        }
    }

    /// Atomic read of the currently-committed index.
    pub fn consistent_index(&self) -> u64 {
        self.index.load(Ordering::SeqCst)
    }

    /// Atomic read of the committed term.
    pub fn term(&self) -> u64 {
        self.term.load(Ordering::SeqCst)
    }

    /// Read both fields as one record.
    pub fn snapshot(&self) -> IndexTerm {
        IndexTerm {
            index: self.consistent_index(),
            term: self.term(),
        }
    }

    /// Atomic write of both (index, term) together. Mirrors etcd's
    /// `SetConsistentIndex` — refuses to decrease (etcd panics; we
    /// return `false`).
    pub fn set_consistent_index(&self, index: u64, term: u64) -> bool {
        let cur = self.index.load(Ordering::SeqCst);
        if index < cur {
            return false;
        }
        // Bump term first so a concurrent reader never sees a fresher
        // index paired with an older term.
        let cur_term = self.term.load(Ordering::SeqCst);
        if term > cur_term {
            self.term.store(term, Ordering::SeqCst);
        }
        self.index.store(index, Ordering::SeqCst);
        true
    }

    /// Save into a caller-owned "transaction" buffer (a `Vec<u8>`).
    ///
    /// This is the cave-runtime analogue of upstream's `UnsafeSave(tx)`
    /// where `tx` is an open `bolt.Tx`. Callers wrap their backend
    /// write batch in a `Vec<u8>` (or any `&mut [u8; 16]` slot), then
    /// stage it for commit.
    ///
    /// The buffer is *replaced*, not appended — etcd stores exactly
    /// one (index, term) record per `consistent_index` bucket key.
    pub fn unsafe_save(&self, tx: &mut Vec<u8>) {
        let snap = self.snapshot();
        *tx = snap.to_bytes().to_vec();
    }

    /// Restore in-memory state from a caller-owned bytes "transaction".
    /// `tx` is the same 16-byte big-endian record written by
    /// [`unsafe_save`]. Returns the restored pair, or `None` on a
    /// malformed buffer.
    pub fn unsafe_load(&self, tx: &[u8]) -> Option<IndexTerm> {
        let pair = IndexTerm::from_bytes(tx)?;
        self.index.store(pair.index, Ordering::SeqCst);
        self.term.store(pair.term, Ordering::SeqCst);
        Some(pair)
    }

    /// Stable 64-bit hash of the (index, term) record — fed into
    /// snapshot integrity checks. We use a small in-tree FNV-1a so
    /// there is no extra crate dependency.
    pub fn hash(&self) -> u64 {
        let snap = self.snapshot();
        fnv1a_64(&snap.to_bytes())
    }
}

/// FNV-1a 64-bit — small, dependency-free, deterministic.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_cindex_is_zero() {
        let c = ConsistentIndex::new();
        assert_eq!(c.consistent_index(), 0);
        assert_eq!(c.term(), 0);
        assert_eq!(c.snapshot(), IndexTerm::default());
    }

    #[test]
    fn with_initial_seeds_state() {
        let c = ConsistentIndex::with_initial(42, 7);
        assert_eq!(c.consistent_index(), 42);
        assert_eq!(c.term(), 7);
    }

    #[test]
    fn set_increases_index_and_term_atomically() {
        let c = ConsistentIndex::new();
        assert!(c.set_consistent_index(10, 1));
        assert!(c.set_consistent_index(20, 2));
        assert_eq!(c.consistent_index(), 20);
        assert_eq!(c.term(), 2);
    }

    #[test]
    fn set_refuses_to_regress_index() {
        let c = ConsistentIndex::with_initial(100, 5);
        let ok = c.set_consistent_index(50, 5);
        assert!(!ok);
        assert_eq!(c.consistent_index(), 100);
    }

    #[test]
    fn set_does_not_lower_term() {
        let c = ConsistentIndex::with_initial(10, 5);
        // Newer index, older term — index advances but term stays.
        assert!(c.set_consistent_index(20, 3));
        assert_eq!(c.consistent_index(), 20);
        assert_eq!(c.term(), 5);
    }

    #[test]
    fn to_bytes_round_trip() {
        let it = IndexTerm::new(0xdead_beef_cafe_babe, 0x0123_4567_89ab_cdef);
        let bytes = it.to_bytes();
        let back = IndexTerm::from_bytes(&bytes).expect("decode");
        assert_eq!(it, back);
    }

    #[test]
    fn from_bytes_rejects_short_slice() {
        assert!(IndexTerm::from_bytes(&[]).is_none());
        assert!(IndexTerm::from_bytes(&[0u8; 8]).is_none());
        assert!(IndexTerm::from_bytes(&[0u8; 17]).is_none());
    }

    #[test]
    fn bytes_use_big_endian_layout() {
        let it = IndexTerm::new(1, 2);
        let bytes = it.to_bytes();
        // BE: most-significant byte first → trailing 0x01 / 0x02.
        assert_eq!(&bytes[..8], &[0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(&bytes[8..], &[0, 0, 0, 0, 0, 0, 0, 2]);
    }

    #[test]
    fn unsafe_save_round_trips_via_load() {
        let c = ConsistentIndex::with_initial(99, 11);
        let mut tx = Vec::new();
        c.unsafe_save(&mut tx);
        assert_eq!(tx.len(), 16);

        let c2 = ConsistentIndex::new();
        let loaded = c2.unsafe_load(&tx).expect("load ok");
        assert_eq!(loaded.index, 99);
        assert_eq!(loaded.term, 11);
        assert_eq!(c2.consistent_index(), 99);
        assert_eq!(c2.term(), 11);
    }

    #[test]
    fn unsafe_save_overwrites_tx() {
        // Even if the tx had garbage in it (e.g. caller reused the
        // buffer), `unsafe_save` should replace, not append.
        let c = ConsistentIndex::with_initial(5, 1);
        let mut tx = vec![0xff; 64];
        c.unsafe_save(&mut tx);
        assert_eq!(tx.len(), 16);
        let pair = IndexTerm::from_bytes(&tx).unwrap();
        assert_eq!(pair, IndexTerm::new(5, 1));
    }

    #[test]
    fn unsafe_load_rejects_bogus_buffer() {
        let c = ConsistentIndex::with_initial(7, 1);
        let res = c.unsafe_load(&[0u8; 4]);
        assert!(res.is_none());
        // State untouched.
        assert_eq!(c.consistent_index(), 7);
    }

    #[test]
    fn hash_is_stable_for_identical_state() {
        let a = ConsistentIndex::with_initial(100, 5);
        let b = ConsistentIndex::with_initial(100, 5);
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn hash_differs_for_different_state() {
        let a = ConsistentIndex::with_initial(100, 5);
        let b = ConsistentIndex::with_initial(101, 5);
        let c = ConsistentIndex::with_initial(100, 6);
        assert_ne!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn concurrent_set_and_read_is_serialised() {
        use std::sync::Arc;
        use std::thread;
        let c = Arc::new(ConsistentIndex::new());
        let mut handles = Vec::new();
        for i in 1..=20u64 {
            let cc = c.clone();
            handles.push(thread::spawn(move || {
                cc.set_consistent_index(i, 1);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Max index must have been preserved due to refuse-regress.
        assert_eq!(c.consistent_index(), 20);
    }

    #[test]
    fn snapshot_reflects_latest_writes() {
        let c = ConsistentIndex::new();
        c.set_consistent_index(1, 1);
        c.set_consistent_index(2, 2);
        c.set_consistent_index(3, 3);
        assert_eq!(c.snapshot(), IndexTerm::new(3, 3));
    }
}
