// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Semantic recall.
//!
//! Hermes' upstream recall pipeline embeds memory bodies via the active
//! provider and ranks by cosine similarity. We don't have an embedding
//! model in the workspace yet (cave-search ships skeleton FAISS but
//! isn't wired to a generator), so the MVP ships a *hash + token-overlap*
//! fallback that mimics the API:
//!
//! * [`HashRecall`] — deterministic, embedding-free. Returns hits ranked
//!   by Jaccard token overlap.
//! * Roadmap: `EmbeddingRecall` once cave-search exposes an `embed`
//!   trait (see `PARITY_REPORT.md §6`).

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::HermesError;
use crate::memory::MemoryRecord;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallHit {
    pub record: MemoryRecord,
    pub score: f32,
    pub fingerprint: String,
}

pub trait RecallEngine: Send + Sync {
    /// Index `records` so subsequent calls to `query` may surface them.
    fn index(&self, records: &[MemoryRecord]) -> crate::error::Result<()>;

    /// Return the top-`k` records whose token-overlap with `query` is
    /// positive, sorted by descending score.
    fn query(&self, query: &str, k: usize) -> crate::error::Result<Vec<RecallHit>>;

    /// Drop everything from the index. Used on session reset.
    fn clear(&self) -> crate::error::Result<()>;

    /// Number of records currently in the index.
    fn len(&self) -> crate::error::Result<usize>;

    fn is_empty(&self) -> crate::error::Result<bool> {
        Ok(self.len()? == 0)
    }
}

/// Embedding-free recall. Records are tokenised once at index time;
/// queries are scored by Jaccard overlap.
#[derive(Default)]
pub struct HashRecall {
    inner: parking_lot::RwLock<Vec<IndexedRecord>>,
}

struct IndexedRecord {
    record: MemoryRecord,
    tokens: BTreeSet<String>,
}

impl HashRecall {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RecallEngine for HashRecall {
    fn index(&self, records: &[MemoryRecord]) -> crate::error::Result<()> {
        let mut guard = self.inner.write();
        for r in records {
            let tokens = tokenise(&r.body);
            if tokens.is_empty() {
                continue;
            }
            // Replace by id if already present.
            if let Some(slot) = guard.iter_mut().find(|x| x.record.id == r.id) {
                slot.record = r.clone();
                slot.tokens = tokens;
            } else {
                guard.push(IndexedRecord {
                    record: r.clone(),
                    tokens,
                });
            }
        }
        Ok(())
    }

    fn query(&self, query: &str, k: usize) -> crate::error::Result<Vec<RecallHit>> {
        if k == 0 {
            return Err(HermesError::Recall("k must be > 0".into()));
        }
        let q_tokens = tokenise(query);
        if q_tokens.is_empty() {
            return Ok(Vec::new());
        }
        let guard = self.inner.read();
        let mut scored: Vec<(f32, &IndexedRecord)> = guard
            .iter()
            .filter_map(|ir| {
                let score = jaccard(&q_tokens, &ir.tokens);
                if score > 0.0 {
                    Some((score, ir))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored
            .into_iter()
            .map(|(score, ir)| RecallHit {
                record: ir.record.clone(),
                score,
                fingerprint: fingerprint(&ir.record),
            })
            .collect())
    }

    fn clear(&self) -> crate::error::Result<()> {
        self.inner.write().clear();
        Ok(())
    }

    fn len(&self) -> crate::error::Result<usize> {
        Ok(self.inner.read().len())
    }
}

fn tokenise(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .filter(|t| !STOP.contains(&t.as_str()))
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_set: HashSet<&String> = a.iter().collect();
    let b_set: HashSet<&String> = b.iter().collect();
    let inter = a_set.intersection(&b_set).count();
    let union = a_set.union(&b_set).count();
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

/// Stable SHA-256 fingerprint for a record (`id|body` digest).
fn fingerprint(rec: &MemoryRecord) -> String {
    let mut h = Sha256::new();
    h.update(rec.id.as_bytes());
    h.update(b"|");
    h.update(rec.body.as_bytes());
    hex::encode(h.finalize())
}

const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "in", "on", "for", "to", "with", "from", "is",
    "are", "was", "were", "be", "been", "it", "this", "that", "as", "at", "by",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, body: &str) -> MemoryRecord {
        MemoryRecord::new(id, "s", body)
    }

    #[test]
    fn empty_index_returns_no_hits() {
        let r = HashRecall::new();
        assert!(r.query("anything", 5).unwrap().is_empty());
        assert!(r.is_empty().unwrap());
    }

    #[test]
    fn k_zero_is_rejected() {
        let r = HashRecall::new();
        let err = r.query("x", 0).unwrap_err();
        assert!(matches!(err, HermesError::Recall(_)));
    }

    #[test]
    fn jaccard_ranks_overlapping_records_higher() {
        let r = HashRecall::new();
        r.index(&[
            rec("k1", "rust async programming with tokio"),
            rec("k2", "python flask web servers"),
            rec("k3", "rust tokio runtime internals"),
        ])
        .unwrap();
        let hits = r.query("rust tokio runtime", 2).unwrap();
        assert_eq!(hits.len(), 2);
        // k3 shares more tokens than k1, so it should be #1.
        assert_eq!(hits[0].record.id, "k3");
        assert!(hits[0].score >= hits[1].score);
        // k2 has zero overlap and must be filtered.
        assert!(hits.iter().all(|h| h.record.id != "k2"));
    }

    #[test]
    fn fingerprint_is_stable_and_64_hex_chars() {
        let r = rec("k", "hello");
        let fp = fingerprint(&r);
        assert_eq!(fp.len(), 64);
        // Same input → same output.
        assert_eq!(fp, fingerprint(&r));
    }

    #[test]
    fn re_index_replaces_existing_by_id() {
        let r = HashRecall::new();
        r.index(&[rec("k1", "old body alpha beta")]).unwrap();
        r.index(&[rec("k1", "fresh body gamma delta")]).unwrap();
        assert_eq!(r.len().unwrap(), 1);
        let hits = r.query("gamma", 1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.body, "fresh body gamma delta");
    }

    #[test]
    fn clear_empties_index() {
        let r = HashRecall::new();
        r.index(&[rec("k1", "x")]).unwrap();
        r.clear().unwrap();
        assert!(r.is_empty().unwrap());
    }

    #[test]
    fn stopwords_are_dropped_from_tokens() {
        let toks = tokenise("the quick brown fox");
        assert!(!toks.contains("the"));
        assert!(toks.contains("quick"));
    }

    #[test]
    fn empty_records_are_skipped_at_index_time() {
        let r = HashRecall::new();
        r.index(&[rec("k1", "")]).unwrap();
        assert_eq!(r.len().unwrap(), 0);
    }
}
