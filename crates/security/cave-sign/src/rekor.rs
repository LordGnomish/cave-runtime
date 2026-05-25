// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rekor transparency log client.
//!
//! Maps to:
//!   * pkg/cosign/rekor_factory.go  → RekorClient construction
//!   * pkg/cosign/tlog.go           → upload + lookup
//!   * cmd/cosign/cli/rekor         → rekor sub-command surface
//!
//! Rekor stores a Merkle-tree of (artifact_digest, signature, public_key)
//! triples. Every keyless signature lands in Rekor; verification later
//! consults the log to prove "this signature existed at time T".

use crate::error::{Result, SignError};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const PUBLIC_GOOD_REKOR_URL: &str = "https://rekor.sigstore.dev";
pub const CAVE_REKOR_DEFAULT_URL: &str = "http://cave-rekor.cave.svc.cluster.local:3000";

/// Rekor "hashedrekord" entry — the most common kind. Other kinds
/// (`intoto`, `rfc3161`, ...) follow the same envelope structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HashedRekordEntry {
    pub digest_hex: String,
    pub signature_b64: String,
    pub public_key_pem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RekorLogEntry {
    pub uuid: String,
    pub log_index: u64,
    pub integrated_time: i64,
    pub log_id: String,
    pub body_b64: String,
    pub kind: RekorKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RekorKind {
    HashedRekord,
    Intoto,
    Rfc3161,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InclusionProof {
    pub log_index: u64,
    pub tree_size: u64,
    pub root_hash: String,
    pub hashes: Vec<String>,
}

/// Online Rekor client (HTTP) — plus an offline in-memory log for tests
/// and cave-internal smoke runs.
pub struct RekorClient {
    pub base_url: String,
    in_memory: Mutex<InMemoryLog>,
}

impl std::fmt::Debug for RekorClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RekorClient")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl Default for RekorClient {
    fn default() -> Self {
        Self {
            base_url: PUBLIC_GOOD_REKOR_URL.to_string(),
            in_memory: Mutex::new(InMemoryLog::default()),
        }
    }
}

impl RekorClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            base_url: url.into(),
            in_memory: Mutex::new(InMemoryLog::default()),
        }
    }

    /// Upload a hashedrekord entry to the in-memory log; returns the entry
    /// as Rekor would.
    pub fn upload_offline(&self, entry: HashedRekordEntry) -> Result<RekorLogEntry> {
        let mut log = self
            .in_memory
            .lock()
            .map_err(|e| SignError::Rekor(format!("lock: {}", e)))?;
        Ok(log.append(entry))
    }

    /// Look up an entry by log_index in the in-memory log.
    pub fn get_by_index_offline(&self, log_index: u64) -> Result<RekorLogEntry> {
        let log = self
            .in_memory
            .lock()
            .map_err(|e| SignError::Rekor(format!("lock: {}", e)))?;
        log.entries
            .iter()
            .find(|e| e.log_index == log_index)
            .cloned()
            .ok_or_else(|| SignError::NotFound(format!("rekor entry {}", log_index)))
    }

    /// Search the offline log by `sha256:<hex>` digest. Returns *all*
    /// signatures (a single artifact may carry many).
    pub fn search_by_digest_offline(&self, digest: &str) -> Result<Vec<RekorLogEntry>> {
        let needle = digest.strip_prefix("sha256:").unwrap_or(digest);
        let log = self
            .in_memory
            .lock()
            .map_err(|e| SignError::Rekor(format!("lock: {}", e)))?;
        let mut hits = Vec::new();
        for e in &log.entries {
            if let Ok(body) = decode_entry_body(e) {
                if body.digest_hex == needle {
                    hits.push(e.clone());
                }
            }
        }
        Ok(hits)
    }

    /// Build a Merkle inclusion proof for an entry. Hashes are produced
    /// in the order the verifier consumes them (sibling-up).
    pub fn inclusion_proof_offline(&self, log_index: u64) -> Result<InclusionProof> {
        let log = self
            .in_memory
            .lock()
            .map_err(|e| SignError::Rekor(format!("lock: {}", e)))?;
        log.build_inclusion(log_index)
    }

    /// Tree size + current root hash — produces the witness a verifier
    /// needs to bind an entry to a particular log state.
    pub fn tree_state_offline(&self) -> Result<(u64, String)> {
        let log = self
            .in_memory
            .lock()
            .map_err(|e| SignError::Rekor(format!("lock: {}", e)))?;
        Ok((log.entries.len() as u64, log.root_hash()))
    }

    /// Live HTTP upload — `POST /api/v1/log/entries`.
    pub async fn upload(&self, entry: HashedRekordEntry) -> Result<RekorLogEntry> {
        let url = format!(
            "{}/api/v1/log/entries",
            self.base_url.trim_end_matches('/')
        );
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "kind": "hashedrekord",
            "apiVersion": "0.0.1",
            "spec": {
                "data": {"hash": {"algorithm":"sha256","value": entry.digest_hex}},
                "signature": {
                    "content": entry.signature_b64,
                    "publicKey": {"content": base64::engine::general_purpose::STANDARD.encode(entry.public_key_pem.as_bytes())}
                }
            }
        });
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SignError::Rekor(format!("post {}: {}", url, e)))?;
        if !resp.status().is_success() {
            return Err(SignError::Rekor(format!("rekor status {}", resp.status())));
        }
        let j: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SignError::Rekor(format!("json decode: {}", e)))?;
        RekorLogEntry::from_rekor_json(&j)
    }
}

#[derive(Debug, Default)]
struct InMemoryLog {
    entries: Vec<RekorLogEntry>,
    /// Leaf hashes — kept in insertion order for Merkle reconstruction.
    leaves: Vec<[u8; 32]>,
}

impl InMemoryLog {
    fn append(&mut self, entry: HashedRekordEntry) -> RekorLogEntry {
        let log_index = self.entries.len() as u64;
        let body_b64 = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&entry).unwrap());
        let mut h = Sha256::new();
        h.update(body_b64.as_bytes());
        let leaf = h.finalize();
        let mut leaf_arr = [0u8; 32];
        leaf_arr.copy_from_slice(&leaf);
        self.leaves.push(leaf_arr);

        let log_entry = RekorLogEntry {
            uuid: hex::encode(leaf),
            log_index,
            integrated_time: deterministic_time(log_index),
            log_id: "cave-rekor-mock".into(),
            body_b64,
            kind: RekorKind::HashedRekord,
        };
        self.entries.push(log_entry.clone());
        log_entry
    }

    fn root_hash(&self) -> String {
        let levels = build_levels(self.leaves.clone());
        match levels.last() {
            Some(top) if !top.is_empty() => hex::encode(top[0]),
            _ => hex::encode([0u8; 32]),
        }
    }

    fn build_inclusion(&self, log_index: u64) -> Result<InclusionProof> {
        if (log_index as usize) >= self.leaves.len() {
            return Err(SignError::NotFound(format!("index {}", log_index)));
        }
        let levels = build_levels(self.leaves.clone());
        let mut hashes = Vec::new();
        let mut idx = log_index as usize;
        for level in &levels[..levels.len().saturating_sub(1)] {
            let sibling = if idx % 2 == 0 {
                level.get(idx + 1).copied().unwrap_or(level[idx])
            } else {
                level[idx - 1]
            };
            hashes.push(hex::encode(sibling));
            idx /= 2;
        }
        let root = levels
            .last()
            .and_then(|l| l.first())
            .copied()
            .unwrap_or([0u8; 32]);
        Ok(InclusionProof {
            log_index,
            tree_size: self.entries.len() as u64,
            root_hash: hex::encode(root),
            hashes,
        })
    }
}

fn build_levels(mut current: Vec<[u8; 32]>) -> Vec<Vec<[u8; 32]>> {
    let mut levels = Vec::new();
    levels.push(current.clone());
    while current.len() > 1 {
        let mut next = Vec::with_capacity((current.len() + 1) / 2);
        for pair in current.chunks(2) {
            let mut h = Sha256::new();
            h.update(pair[0]);
            if pair.len() == 2 {
                h.update(pair[1]);
            } else {
                h.update(pair[0]);
            }
            let out = h.finalize();
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&out);
            next.push(arr);
        }
        current = next;
        levels.push(current.clone());
    }
    levels
}

fn deterministic_time(seed: u64) -> i64 {
    // Pinned base + per-entry offset → reproducible smoke fixtures.
    1_700_000_000i64 + seed as i64
}

pub fn decode_entry_body(entry: &RekorLogEntry) -> Result<HashedRekordEntry> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(entry.body_b64.as_bytes())
        .map_err(|e| SignError::Rekor(format!("body base64: {}", e)))?;
    let body: HashedRekordEntry = serde_json::from_slice(&bytes)
        .map_err(|e| SignError::Rekor(format!("body json: {}", e)))?;
    Ok(body)
}

impl RekorLogEntry {
    pub fn from_rekor_json(j: &serde_json::Value) -> Result<Self> {
        // Rekor returns `{uuid: {logIndex, integratedTime, logID, body}}`.
        let map = j
            .as_object()
            .ok_or_else(|| SignError::Rekor("expected object".into()))?;
        let (uuid, val) = map
            .iter()
            .next()
            .ok_or_else(|| SignError::Rekor("empty object".into()))?;
        let log_index = val["logIndex"]
            .as_u64()
            .ok_or_else(|| SignError::Rekor("missing logIndex".into()))?;
        let integrated_time = val["integratedTime"].as_i64().unwrap_or(0);
        let log_id = val["logID"].as_str().unwrap_or("").to_string();
        let body_b64 = val["body"].as_str().unwrap_or("").to_string();
        Ok(Self {
            uuid: uuid.clone(),
            log_index,
            integrated_time,
            log_id,
            body_b64,
            kind: RekorKind::HashedRekord,
        })
    }
}

/// Top-level lookup used by callers that don't want to think about
/// online vs offline.
pub fn lookup_in_map(
    log: &BTreeMap<u64, RekorLogEntry>,
    log_index: u64,
) -> Result<&RekorLogEntry> {
    log.get(&log_index)
        .ok_or_else(|| SignError::NotFound(format!("entry {}", log_index)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(d: &str, s: &str) -> HashedRekordEntry {
        HashedRekordEntry {
            digest_hex: d.into(),
            signature_b64: s.into(),
            public_key_pem: "-----BEGIN PUBLIC KEY-----\nAAA\n-----END PUBLIC KEY-----".into(),
        }
    }

    #[test]
    fn upload_assigns_monotonic_index() {
        let c = RekorClient::default();
        let a = c.upload_offline(entry("aa", "x")).unwrap();
        let b = c.upload_offline(entry("bb", "y")).unwrap();
        assert_eq!(a.log_index, 0);
        assert_eq!(b.log_index, 1);
    }

    #[test]
    fn get_by_index_roundtrip() {
        let c = RekorClient::default();
        let e = c.upload_offline(entry("dd", "z")).unwrap();
        let back = c.get_by_index_offline(e.log_index).unwrap();
        assert_eq!(back, e);
        let body = decode_entry_body(&back).unwrap();
        assert_eq!(body.digest_hex, "dd");
    }

    #[test]
    fn search_by_digest_finds_all() {
        let c = RekorClient::default();
        c.upload_offline(entry("11", "a")).unwrap();
        c.upload_offline(entry("22", "b")).unwrap();
        c.upload_offline(entry("11", "c")).unwrap();
        let hits = c.search_by_digest_offline("sha256:11").unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_returns_empty_for_unknown() {
        let c = RekorClient::default();
        c.upload_offline(entry("11", "a")).unwrap();
        let hits = c.search_by_digest_offline("sha256:99").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn get_unknown_returns_not_found() {
        let c = RekorClient::default();
        let err = c.get_by_index_offline(42).expect_err("must fail");
        assert!(matches!(err, SignError::NotFound(_)));
    }

    #[test]
    fn tree_state_grows() {
        let c = RekorClient::default();
        let (size0, root0) = c.tree_state_offline().unwrap();
        assert_eq!(size0, 0);
        c.upload_offline(entry("aa", "x")).unwrap();
        let (size1, root1) = c.tree_state_offline().unwrap();
        assert_eq!(size1, 1);
        assert_ne!(root0, root1);
    }

    #[test]
    fn inclusion_proof_size_log_n() {
        let c = RekorClient::default();
        for i in 0..8 {
            c.upload_offline(entry(&format!("{:02x}", i), "s")).unwrap();
        }
        let proof = c.inclusion_proof_offline(0).unwrap();
        // 8 leaves → log2(8) = 3 sibling hashes.
        assert_eq!(proof.hashes.len(), 3);
        assert_eq!(proof.tree_size, 8);
    }

    #[test]
    fn inclusion_proof_unknown_index_fails() {
        let c = RekorClient::default();
        c.upload_offline(entry("aa", "x")).unwrap();
        let err = c.inclusion_proof_offline(7).expect_err("must fail");
        assert!(matches!(err, SignError::NotFound(_)));
    }

    #[test]
    fn from_rekor_json_parses() {
        let j = serde_json::json!({
            "deadbeef": {
                "logIndex": 7,
                "integratedTime": 1_700_000_007i64,
                "logID": "abc",
                "body": "YQ=="
            }
        });
        let e = RekorLogEntry::from_rekor_json(&j).unwrap();
        assert_eq!(e.uuid, "deadbeef");
        assert_eq!(e.log_index, 7);
    }

    #[test]
    fn lookup_in_map_finds() {
        let mut m = BTreeMap::new();
        let e = RekorLogEntry {
            uuid: "u".into(),
            log_index: 3,
            integrated_time: 0,
            log_id: "L".into(),
            body_b64: "".into(),
            kind: RekorKind::HashedRekord,
        };
        m.insert(3, e.clone());
        assert_eq!(lookup_in_map(&m, 3).unwrap(), &e);
        assert!(lookup_in_map(&m, 4).is_err());
    }
}
