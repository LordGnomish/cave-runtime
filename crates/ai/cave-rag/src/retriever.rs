// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Retrievers.
//!
//! All retrievers implement the langchain [`Retriever`] seam
//! (`get_relevant_documents`). Four strategies ship:
//!
//! * [`SimilarityRetriever`] — dense cosine top-k over a [`VectorStore`].
//! * [`MmrRetriever`] — Maximal Marginal Relevance: relevance traded against
//!   novelty so the result set isn't a pile of near-duplicates.
//! * [`Bm25Retriever`] — Okapi BM25 sparse lexical scoring (no embedder).
//! * [`HybridRetriever`] — fuses a dense and a sparse retriever with
//!   Reciprocal Rank Fusion.

use std::collections::BTreeMap;

use crate::document::Document;
use crate::embedding::{tokenize, Embeddings};
use crate::error::Result;
use crate::math::cosine_similarity;
use crate::vectorstore::{InMemoryVectorStore, ScoredDocument, VectorStore};

/// Retrieve documents relevant to a query.
pub trait Retriever {
    /// Return up to `k` documents relevant to `query`, best first.
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<ScoredDocument>>;
}

/// Dense cosine-similarity retriever.
pub struct SimilarityRetriever<'a> {
    store: &'a InMemoryVectorStore,
    embedder: &'a dyn Embeddings,
}

impl<'a> SimilarityRetriever<'a> {
    /// Build over a store and the embedder used to populate it.
    pub fn new(store: &'a InMemoryVectorStore, embedder: &'a dyn Embeddings) -> Self {
        SimilarityRetriever { store, embedder }
    }
}

impl Retriever for SimilarityRetriever<'_> {
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<ScoredDocument>> {
        let q = self.embedder.embed_query(query)?;
        Ok(self.store.similarity_search(&q, k))
    }
}

/// Maximal Marginal Relevance retriever (diversity-aware).
pub struct MmrRetriever<'a> {
    store: &'a InMemoryVectorStore,
    embedder: &'a dyn Embeddings,
    lambda: f32,
    fetch_k: usize,
}

impl<'a> MmrRetriever<'a> {
    /// Build over a store and its embedder (default lambda 0.5, fetch_k 20).
    pub fn new(store: &'a InMemoryVectorStore, embedder: &'a dyn Embeddings) -> Self {
        MmrRetriever {
            store,
            embedder,
            lambda: 0.5,
            fetch_k: 20,
        }
    }

    /// Relevance/diversity trade-off in `[0, 1]` (1 = pure relevance).
    pub fn with_lambda(mut self, lambda: f32) -> Self {
        self.lambda = lambda;
        self
    }

    /// How many candidates to pre-fetch before re-ranking down to `k`.
    pub fn with_fetch_k(mut self, fetch_k: usize) -> Self {
        self.fetch_k = fetch_k;
        self
    }
}

impl Retriever for MmrRetriever<'_> {
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<ScoredDocument>> {
        let q = self.embedder.embed_query(query)?;
        let candidates = self.store.similarity_search_with_vectors(&q, self.fetch_k);
        // Key each candidate by position so duplicate content never collides.
        let keyed: Vec<(String, Vec<f32>)> = candidates
            .iter()
            .enumerate()
            .map(|(i, (_, v))| (i.to_string(), v.clone()))
            .collect();
        let order = maximal_marginal_relevance(&q, &keyed, self.lambda, k);
        Ok(order
            .into_iter()
            .filter_map(|key| key.parse::<usize>().ok())
            .map(|i| candidates[i].0.clone())
            .collect())
    }
}

/// Maximal Marginal Relevance selection.
///
/// Greedily pick the candidate maximising
/// `lambda * sim(c, query) - (1 - lambda) * max_{s in selected} sim(c, s)`.
/// Returns the selected candidate keys in selection order.
pub fn maximal_marginal_relevance(
    query: &[f32],
    candidates: &[(String, Vec<f32>)],
    lambda: f32,
    k: usize,
) -> Vec<String> {
    let mut selected: Vec<usize> = Vec::new();
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();
    while selected.len() < k && !remaining.is_empty() {
        let mut best: Option<usize> = None;
        let mut best_score = f32::NEG_INFINITY;
        for &ci in &remaining {
            let rel = cosine_similarity(query, &candidates[ci].1);
            let div = selected
                .iter()
                .map(|&si| cosine_similarity(&candidates[ci].1, &candidates[si].1))
                .fold(0.0f32, f32::max);
            let score = lambda * rel - (1.0 - lambda) * div;
            if score > best_score {
                best_score = score;
                best = Some(ci);
            }
        }
        let chosen = best.expect("remaining is non-empty");
        selected.push(chosen);
        remaining.retain(|&x| x != chosen);
    }
    selected.iter().map(|&i| candidates[i].0.clone()).collect()
}

/// Okapi BM25 sparse lexical retriever.
pub struct Bm25Retriever {
    docs: Vec<Document>,
    /// per-doc token frequency
    term_freqs: Vec<BTreeMap<String, usize>>,
    /// per-doc length (token count)
    doc_lens: Vec<usize>,
    /// document frequency per term
    df: BTreeMap<String, usize>,
    avgdl: f32,
    k1: f32,
    b: f32,
}

impl Bm25Retriever {
    /// Build a BM25 index over a corpus (k1 = 1.2, b = 0.75).
    pub fn new(docs: Vec<Document>) -> Self {
        let mut term_freqs = Vec::with_capacity(docs.len());
        let mut doc_lens = Vec::with_capacity(docs.len());
        let mut df: BTreeMap<String, usize> = BTreeMap::new();
        for d in &docs {
            let toks = tokenize(&d.content);
            doc_lens.push(toks.len());
            let mut tf: BTreeMap<String, usize> = BTreeMap::new();
            for t in &toks {
                *tf.entry(t.clone()).or_insert(0) += 1;
            }
            for term in tf.keys() {
                *df.entry(term.clone()).or_insert(0) += 1;
            }
            term_freqs.push(tf);
        }
        let total: usize = doc_lens.iter().sum();
        let avgdl = if doc_lens.is_empty() {
            0.0
        } else {
            total as f32 / doc_lens.len() as f32
        };
        Bm25Retriever {
            docs,
            term_freqs,
            doc_lens,
            df,
            avgdl,
            k1: 1.2,
            b: 0.75,
        }
    }

    fn idf(&self, term: &str) -> f32 {
        let n = self.docs.len() as f32;
        let df = *self.df.get(term).unwrap_or(&0) as f32;
        // BM25 idf with +0.5 smoothing; clamped at 0 to avoid negatives.
        (((n - df + 0.5) / (df + 0.5)) + 1.0).ln().max(0.0)
    }

    fn score(&self, doc_idx: usize, query_terms: &[String]) -> f32 {
        let dl = self.doc_lens[doc_idx] as f32;
        let mut score = 0.0;
        for term in query_terms {
            let f = *self.term_freqs[doc_idx].get(term).unwrap_or(&0) as f32;
            if f == 0.0 {
                continue;
            }
            let idf = self.idf(term);
            let denom = f + self.k1 * (1.0 - self.b + self.b * dl / self.avgdl.max(1.0));
            score += idf * (f * (self.k1 + 1.0)) / denom;
        }
        score
    }
}

impl Retriever for Bm25Retriever {
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<ScoredDocument>> {
        let qterms = tokenize(query);
        let mut scored: Vec<ScoredDocument> = (0..self.docs.len())
            .map(|i| ScoredDocument {
                document: self.docs[i].clone(),
                score: self.score(i, &qterms),
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }
}

/// Fuse two (or more) ranked retrievers with Reciprocal Rank Fusion.
pub struct HybridRetriever<'a> {
    dense: &'a dyn Retriever,
    sparse: &'a dyn Retriever,
    rrf_k: usize,
}

impl<'a> HybridRetriever<'a> {
    /// Build over a dense and a sparse retriever (RRF constant 60).
    pub fn new(dense: &'a dyn Retriever, sparse: &'a dyn Retriever) -> Self {
        HybridRetriever {
            dense,
            sparse,
            rrf_k: 60,
        }
    }
}

impl Retriever for HybridRetriever<'_> {
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<ScoredDocument>> {
        let pool = (k * 4).max(10);
        let dense = self.dense.retrieve(query, pool)?;
        let sparse = self.sparse.retrieve(query, pool)?;

        // Map each document's content-id to its Document for reassembly.
        let mut by_id: BTreeMap<String, Document> = BTreeMap::new();
        let to_ranking = |hits: &[ScoredDocument], map: &mut BTreeMap<String, Document>| {
            hits.iter()
                .map(|h| {
                    let id = h.document.id();
                    map.entry(id.clone()).or_insert_with(|| h.document.clone());
                    id
                })
                .collect::<Vec<_>>()
        };
        let dr = to_ranking(&dense, &mut by_id);
        let sr = to_ranking(&sparse, &mut by_id);

        let fused = reciprocal_rank_fusion(&[dr, sr], self.rrf_k);
        Ok(fused
            .into_iter()
            .take(k)
            .filter_map(|(id, score)| by_id.get(&id).map(|d| ScoredDocument {
                document: d.clone(),
                score,
            }))
            .collect())
    }
}

/// Reciprocal Rank Fusion: `score(d) = Σ_r 1 / (rrf_k + rank_r(d) + 1)`.
///
/// Returns `(key, fused_score)` pairs sorted by score descending, ties broken
/// by key ascending for determinism.
pub fn reciprocal_rank_fusion(rankings: &[Vec<String>], rrf_k: usize) -> Vec<(String, f32)> {
    let mut scores: BTreeMap<String, f32> = BTreeMap::new();
    for ranking in rankings {
        for (rank, key) in ranking.iter().enumerate() {
            *scores.entry(key.clone()).or_insert(0.0) +=
                1.0 / (rrf_k as f32 + rank as f32 + 1.0);
        }
    }
    let mut out: Vec<(String, f32)> = scores.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    out
}
