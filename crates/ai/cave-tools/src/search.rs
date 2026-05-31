// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Semantic tool search.
//!
//! When a registry holds dozens of tools, sending all of them to the model
//! wastes context. [`ToolSearchIndex`] ranks tools against a natural-language
//! query so callers can surface only the top-k relevant ones.
//!
//! The ranking is classic TF-IDF with cosine similarity over a bag of words
//! drawn from each tool's `name`, `description`, and `toolset`. It is pure
//! Rust with no model dependency — deterministic and dependency-free — and
//! is hot-swappable for an embedding-backed index once `cave-search` exposes
//! a vector store (tracked as a manifest scope-cut).

use std::collections::BTreeMap;

use crate::tool::{ToolRegistry, ToolSpec};

/// A ranked search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub name: String,
    pub score: f64,
}

/// An in-memory TF-IDF index over a set of tools.
pub struct ToolSearchIndex {
    /// Per-document (tool) term-frequency maps, plus the tool name.
    docs: Vec<(String, BTreeMap<String, f64>)>,
    /// Inverse document frequency per term.
    idf: BTreeMap<String, f64>,
    /// Cached L2 norm of each document's tf-idf vector.
    doc_norms: Vec<f64>,
}

impl ToolSearchIndex {
    /// Build an index from tool descriptors.
    pub fn build(specs: &[ToolSpec]) -> Self {
        let n = specs.len().max(1) as f64;

        // Term frequencies per document.
        let docs: Vec<(String, BTreeMap<String, f64>)> = specs
            .iter()
            .map(|s| {
                let text = format!("{} {} {}", s.name, s.description, s.toolset);
                (s.name.clone(), term_freqs(&text))
            })
            .collect();

        // Document frequency per term.
        let mut df: BTreeMap<String, usize> = BTreeMap::new();
        for (_, tf) in &docs {
            for term in tf.keys() {
                *df.entry(term.clone()).or_insert(0) += 1;
            }
        }
        // Smoothed idf: ln((1 + N) / (1 + df)) + 1 — always positive, so a
        // term present in every document still contributes a little.
        let idf: BTreeMap<String, f64> = df
            .into_iter()
            .map(|(term, d)| (term, ((1.0 + n) / (1.0 + d as f64)).ln() + 1.0))
            .collect();

        // Precompute document vector norms.
        let doc_norms = docs
            .iter()
            .map(|(_, tf)| {
                tf.iter()
                    .map(|(t, &f)| {
                        let w = f * idf.get(t).copied().unwrap_or(0.0);
                        w * w
                    })
                    .sum::<f64>()
                    .sqrt()
            })
            .collect();

        Self {
            docs,
            idf,
            doc_norms,
        }
    }

    /// Build an index over every tool in a registry.
    pub fn from_registry(reg: &ToolRegistry) -> Self {
        Self::build(&reg.list_specs())
    }

    /// Return the top-`k` tools most relevant to `query`, highest score
    /// first (ties broken by name). Tools with zero overlap are omitted.
    pub fn search(&self, query: &str, k: usize) -> Vec<SearchHit> {
        let q_tf = term_freqs(query);
        if q_tf.is_empty() {
            return Vec::new();
        }
        // Query tf-idf vector + its norm.
        let q_vec: BTreeMap<&str, f64> = q_tf
            .iter()
            .map(|(t, &f)| (t.as_str(), f * self.idf.get(t).copied().unwrap_or(0.0)))
            .collect();
        let q_norm: f64 = q_vec.values().map(|w| w * w).sum::<f64>().sqrt();
        if q_norm == 0.0 {
            return Vec::new();
        }

        let mut hits: Vec<SearchHit> = Vec::new();
        for (i, (name, tf)) in self.docs.iter().enumerate() {
            let dot: f64 = q_vec
                .iter()
                .map(|(&t, &qw)| {
                    let dw = tf.get(t).copied().unwrap_or(0.0)
                        * self.idf.get(t).copied().unwrap_or(0.0);
                    qw * dw
                })
                .sum();
            let denom = q_norm * self.doc_norms[i];
            let score = if denom > 0.0 { dot / denom } else { 0.0 };
            if score > 0.0 {
                hits.push(SearchHit {
                    name: name.clone(),
                    score,
                });
            }
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });
        hits.truncate(k);
        hits
    }
}

/// Tokenize into lowercase alphanumeric terms and count frequencies.
fn term_freqs(text: &str) -> BTreeMap<String, f64> {
    let mut tf: BTreeMap<String, f64> = BTreeMap::new();
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        *tf.entry(token.to_lowercase()).or_insert(0.0) += 1.0;
    }
    tf
}
