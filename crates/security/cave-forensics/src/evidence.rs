// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Evidence + chain-of-custody primitives. The chain is recorded as a
//! linked list of `CustodyEntry` items, each carrying a SHA-256 hash of
//! the previous entry, an actor identity, and a wall-clock timestamp.
//!
//! Cave-forensics value-add on top of Tetragon: ingested events get
//! frozen into evidence items so they survive WORM rotation.

use crate::error::{ForensicsError, Result};
use crate::models::{EvidenceItem, EvidenceType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// One link in the chain of custody.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustodyEntry {
    pub actor: String,
    pub action: String,
    pub at: DateTime<Utc>,
    /// Hex SHA-256 of the previous entry (or "0"*64 for the first).
    pub prev_hash: String,
}

impl CustodyEntry {
    pub fn genesis(actor: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            action: action.into(),
            at: Utc::now(),
            prev_hash: "0".repeat(64),
        }
    }

    pub fn following(prev: &CustodyEntry, actor: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            action: action.into(),
            at: Utc::now(),
            prev_hash: hash(prev),
        }
    }
}

pub fn hash(entry: &CustodyEntry) -> String {
    let mut h = Sha256::new();
    h.update(entry.actor.as_bytes());
    h.update(b"|");
    h.update(entry.action.as_bytes());
    h.update(b"|");
    h.update(entry.at.to_rfc3339().as_bytes());
    h.update(b"|");
    h.update(entry.prev_hash.as_bytes());
    format!("{:x}", h.finalize())
}

/// Verify that a chain is well-formed — every entry after the first has
/// a `prev_hash` equal to the SHA-256 hash of the previous entry.
pub fn verify_chain(entries: &[CustodyEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let first = &entries[0];
    if first.prev_hash != "0".repeat(64) {
        return Err(ForensicsError::ChainBroken(
            "first entry prev_hash must be the zero hash".into(),
        ));
    }
    for w in entries.windows(2) {
        let expected = hash(&w[0]);
        if w[1].prev_hash != expected {
            return Err(ForensicsError::ChainBroken(format!(
                "broken link: expected prev_hash={} got={}",
                expected, w[1].prev_hash
            )));
        }
    }
    Ok(())
}

/// Build an `EvidenceItem` from a raw payload, automatically computing
/// the SHA-256 + initialising a genesis chain-of-custody entry.
pub fn build_evidence_item(
    evidence_type: EvidenceType,
    description: impl Into<String>,
    payload: &[u8],
    actor: impl Into<String>,
) -> EvidenceItem {
    let mut h = Sha256::new();
    h.update(payload);
    let hex = format!("{:x}", h.finalize());
    let actor_s: String = actor.into();
    let _genesis = CustodyEntry::genesis(actor_s.clone(), "collect");
    EvidenceItem {
        id: Uuid::new_v4(),
        evidence_type,
        description: description.into(),
        hash_sha256: Some(hex),
        collected_at: Utc::now(),
        chain_of_custody: vec![format!("{actor_s}:collect")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_entry_has_zero_prev_hash() {
        let e = CustodyEntry::genesis("alice", "collect");
        assert_eq!(e.prev_hash, "0".repeat(64));
        assert_eq!(e.actor, "alice");
    }

    #[test]
    fn test_following_entry_links_to_previous() {
        let g = CustodyEntry::genesis("alice", "collect");
        let f = CustodyEntry::following(&g, "bob", "transfer");
        assert_eq!(f.prev_hash, hash(&g));
    }

    #[test]
    fn test_hash_is_deterministic_64_hex() {
        let g = CustodyEntry::genesis("alice", "collect");
        let h = hash(&g);
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(h, hash(&g));
    }

    #[test]
    fn test_verify_chain_valid_single_entry() {
        let g = CustodyEntry::genesis("alice", "collect");
        assert!(verify_chain(&[g]).is_ok());
    }

    #[test]
    fn test_verify_chain_valid_multi_entry() {
        let g = CustodyEntry::genesis("alice", "collect");
        let f = CustodyEntry::following(&g, "bob", "transfer");
        let f2 = CustodyEntry::following(&f, "carol", "seal");
        assert!(verify_chain(&[g, f, f2]).is_ok());
    }

    #[test]
    fn test_verify_chain_detects_tamper() {
        let g = CustodyEntry::genesis("alice", "collect");
        let mut f = CustodyEntry::following(&g, "bob", "transfer");
        f.prev_hash = "deadbeef".repeat(8);
        let err = verify_chain(&[g, f]).unwrap_err();
        assert!(format!("{err}").contains("broken link"));
    }

    #[test]
    fn test_verify_chain_rejects_nonzero_genesis_prev() {
        let mut g = CustodyEntry::genesis("alice", "collect");
        g.prev_hash = "1".repeat(64);
        let err = verify_chain(&[g]).unwrap_err();
        assert!(format!("{err}").contains("zero hash"));
    }

    #[test]
    fn test_empty_chain_is_ok() {
        assert!(verify_chain(&[]).is_ok());
    }

    #[test]
    fn test_build_evidence_item_assigns_sha256() {
        let it = build_evidence_item(
            EvidenceType::LogFile,
            "tetragon-stream-1.json",
            b"sample-bytes",
            "alice",
        );
        let h = it.hash_sha256.unwrap();
        assert_eq!(h.len(), 64);
        assert!(it.description.contains("tetragon"));
    }

    #[test]
    fn test_build_evidence_item_seeds_chain() {
        let it = build_evidence_item(
            EvidenceType::NetworkCapture,
            "pcap-1",
            b"x",
            "bob",
        );
        assert_eq!(it.chain_of_custody, vec!["bob:collect".to_string()]);
    }

    #[test]
    fn test_custody_entry_serde_roundtrip() {
        let g = CustodyEntry::genesis("alice", "collect");
        let j = serde_json::to_string(&g).unwrap();
        let back: CustodyEntry = serde_json::from_str(&j).unwrap();
        assert_eq!(back, g);
    }
}
