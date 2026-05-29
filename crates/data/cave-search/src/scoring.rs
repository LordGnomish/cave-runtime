// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Relevance scoring: BM25 (Okapi BM25) algorithm.
//!
//! BM25 is the standard probabilistic ranking function used by Manticore Search,
//! Elasticsearch, and most modern full-text search engines. Formula:
//!
//! score(D, Q) = Σ IDF(qi) * (tf(qi,D) * (k1+1)) / (tf(qi,D) + k1*(1-b+b*|D|/avgdl))
//!
//! where:
//!   IDF(qi) = ln((N - df(qi) + 0.5) / (df(qi) + 0.5) + 1)  [smooth IDF variant]
//!   tf(qi,D) = term frequency of qi in document D
//!   k1, b    = free parameters (defaults: k1=1.2, b=0.75)
//!   |D|      = document length in tokens
//!   avgdl    = average document length across the corpus
//!   N        = total number of documents
//!   df(qi)   = number of documents containing term qi
//!
//! upstream: manticoresoftware/manticoresearch — src/sphinxsearch.cpp BM25 ranker

use crate::analyzer::tokenize;
use crate::index::Index;

/// BM25 tuning parameters.
#[derive(Debug, Clone)]
pub struct BM25Params {
    /// Term frequency saturation parameter (Manticore default: 1.2).
    pub k1: f64,
    /// Length normalization parameter (Manticore default: 0.75).
    pub b: f64,
}

impl Default for BM25Params {
    fn default() -> Self {
        BM25Params { k1: 1.2, b: 0.75 }
    }
}

/// A document with its relevance score for a query.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredDoc {
    pub doc_id: u32,
    pub score: f64,
}

/// Compute BM25 score for a single term occurrence.
///
/// Parameters mirror the `bm25_score` function signature expected by the
/// existing skeleton and integration tests.
///
/// # Arguments
/// * `term_freq`   - tf(qi, D): how many times the query term appears in D
/// * `doc_len`     - |D|: length of D in tokens
/// * `avg_doc_len` - avgdl: average document length in the corpus
/// * `doc_freq`    - df(qi): number of documents containing the term
/// * `num_docs`    - N: total documents in the corpus
pub fn bm25_score(
    term_freq: u32,
    doc_len: u32,
    avg_doc_len: f64,
    doc_freq: u32,
    num_docs: u32,
) -> f64 {
    bm25_score_with_params(term_freq, doc_len, avg_doc_len, doc_freq, num_docs, &BM25Params::default())
}

/// Compute BM25 score with explicit tuning parameters.
pub fn bm25_score_with_params(
    term_freq: u32,
    doc_len: u32,
    avg_doc_len: f64,
    doc_freq: u32,
    num_docs: u32,
    params: &BM25Params,
) -> f64 {
    if term_freq == 0 {
        return 0.0;
    }

    let tf = term_freq as f64;
    let dl = doc_len as f64;
    let df = doc_freq as f64;
    let n = num_docs as f64;
    let avgdl = if avg_doc_len == 0.0 { 1.0 } else { avg_doc_len };

    // Smooth IDF (Lucene/Elasticsearch variant; avoids negative IDF for common terms).
    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

    // Length-normalized TF.
    let tf_norm = tf * (params.k1 + 1.0)
        / (tf + params.k1 * (1.0 - params.b + params.b * dl / avgdl));

    idf * tf_norm
}

/// Search `index` for a single `query_term` and return BM25-scored results.
///
/// Returns one `ScoredDoc` per document that contains the query term,
/// scored by BM25 using the global corpus statistics from `index`.
pub fn search_bm25(index: &Index, query_term: &str, params: &BM25Params) -> Vec<ScoredDoc> {
    let normalized = query_term.to_lowercase();
    let pl = match index.get_posting_list(&normalized) {
        Some(pl) => pl,
        None => return Vec::new(),
    };

    let num_docs = index.doc_count() as u32;
    let avg_doc_len = index.avg_doc_len();
    let doc_freq = pl.doc_freq();

    pl.iter()
        .map(|(doc_id, tf)| {
            let doc_len = index.doc_len(doc_id);
            let score = bm25_score_with_params(tf, doc_len, avg_doc_len, doc_freq, num_docs, params);
            ScoredDoc { doc_id, score }
        })
        .collect()
}

/// Return the top-`k` documents sorted by score (descending).
pub fn rank_results(mut docs: Vec<ScoredDoc>, k: usize) -> Vec<ScoredDoc> {
    // Sort descending by score; break ties by doc_id ascending.
    docs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.doc_id.cmp(&b.doc_id))
    });
    docs.truncate(k);
    docs
}

/// Multi-term BM25 search: tokenize `query`, score each term, accumulate
/// per-document scores (sum), return ranked results.
pub fn search_multi_term(index: &Index, query: &str, params: &BM25Params, top_k: usize) -> Vec<ScoredDoc> {
    use std::collections::HashMap;

    let terms = tokenize(query, index.tenant());
    let mut accum: HashMap<u32, f64> = HashMap::new();

    for term in &terms {
        for scored in search_bm25(index, term, params) {
            *accum.entry(scored.doc_id).or_insert(0.0) += scored.score;
        }
    }

    let docs: Vec<ScoredDoc> = accum
        .into_iter()
        .map(|(doc_id, score)| ScoredDoc { doc_id, score })
        .collect();

    rank_results(docs, top_k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bm25_zero_tf_is_zero() {
        assert_eq!(bm25_score(0, 10, 10.0, 5, 100), 0.0);
    }

    #[test]
    fn bm25_positive_for_matching() {
        let s = bm25_score(1, 10, 10.0, 1, 10);
        assert!(s > 0.0, "score must be positive; got {}", s);
    }

    #[test]
    fn rank_empty_returns_empty() {
        assert!(rank_results(vec![], 10).is_empty());
    }
}
