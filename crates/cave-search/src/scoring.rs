// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Relevance scoring — Okapi BM25.
//!
//! Mirrors Lucene `BM25Similarity` defaults (k1=1.2, b=0.75) which is also
//! the OpenSearch / Manticore default.  Upstream reference:
//! opensearch v3.0/server/src/main/java/org/opensearch/index/similarity/BM25Similarity.java
//!
//! Formula:
//!   idf = ln( (N − n + 0.5) / (n + 0.5) + 1 )
//!   tf' = tf · (k1 + 1) / ( tf + k1 · (1 − b + b · |D| / avg|D|) )
//!   score = idf · tf'
//!
//! Returns `0.0` when `tf == 0` so callers can blindly sum per-term scores.

const K1: f64 = 1.2;
const B: f64 = 0.75;

pub fn bm25_score(
    term_freq: u32,
    doc_len: u32,
    avg_doc_len: f64,
    doc_freq: u32,
    num_docs: u32,
) -> f64 {
    if term_freq == 0 {
        return 0.0;
    }
    let tf = f64::from(term_freq);
    let df = f64::from(doc_freq);
    let n = f64::from(num_docs);
    let dl = f64::from(doc_len);
    let avgdl = if avg_doc_len > 0.0 { avg_doc_len } else { 1.0 };

    // Lucene-style smoothed IDF: ln(1 + (N − n + 0.5) / (n + 0.5)).
    // Always non-negative — even if df > N/2, the +1 inside the ln keeps it
    // away from negative scores that would surprise callers summing terms.
    let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();

    let norm = 1.0 - B + B * (dl / avgdl);
    let tf_component = tf * (K1 + 1.0) / (tf + K1 * norm);
    idf * tf_component
}
