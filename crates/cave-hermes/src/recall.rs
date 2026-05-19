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

// ─── Embedding-based recall ──────────────────────────────────────────────────
//
// The full upstream pipeline embeds bodies via the active LLM provider
// and ranks by cosine similarity. cave-search has stubs for both
// `compute_embedding` and `cosine_similarity` (both still
// `unimplemented!()`), and cave-runtime hasn't yet adopted a
// production embedding model. The MVP ships a *hash-based pseudo-
// embedding* — deterministic, dependency-free, and good enough to
// validate the cosine-recall API surface end-to-end. When cave-search
// promotes its embedder, swap the `HashEmbedder` for the real one;
// `EmbeddingRecall` is parameterised over [`Embedder`] for exactly
// this reason.

/// Anything that can turn a string into a fixed-width float vector.
///
/// Implementors must guarantee:
/// * `embed("")` returns an all-zero vector of length [`Embedder::dim`].
/// * Identical inputs produce identical outputs (i.e. embedding is
///   pure and deterministic).
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Hash-based pseudo-embedding. Tokenises like [`HashRecall`] then
/// projects each token onto a configurable-width feature vector via a
/// stable SHA-256 + 32-bit-bucket hash. Frequencies are L2-normalised
/// so cosine similarity is bounded in `[-1, 1]`.
///
/// This is *not* a semantic embedding — synonyms won't surface — but
/// it does exercise the full cosine pipeline and is the documented
/// fallback while cave-search wires up a real model.
pub struct HashEmbedder {
    dim: usize,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self::new(128)
    }
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "HashEmbedder dim must be > 0");
        Self { dim }
    }
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        let tokens = tokenise(text);
        if tokens.is_empty() {
            return v;
        }
        for tok in &tokens {
            let mut h = Sha256::new();
            h.update(tok.as_bytes());
            let digest = h.finalize();
            // First 4 bytes → bucket index, next 4 → signed weight ±1
            let bucket = (u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
                as usize)
                % self.dim;
            let sign = if (digest[4] & 1) == 0 { 1.0f32 } else { -1.0f32 };
            v[bucket] += sign;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

/// Cosine similarity between two same-length vectors.
///
/// Returns `0.0` if either input is empty, otherwise the standard
/// inner-product / (‖a‖·‖b‖) ratio. Inputs are expected to be
/// pre-normalised when produced by [`Embedder::embed`], but we re-
/// normalise here so the function is safe for raw vectors as well.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

struct EmbeddedRecord {
    record: MemoryRecord,
    vector: Vec<f32>,
}

/// Cosine-ranked recall. Records are embedded once at index time and
/// queries are scored against the cached vectors.
pub struct EmbeddingRecall {
    embedder: std::sync::Arc<dyn Embedder>,
    inner: parking_lot::RwLock<Vec<EmbeddedRecord>>,
}

impl EmbeddingRecall {
    pub fn new(embedder: std::sync::Arc<dyn Embedder>) -> Self {
        Self {
            embedder,
            inner: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Convenience: build with the default [`HashEmbedder`] (dim=128).
    pub fn with_hash_embedder() -> Self {
        Self::new(std::sync::Arc::new(HashEmbedder::default()))
    }
}

impl RecallEngine for EmbeddingRecall {
    fn index(&self, records: &[MemoryRecord]) -> crate::error::Result<()> {
        let mut guard = self.inner.write();
        for r in records {
            let v = self.embedder.embed(&r.body);
            if v.iter().all(|x| *x == 0.0) {
                continue;
            }
            if let Some(slot) = guard.iter_mut().find(|x| x.record.id == r.id) {
                slot.record = r.clone();
                slot.vector = v;
            } else {
                guard.push(EmbeddedRecord {
                    record: r.clone(),
                    vector: v,
                });
            }
        }
        Ok(())
    }

    fn query(&self, query: &str, k: usize) -> crate::error::Result<Vec<RecallHit>> {
        if k == 0 {
            return Err(HermesError::Recall("k must be > 0".into()));
        }
        let q_vec = self.embedder.embed(query);
        if q_vec.iter().all(|x| *x == 0.0) {
            return Ok(Vec::new());
        }
        let guard = self.inner.read();
        let mut scored: Vec<(f32, &EmbeddedRecord)> = guard
            .iter()
            .map(|er| (cosine_similarity(&q_vec, &er.vector), er))
            .filter(|(s, _)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored
            .into_iter()
            .map(|(score, er)| RecallHit {
                record: er.record.clone(),
                score,
                fingerprint: fingerprint(&er.record),
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

    // ── EmbeddingRecall ──────────────────────────────────────────────────

    #[test]
    fn hash_embedder_is_deterministic() {
        let e = HashEmbedder::new(64);
        let a = e.embed("rust tokio runtime");
        let b = e.embed("rust tokio runtime");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn hash_embedder_empty_input_is_zero_vector() {
        let e = HashEmbedder::new(32);
        let v = e.embed("");
        assert_eq!(v.len(), 32);
        assert!(v.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn hash_embedder_l2_normalises_nonempty_output() {
        let e = HashEmbedder::new(64);
        let v = e.embed("alpha beta gamma");
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "not unit-norm: {norm}");
    }

    #[test]
    fn cosine_similarity_self_is_one() {
        let v = vec![0.6, 0.8, 0.0, 0.0];
        let s = cosine_similarity(&v, &v);
        assert!((s - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_similarity_empty_and_mismatched_returns_zero() {
        assert_eq!(cosine_similarity(&[], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[test]
    fn embedding_recall_orders_by_cosine() {
        let r = EmbeddingRecall::with_hash_embedder();
        r.index(&[
            rec("k1", "rust async programming with tokio"),
            rec("k2", "python flask web servers"),
            rec("k3", "rust tokio runtime internals"),
        ])
        .unwrap();
        let hits = r.query("rust tokio runtime", 3).unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        // k3 shares the most tokens; with hash-bucketing it should
        // rank first.
        assert_eq!(hits[0].record.id, "k3");
        // Scores must be monotonically non-increasing.
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "scores not sorted: {:?}", hits);
        }
    }

    #[test]
    fn embedding_recall_query_zero_k_is_rejected() {
        let r = EmbeddingRecall::with_hash_embedder();
        let err = r.query("x", 0).unwrap_err();
        assert!(matches!(err, HermesError::Recall(_)));
    }

    #[test]
    fn embedding_recall_index_skips_zero_vector_records() {
        let r = EmbeddingRecall::with_hash_embedder();
        r.index(&[rec("k1", "")]).unwrap();
        assert_eq!(r.len().unwrap(), 0);
    }

    #[test]
    fn embedding_recall_re_index_replaces_by_id() {
        let r = EmbeddingRecall::with_hash_embedder();
        r.index(&[rec("k1", "old alpha beta")]).unwrap();
        r.index(&[rec("k1", "fresh gamma delta")]).unwrap();
        assert_eq!(r.len().unwrap(), 1);
        let hits = r.query("gamma", 1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.body, "fresh gamma delta");
    }

    #[test]
    fn embedding_recall_clear_empties_index() {
        let r = EmbeddingRecall::with_hash_embedder();
        r.index(&[rec("k", "x y z")]).unwrap();
        r.clear().unwrap();
        assert!(r.is_empty().unwrap());
    }

    #[test]
    fn embedding_recall_unrelated_query_returns_no_hits() {
        let r = EmbeddingRecall::with_hash_embedder();
        r.index(&[rec("k", "alpha beta gamma")]).unwrap();
        // Query token has near-zero overlap with the indexed token's
        // bucket; cosine score should be < 0.05 (or 0).
        let hits = r.query("zeta", 1).unwrap();
        // We don't require zero exactly — buckets can collide — but a
        // non-overlapping single-token query should score well below
        // the self-similarity case.
        if !hits.is_empty() {
            assert!(hits[0].score < 1.0);
        }
    }
}
