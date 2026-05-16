// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   bookkeeper-server/src/main/java/org/apache/bookkeeper/client/LedgerHandle.java
//   bookkeeper-server/src/main/java/org/apache/bookkeeper/client/api/WriteHandle.java
//   bookkeeper-server/src/main/java/org/apache/bookkeeper/proto/BookieProtocol.java

//! BookKeeper-style segmented storage simulation.
//!
//! Apache Pulsar persists every topic as a chain of *ledgers* — each
//! ledger is an append-only, fenced-after-close segment striped across
//! a write set of *bookies*.  Three numbers define replication:
//!
//! - **E** (ensemble size) — how many bookies hold the ledger
//! - **Qw** (write quorum) — how many bookies each entry is written to
//! - **Qa** (ack quorum) — how many bookies must ack before the entry
//!   is considered durable (`addEntry` returns to the client)
//!
//! `Qa ≤ Qw ≤ E` is invariant.  cave-streams simulates the storage
//! semantics in-process: a [`BookieRing`] hosts E [`Bookie`]s and the
//! [`LedgerHandle`] writes each entry to a rotating Qw-sized slice
//! and waits for Qa acks.  No real BookKeeper wire protocol is on the
//! wire — this is the substrate that [`super::managed_ledger`] sits
//! on, not a network port.
//!
//! Differences vs upstream BookKeeper 4.2.0:
//! - No fsync — entries live in-memory only (durability is a follow-up).
//! - Single-process, no replication transport on the wire.
//! - Ensemble change on bookie failure (rolling ensemble) is tracked
//!   as `EnsembleChange` events but not yet materialised into a new
//!   write set.

use crate::error::{StreamsError, StreamsResult};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// Replication tuple (E, Qw, Qa) — see module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerQuorum {
    pub ensemble_size: u16,
    pub write_quorum: u16,
    pub ack_quorum: u16,
}

impl LedgerQuorum {
    /// Validated constructor — enforces `1 ≤ Qa ≤ Qw ≤ E`.
    pub fn new(ensemble: u16, write: u16, ack: u16) -> StreamsResult<Self> {
        if !(ack >= 1 && ack <= write && write <= ensemble) {
            return Err(StreamsError::Internal(format!(
                "invalid quorum: E={ensemble} Qw={write} Qa={ack} (need 1 ≤ Qa ≤ Qw ≤ E)"
            )));
        }
        Ok(Self {
            ensemble_size: ensemble,
            write_quorum: write,
            ack_quorum: ack,
        })
    }

    /// BookKeeper default profile (3, 2, 2).
    pub fn default_3_2_2() -> Self {
        Self {
            ensemble_size: 3,
            write_quorum: 2,
            ack_quorum: 2,
        }
    }
}

/// A simulated bookie — durable per-entry storage keyed by `(ledger_id,
/// entry_id)`.  Real BookKeeper bookies are TCP servers; here we model
/// behaviour only.
pub struct Bookie {
    pub id: u32,
    entries: Mutex<BTreeMap<(u64, u64), Vec<u8>>>,
    available: AtomicBool,
}

impl Bookie {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            entries: Mutex::new(BTreeMap::new()),
            available: AtomicBool::new(true),
        }
    }

    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    pub fn fail(&self) {
        self.available.store(false, Ordering::Release);
    }

    pub fn recover(&self) {
        self.available.store(true, Ordering::Release);
    }

    /// `Bookie.addEntry` — store the bytes; fails when the bookie is
    /// marked unavailable.
    pub fn add_entry(
        &self,
        ledger_id: u64,
        entry_id: u64,
        payload: &[u8],
    ) -> StreamsResult<()> {
        if !self.is_available() {
            return Err(StreamsError::Internal(format!(
                "bookie {} unavailable",
                self.id
            )));
        }
        self.entries
            .lock()
            .unwrap()
            .insert((ledger_id, entry_id), payload.to_vec());
        Ok(())
    }

    /// `Bookie.readEntry`.
    pub fn read_entry(&self, ledger_id: u64, entry_id: u64) -> Option<Vec<u8>> {
        self.entries
            .lock()
            .unwrap()
            .get(&(ledger_id, entry_id))
            .cloned()
    }
}

/// Ring of bookies the cluster knows about.
pub struct BookieRing {
    bookies: Vec<Bookie>,
}

impl BookieRing {
    /// Build a ring with `size` bookies numbered 0..size.
    pub fn with_size(size: u16) -> Self {
        Self {
            bookies: (0..size as u32).map(Bookie::new).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.bookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bookies.is_empty()
    }

    pub fn bookie(&self, idx: usize) -> &Bookie {
        &self.bookies[idx]
    }

    /// Currently-available bookie indices.
    pub fn available_indices(&self) -> Vec<usize> {
        self.bookies
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_available())
            .map(|(i, _)| i)
            .collect()
    }
}

/// One ledger — append-only, fenced-after-close, striped over a write
/// set rotated through the bookie ring.
pub struct LedgerHandle<'r> {
    pub ledger_id: u64,
    pub quorum: LedgerQuorum,
    ring: &'r BookieRing,
    /// Ensemble — the bookies that ever host this ledger (Qw subset
    /// per entry).
    ensemble: Vec<usize>,
    next_entry_id: AtomicU64,
    closed: AtomicBool,
    fenced: AtomicBool,
    last_add_confirmed: AtomicU64,
}

impl<'r> LedgerHandle<'r> {
    /// Open a new ledger on `ring`.  The first `quorum.ensemble_size`
    /// available bookies are reserved as the ensemble.
    pub fn create(
        ring: &'r BookieRing,
        ledger_id: u64,
        quorum: LedgerQuorum,
    ) -> StreamsResult<Self> {
        let avail = ring.available_indices();
        if (avail.len() as u16) < quorum.ensemble_size {
            return Err(StreamsError::Internal(format!(
                "not enough bookies: need {} have {}",
                quorum.ensemble_size,
                avail.len()
            )));
        }
        let ensemble = avail
            .into_iter()
            .take(quorum.ensemble_size as usize)
            .collect();
        Ok(Self {
            ledger_id,
            quorum,
            ring,
            ensemble,
            next_entry_id: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            fenced: AtomicBool::new(false),
            last_add_confirmed: AtomicU64::new(u64::MAX), // sentinel: nothing confirmed
        })
    }

    /// Compute the write set for an entry id by rotating Qw bookies
    /// through the ensemble (round-robin striping — same as BK).
    pub fn write_set_for(&self, entry_id: u64) -> Vec<usize> {
        let qw = self.quorum.write_quorum as usize;
        let e = self.ensemble.len();
        let start = (entry_id as usize) % e;
        (0..qw).map(|i| self.ensemble[(start + i) % e]).collect()
    }

    /// `LedgerHandle.addEntry` — write to Qw bookies, succeed when
    /// Qa ack.  Returns the entry id.
    pub fn add_entry(&self, payload: &[u8]) -> StreamsResult<u64> {
        if self.fenced.load(Ordering::Acquire) {
            return Err(StreamsError::Internal(format!(
                "ledger {} fenced",
                self.ledger_id
            )));
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(StreamsError::Internal(format!(
                "ledger {} closed",
                self.ledger_id
            )));
        }
        let entry_id = self.next_entry_id.fetch_add(1, Ordering::AcqRel);
        let ws = self.write_set_for(entry_id);
        let qa = self.quorum.ack_quorum as usize;
        let mut acks = 0usize;
        for &idx in &ws {
            if self
                .ring
                .bookie(idx)
                .add_entry(self.ledger_id, entry_id, payload)
                .is_ok()
            {
                acks += 1;
                if acks >= qa {
                    break;
                }
            }
        }
        if acks < qa {
            return Err(StreamsError::Internal(format!(
                "quorum not met: needed {qa} got {acks}"
            )));
        }
        // Advance LAC monotonically.
        self.last_add_confirmed.store(entry_id, Ordering::Release);
        Ok(entry_id)
    }

    /// `LedgerHandle.readEntries` — read `[start..=end]` (inclusive)
    /// from any available bookie in the write set.  Returns the
    /// successfully-read entries in order.
    pub fn read_entries(&self, start: u64, end: u64) -> StreamsResult<Vec<(u64, Vec<u8>)>> {
        if end < start {
            return Ok(vec![]);
        }
        let mut out = Vec::with_capacity((end - start + 1) as usize);
        for eid in start..=end {
            let ws = self.write_set_for(eid);
            let mut got = None;
            for &idx in &ws {
                let b = self.ring.bookie(idx);
                if b.is_available() {
                    if let Some(payload) = b.read_entry(self.ledger_id, eid) {
                        got = Some(payload);
                        break;
                    }
                }
            }
            match got {
                Some(p) => out.push((eid, p)),
                None => {
                    return Err(StreamsError::Internal(format!(
                        "entry {eid} unreadable: no available replica"
                    )))
                }
            }
        }
        Ok(out)
    }

    /// `LedgerHandle.close` — flips the closed flag; further writes
    /// reject.  Idempotent.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// `LedgerHandle.fence` — marks the ledger as fenced (used by the
    /// recovery protocol when a new writer takes over).  Writes by
    /// the original handle now fail.
    pub fn fence(&self) {
        self.fenced.store(true, Ordering::Release);
        self.closed.store(true, Ordering::Release);
    }

    /// Last add-confirmed entry id, or `None` if nothing was ever
    /// confirmed.
    pub fn last_add_confirmed(&self) -> Option<u64> {
        let v = self.last_add_confirmed.load(Ordering::Acquire);
        if v == u64::MAX {
            None
        } else {
            Some(v)
        }
    }

    pub fn entries_added(&self) -> u64 {
        self.next_entry_id.load(Ordering::Acquire)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn is_fenced(&self) -> bool {
        self.fenced.load(Ordering::Acquire)
    }

    pub fn ensemble(&self) -> &[usize] {
        &self.ensemble
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ledger_quorum_validates_ack_at_most_write_at_most_ensemble() {
        // cite: pulsar 4.2.0 ManagedLedgerConfig#setEnsembleSize/WriteQuorum/AckQuorum
        // ensemble = pl-001
        assert!(LedgerQuorum::new(3, 2, 2).is_ok());
        assert!(LedgerQuorum::new(3, 2, 3).is_err()); // Qa > Qw
        assert!(LedgerQuorum::new(2, 3, 1).is_err()); // Qw > E
        assert!(LedgerQuorum::new(3, 2, 0).is_err()); // Qa < 1
    }

    #[test]
    fn test_ledger_create_picks_ensemble_from_available_bookies() {
        // cite: pulsar 4.2.0 LedgerHandle ensemble selection
        // ensemble = pl-002
        let ring = BookieRing::with_size(5);
        let lh = LedgerHandle::create(&ring, 100, LedgerQuorum::new(3, 2, 2).unwrap()).unwrap();
        assert_eq!(lh.ensemble().len(), 3);
        assert_eq!(lh.ensemble(), &[0, 1, 2]);
    }

    #[test]
    fn test_ledger_create_fails_when_not_enough_available() {
        // cite: pulsar 4.2.0 BKNotEnoughBookiesException
        // ensemble = pl-003
        let ring = BookieRing::with_size(2);
        let err = LedgerHandle::create(&ring, 1, LedgerQuorum::new(3, 2, 2).unwrap());
        assert!(err.is_err());
    }

    #[test]
    fn test_ledger_add_entry_assigns_monotonic_ids() {
        // cite: pulsar 4.2.0 LedgerHandle.addEntry returns sequential entry ids
        // ensemble = pl-004
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::default_3_2_2()).unwrap();
        assert_eq!(lh.add_entry(b"a").unwrap(), 0);
        assert_eq!(lh.add_entry(b"b").unwrap(), 1);
        assert_eq!(lh.add_entry(b"c").unwrap(), 2);
        assert_eq!(lh.last_add_confirmed(), Some(2));
    }

    #[test]
    fn test_ledger_write_set_rotates_through_ensemble() {
        // cite: pulsar 4.2.0 striping policy (RoundRobinDistributionSchedule)
        // ensemble = pl-005
        let ring = BookieRing::with_size(4);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::new(4, 2, 1).unwrap()).unwrap();
        assert_eq!(lh.write_set_for(0), vec![0, 1]);
        assert_eq!(lh.write_set_for(1), vec![1, 2]);
        assert_eq!(lh.write_set_for(2), vec![2, 3]);
        assert_eq!(lh.write_set_for(3), vec![3, 0]); // wrap
    }

    #[test]
    fn test_ledger_add_entry_satisfies_quorum_when_one_bookie_fails() {
        // cite: pulsar 4.2.0 partial write tolerated when Qa < Qw
        // ensemble = pl-006
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::new(3, 3, 2).unwrap()).unwrap();
        ring.bookie(2).fail();
        // Qw=3 attempts, 2 succeed (bookie 2 fails), Qa=2 met.
        assert_eq!(lh.add_entry(b"x").unwrap(), 0);
    }

    #[test]
    fn test_ledger_add_entry_fails_when_quorum_not_met() {
        // cite: pulsar 4.2.0 BKNotEnoughBookiesException on write
        // ensemble = pl-007
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::new(3, 3, 3).unwrap()).unwrap();
        ring.bookie(0).fail();
        let err = lh.add_entry(b"x");
        assert!(err.is_err());
    }

    #[test]
    fn test_ledger_read_entries_returns_payloads_in_order() {
        // cite: pulsar 4.2.0 LedgerHandle.readEntries (inclusive range)
        // ensemble = pl-008
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::default_3_2_2()).unwrap();
        for b in &[b"a", b"b", b"c", b"d"] {
            lh.add_entry(b.as_slice()).unwrap();
        }
        let got = lh.read_entries(1, 2).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, 1);
        assert_eq!(got[0].1, b"b");
        assert_eq!(got[1].0, 2);
        assert_eq!(got[1].1, b"c");
    }

    #[test]
    fn test_ledger_read_entries_tolerates_one_failed_replica() {
        // cite: pulsar 4.2.0 read falls back through write set
        // ensemble = pl-009
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::default_3_2_2()).unwrap();
        lh.add_entry(b"hello").unwrap();
        ring.bookie(0).fail();
        let got = lh.read_entries(0, 0).unwrap();
        assert_eq!(got[0].1, b"hello");
    }

    #[test]
    fn test_ledger_close_blocks_further_writes() {
        // cite: pulsar 4.2.0 LedgerHandle.close (CLOSING_LEDGER status)
        // ensemble = pl-010
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::default_3_2_2()).unwrap();
        lh.add_entry(b"a").unwrap();
        lh.close();
        assert!(lh.is_closed());
        assert!(lh.add_entry(b"b").is_err());
    }

    #[test]
    fn test_ledger_fence_prevents_writes_by_original_handle() {
        // cite: pulsar 4.2.0 ledger recovery fence
        // ensemble = pl-011
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::default_3_2_2()).unwrap();
        lh.fence();
        assert!(lh.is_fenced());
        assert!(lh.add_entry(b"x").is_err());
    }

    #[test]
    fn test_ledger_last_add_confirmed_is_none_before_first_write() {
        // cite: pulsar 4.2.0 LedgerHandle.getLastAddConfirmed (-1 sentinel)
        // ensemble = pl-012
        let ring = BookieRing::with_size(3);
        let lh = LedgerHandle::create(&ring, 1, LedgerQuorum::default_3_2_2()).unwrap();
        assert_eq!(lh.last_add_confirmed(), None);
    }
}
