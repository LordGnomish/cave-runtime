// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vector embeddings + semantic similarity for search.
//!
//! This module provides:
//! - Pure mathematical operations: cosine similarity, dot product, Euclidean distance
//! - TF-IDF dense vector representation (in-crate, no external model)
//! - Stub for neural embeddings (scope-cut to cave-local-llm)
//!
//! Manticore equivalent: src/sphinxexpr.cpp knn/faiss vector search surface.
//! upstream: manticoresoftware/manticoresearch 25.8.2 — src/sphinxexpr.cpp (KNN)
//!
//! Note: compute_embedding with a real neural model is a scope-cut →
//! cave-local-llm (parallel-track). This module provides TF-IDF-based dense
//! vectors as the in-process semantic similarity backend for MVP.

use crate::analyzer::{tokenize, filter_stop_words};
use crate::tenant::TenantId;
use std::collections::{BTreeMap, HashMap};

// ── Pure vector math ──────────────────────────────────────────────────────────

/// Dot product of two equal-length vectors.
///
/// Panics if vectors have different lengths (caller responsibility).
pub fn dot_product(v1: &[f64], v2: &[f64]) -> f64 {
    assert_eq!(v1.len(), v2.len(), "dot_product: vectors must have equal length");
    v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum()
}

/// L2 (Euclidean) distance between two equal-length vectors.
pub fn euclidean_distance(v1: &[f64], v2: &[f64]) -> f64 {
    assert_eq!(v1.len(), v2.len(), "euclidean_distance: vectors must have equal length");
    v1.iter()
        .zip(v2.iter())
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Cosine similarity between two equal-length vectors.
///
/// Returns 0.0 if either vector is the zero vector (undefined similarity).
pub fn cosine_similarity(v1: &[f64], v2: &[f64]) -> f64 {
    if v1.is_empty() || v2.is_empty() {
        return 0.0;
    }
    assert_eq!(v1.len(), v2.len(), "cosine_similarity: vectors must have equal length");

    let dot = dot_product(v1, v2);
    let norm1: f64 = v1.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm2: f64 = v2.iter().map(|x| x * x).sum::<f64>().sqrt();

    if norm1 == 0.0 || norm2 == 0.0 {
        return 0.0;
    }
    // Clamp to [-1, 1] to handle floating point drift near the boundary.
    (dot / (norm1 * norm2)).clamp(-1.0, 1.0)
}

// ── TF-IDF dense vector representation ───────────────────────────────────────

/// Build a TF-IDF feature vector for `text` using the provided `corpus`.
///
/// The vocabulary is derived from all unique tokens across all corpus documents.
/// The vector is indexed by sorted vocabulary entries (deterministic order).
///
/// # Arguments
/// * `text`    - the query or document text to vectorize
/// * `corpus`  - the reference corpus (used to compute IDF weights)
/// * `tenant`  - tenant context passed to the analyzer
pub fn tfidf_vector(text: &str, corpus: &[String], tenant: &TenantId) -> Vec<f64> {
    // Build vocabulary from corpus (sorted BTreeMap for deterministic ordering).
    let vocab = build_vocab(corpus, tenant);
    if vocab.is_empty() {
        return Vec::new();
    }

    let n_docs = corpus.len() as f64;

    // Compute TF for the input text.
    let text_tokens = tokenize(text, tenant);
    let text_tf = term_freq_map(&text_tokens);

    // Compute IDF for each vocabulary term.
    let vocab_terms: Vec<&str> = vocab.keys().map(|s| s.as_str()).collect();

    // Corpus DF counts (how many documents contain each term).
    let df_map = corpus_df_map(corpus, tenant);

    vocab_terms
        .iter()
        .map(|term| {
            let tf = *text_tf.get(*term).unwrap_or(&0) as f64;
            if tf == 0.0 {
                return 0.0;
            }
            // TF = raw count / document length (normalized TF)
            let doc_len = text_tokens.len().max(1) as f64;
            let tf_norm = tf / doc_len;

            // IDF = ln(N / (1 + df)) + 1 (smooth variant, Lucene-compatible)
            let df = *df_map.get(*term).unwrap_or(&0) as f64;
            let idf = (n_docs / (1.0 + df)).ln() + 1.0;

            tf_norm * idf
        })
        .collect()
}

/// Build a sorted vocabulary map (term → index) from the corpus.
fn build_vocab(corpus: &[String], tenant: &TenantId) -> BTreeMap<String, usize> {
    let mut vocab: BTreeMap<String, usize> = BTreeMap::new();
    let mut idx = 0usize;
    for doc in corpus {
        let tokens = tokenize(doc, tenant);
        for token in tokens {
            vocab.entry(token).or_insert_with(|| {
                let i = idx;
                idx += 1;
                i
            });
        }
    }
    vocab
}

/// Compute per-term frequency map for a token list.
fn term_freq_map(tokens: &[String]) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for t in tokens {
        *map.entry(t.clone()).or_insert(0) += 1;
    }
    map
}

/// Compute document-frequency map from corpus (how many docs contain each term).
fn corpus_df_map(corpus: &[String], tenant: &TenantId) -> HashMap<String, u32> {
    let mut df: HashMap<String, u32> = HashMap::new();
    for doc in corpus {
        let tokens = tokenize(doc, tenant);
        // Unique tokens per doc.
        let unique: std::collections::HashSet<String> = tokens.into_iter().collect();
        for t in unique {
            *df.entry(t).or_insert(0) += 1;
        }
    }
    df
}

// ── Neural embedding (scope-cut to cave-local-llm) ───────────────────────────

/// Compute a neural embedding for `text`.
///
/// # Scope-cut
/// This function is intentionally not implemented in-process: neural embedding
/// inference is delegated to `cave-local-llm` (parallel-track). The function
/// returns an empty vector as a stable, non-panicking no-op. Callers that need
/// real embeddings should call the cave-local-llm gRPC endpoint.
///
/// See: ADR-RUNTIME-PARITY-100-PCT-001 §5 parallel-track.
pub fn compute_embedding(_text: &str, _tenant_id: &TenantId) -> Vec<f64> {
    // Delegated to cave-local-llm (parallel-track scope-cut).
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn tenant() -> TenantId {
        TenantId::from_str("default").unwrap()
    }

    #[test]
    fn cosine_identical() {
        let v = vec![1.0, 2.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_zero_vector() {
        let zero = vec![0.0, 0.0];
        let v = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&zero, &v), 0.0);
    }

    #[test]
    fn tfidf_nonempty() {
        let corpus = vec!["hello world".to_string(), "world foo".to_string()];
        let v = tfidf_vector("hello", &corpus, &tenant());
        assert!(!v.is_empty());
    }

    #[test]
    fn compute_embedding_returns_empty_stub() {
        let v = compute_embedding("test", &tenant());
        // Scope-cut → returns empty Vec (not a panic).
        assert!(v.is_empty());
    }
}
