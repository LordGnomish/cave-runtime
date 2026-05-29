// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vector embeddings + semantic similarity for search.
//!
//! Provides:
//! - Pure vector math: cosine similarity, dot product, Euclidean distance
//! - TF-IDF dense vector representation (in-crate, no external model)
//! - compute_embedding: returns empty Vec — scope-cut to cave-local-llm
//!
//! Manticore equivalent: src/sphinxexpr.cpp KNN/FAISS vector search surface.
//! upstream: manticoresoftware/manticoresearch v25.8.2 — src/sphinxexpr.cpp (KNN)
//!
//! Note: compute_embedding with a real neural model is a scope-cut →
//! cave-local-llm (parallel-track). TF-IDF vectors serve as the in-process
//! semantic similarity backend for MVP.

use crate::analyzer::tokenize;
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
/// Result is clamped to [-1, 1] to handle floating-point drift.
pub fn cosine_similarity(v1: &[f64], v2: &[f64]) -> f64 {
    if v1.is_empty() || v2.is_empty() { return 0.0; }
    assert_eq!(v1.len(), v2.len(), "cosine_similarity: vectors must have equal length");

    let dot = dot_product(v1, v2);
    let norm1: f64 = v1.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm2: f64 = v2.iter().map(|x| x * x).sum::<f64>().sqrt();

    if norm1 == 0.0 || norm2 == 0.0 { return 0.0; }
    (dot / (norm1 * norm2)).clamp(-1.0, 1.0)
}

// ── TF-IDF dense vector representation ───────────────────────────────────────

/// Build a TF-IDF feature vector for `text` using the provided `corpus`.
///
/// The vocabulary is derived from all unique tokens across all corpus documents.
/// The vector is indexed by sorted vocabulary entries (deterministic order).
pub fn tfidf_vector(text: &str, corpus: &[String], tenant: &TenantId) -> Vec<f64> {
    let vocab = build_vocab(corpus, tenant);
    if vocab.is_empty() { return Vec::new(); }

    let n_docs = corpus.len() as f64;
    let text_tokens = tokenize(text, tenant);
    let text_tf = term_freq_map(&text_tokens);
    let df_map = corpus_df_map(corpus, tenant);

    let vocab_terms: Vec<&str> = vocab.keys().map(|s| s.as_str()).collect();

    vocab_terms
        .iter()
        .map(|term| {
            let tf = *text_tf.get(*term).unwrap_or(&0) as f64;
            if tf == 0.0 { return 0.0; }
            let doc_len = text_tokens.len().max(1) as f64;
            let tf_norm = tf / doc_len;
            let df = *df_map.get(*term).unwrap_or(&0) as f64;
            // IDF smooth variant (Lucene-compatible): ln(N / (1 + df)) + 1
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

/// Compute document-frequency map (how many docs contain each term).
fn corpus_df_map(corpus: &[String], tenant: &TenantId) -> HashMap<String, u32> {
    let mut df: HashMap<String, u32> = HashMap::new();
    for doc in corpus {
        let tokens = tokenize(doc, tenant);
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
/// Neural embedding inference is delegated to `cave-local-llm` (parallel-track).
/// This function returns an empty Vec as a stable, non-panicking no-op.
/// Callers that need real embeddings should call the cave-local-llm gRPC endpoint.
///
/// See: ADR-RUNTIME-PARITY-100-PCT-001 §5 parallel-track.
pub fn compute_embedding(_text: &str, _tenant_id: &TenantId) -> Vec<f64> {
    Vec::new()
}
