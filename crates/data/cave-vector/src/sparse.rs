// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sparse vectors, BM25 lexical scoring, and ColBERT multi-vector MaxSim.
//!
//! * [`SparseVector`] + [`sparse_dot`] — Qdrant sparse-vector index scoring
//!   (`lib/sparse`), a merge-join dot product over sorted index/value pairs.
//! * [`Bm25Index`] — Okapi BM25 (Robertson/Spärck-Jones), the lexical half of
//!   Qdrant/Milvus hybrid search and the `fastembed` BM25 sparse encoder.
//! * [`colbert_maxsim`] — ColBERT late-interaction MaxSim (Khattab & Zaharia
//!   2020), the multi-vector re-ranker Qdrant exposes as a `MultiVector`.

use crate::distance::Metric;
use std::collections::HashMap;

/// A sparse vector: parallel `indices` (ascending) and `values`.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseVector {
    /// Non-zero dimension indices (must be ascending).
    pub indices: Vec<u32>,
    /// Values aligned with `indices`.
    pub values: Vec<f32>,
}

/// Merge-join dot product of two sparse vectors.
pub fn sparse_dot(a: &SparseVector, b: &SparseVector) -> f32 {
    let (mut i, mut j) = (0usize, 0usize);
    let mut sum = 0.0;
    while i < a.indices.len() && j < b.indices.len() {
        match a.indices[i].cmp(&b.indices[j]) {
            std::cmp::Ordering::Equal => {
                sum += a.values[i] * b.values[j];
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    sum
}

/// Okapi BM25 index over a token-id corpus.
#[derive(Debug, Clone)]
pub struct Bm25Index {
    k1: f32,
    b: f32,
    docs: Vec<Vec<u32>>,
    doc_freq: HashMap<u32, usize>,
    total_len: usize,
}

impl Bm25Index {
    /// New empty index with the given `k1` / `b` parameters (1.2 / 0.75 typical).
    pub fn new(k1: f32, b: f32) -> Self {
        Self { k1, b, docs: Vec::new(), doc_freq: HashMap::new(), total_len: 0 }
    }

    /// Append a tokenized document; returns its doc id.
    pub fn add_document(&mut self, tokens: Vec<u32>) -> usize {
        self.total_len += tokens.len();
        // document frequency counts each distinct term once.
        let mut distinct: Vec<u32> = tokens.clone();
        distinct.sort_unstable();
        distinct.dedup();
        for t in distinct {
            *self.doc_freq.entry(t).or_insert(0) += 1;
        }
        self.docs.push(tokens);
        self.docs.len() - 1
    }

    /// Robertson IDF: `ln(1 + (N - df + 0.5)/(df + 0.5))`.
    pub fn idf(&self, term: u32) -> f32 {
        let n = self.docs.len() as f32;
        let df = *self.doc_freq.get(&term).unwrap_or(&0) as f32;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    /// Average document length.
    pub fn avgdl(&self) -> f32 {
        if self.docs.is_empty() {
            0.0
        } else {
            self.total_len as f32 / self.docs.len() as f32
        }
    }

    /// BM25 score of `query` terms against document `doc_id`.
    pub fn score(&self, query: &[u32], doc_id: usize) -> f32 {
        let Some(doc) = self.docs.get(doc_id) else {
            return 0.0;
        };
        let dl = doc.len() as f32;
        let avgdl = self.avgdl().max(f32::EPSILON);
        let norm = self.k1 * (1.0 - self.b + self.b * dl / avgdl);
        let mut score = 0.0;
        for &term in query {
            let tf = doc.iter().filter(|&&t| t == term).count() as f32;
            if tf > 0.0 {
                score += self.idf(term) * (tf * (self.k1 + 1.0)) / (tf + norm);
            }
        }
        score
    }

    /// Top-`k` documents by BM25 score (id, score), score-descending.
    pub fn search(&self, query: &[u32], k: usize) -> Vec<(usize, f32)> {
        let mut scored: Vec<(usize, f32)> =
            (0..self.docs.len()).map(|d| (d, self.score(query, d))).collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(k);
        scored
    }
}

/// ColBERT late-interaction MaxSim: `Σ_q max_d sim(q, d)` over query token
/// vectors `query` and document token vectors `doc`, using `metric`.
pub fn colbert_maxsim(query: &[Vec<f32>], doc: &[Vec<f32>], metric: Metric) -> f32 {
    if doc.is_empty() {
        return 0.0;
    }
    query
        .iter()
        .map(|q| {
            doc.iter()
                .map(|d| metric.score(q, d))
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Distance;

    fn sv(pairs: &[(u32, f32)]) -> SparseVector {
        SparseVector {
            indices: pairs.iter().map(|&(i, _)| i).collect(),
            values: pairs.iter().map(|&(_, v)| v).collect(),
        }
    }

    #[test]
    fn sparse_dot_merges_indices() {
        // shared index 2 and 5; others disjoint.
        let a = sv(&[(1, 1.0), (2, 2.0), (5, 3.0)]);
        let b = sv(&[(2, 4.0), (3, 1.0), (5, 0.5)]);
        // 2*4 + 3*0.5 = 8 + 1.5
        assert!((sparse_dot(&a, &b) - 9.5).abs() < 1e-6);
    }

    #[test]
    fn sparse_dot_disjoint_is_zero() {
        let a = sv(&[(1, 1.0), (3, 2.0)]);
        let b = sv(&[(2, 4.0), (4, 1.0)]);
        assert_eq!(sparse_dot(&a, &b), 0.0);
    }

    fn corpus() -> Bm25Index {
        let mut idx = Bm25Index::new(1.2, 0.75);
        idx.add_document(vec![1, 1, 1]); // d0: term 1 x3
        idx.add_document(vec![1, 2, 3]); // d1: term 1 x1
        idx.add_document(vec![2, 3, 4]); // d2: no term 1
        idx
    }

    #[test]
    fn bm25_rare_term_has_higher_idf() {
        // term 4 appears in 1 doc, term 1 in 2 docs → idf(4) > idf(1).
        let idx = corpus();
        assert!(idx.idf(4) > idx.idf(1));
        assert!(idx.avgdl() == 3.0);
    }

    #[test]
    fn bm25_more_occurrences_scores_higher() {
        let idx = corpus();
        let s0 = idx.score(&[1], 0);
        let s1 = idx.score(&[1], 1);
        let s2 = idx.score(&[1], 2);
        assert!(s0 > s1, "d0 {s0} should beat d1 {s1}");
        assert!(s1 > s2, "d1 {s1} should beat d2 {s2}");
        assert_eq!(s2, 0.0);
    }

    #[test]
    fn bm25_search_ranks_documents() {
        let idx = corpus();
        let hits = idx.search(&[1], 3);
        assert_eq!(hits[0].0, 0); // d0 first
        assert!(hits.windows(2).all(|w| w[0].1 >= w[1].1));
    }

    #[test]
    fn colbert_maxsim_rewards_full_coverage() {
        let metric = Metric(Distance::Cosine);
        let query = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        // docA covers both query directions; docB only the first.
        let doc_a = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let doc_b = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        let sa = colbert_maxsim(&query, &doc_a, metric);
        let sb = colbert_maxsim(&query, &doc_b, metric);
        assert!((sa - 2.0).abs() < 1e-5, "full coverage should be ~2.0, got {sa}");
        assert!(sa > sb, "full {sa} should beat partial {sb}");
    }
}
