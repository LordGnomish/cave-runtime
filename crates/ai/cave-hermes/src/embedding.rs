// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Real text embedder — TF-IDF projected into a fixed-width vector.
//!
//! The MVP shipped a `HashEmbedder` that exercised the cosine pipeline but
//! produced unstable rankings under synonym/topic drift. `TfIdfEmbedder`
//! upgrades the recall path to a measurable signal: a corpus-aware
//! inverse-document-frequency table weights bucket projections, so common
//! tokens contribute less than rare ones. Still dependency-free; still
//! deterministic; still cosine-rankable. The hash trick keeps the vector
//! width bounded.

use crate::recall::Embedder;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const DEFAULT_DIM: usize = 256;
const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "in", "on", "for", "to", "with", "from", "is",
    "are", "was", "were", "be", "been", "it", "this", "that", "as", "at", "by",
];

/// TF-IDF-backed embedder. Build once from a representative corpus via
/// [`TfIdfEmbedder::fit`]; subsequent calls to `embed` use the learned
/// inverse-document-frequency table to weight bucket projections.
///
/// Each call to `embed`:
///   * tokenises identically to `HashEmbedder` (lowercase, alpha-only, stop
///     words removed),
///   * computes term-frequency,
///   * weights each term by `idf[term]` (defaulting to `1.0` for OOV terms),
///   * projects into a `dim`-wide vector via SHA-256 bucketing with random-
///     sign feature hashing,
///   * L2-normalises.
pub struct TfIdfEmbedder {
    dim: usize,
    idf: HashMap<String, f32>,
}

impl Default for TfIdfEmbedder {
    fn default() -> Self {
        Self::new(DEFAULT_DIM)
    }
}

impl TfIdfEmbedder {
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "TfIdfEmbedder dim must be > 0");
        Self {
            dim,
            idf: HashMap::new(),
        }
    }

    /// Learn IDF weights from a `corpus`. Documents are tokenised the same
    /// way as `embed`. `idf[t] = ln(1 + N / (1 + df[t]))`.
    pub fn fit(&mut self, corpus: &[&str]) {
        let n = corpus.len() as f32;
        let mut df: HashMap<String, u32> = HashMap::new();
        for doc in corpus {
            let mut seen: HashMap<String, ()> = HashMap::new();
            for tok in tokenise(doc) {
                seen.entry(tok).or_insert(());
            }
            for (tok, _) in seen {
                *df.entry(tok).or_insert(0) += 1;
            }
        }
        for (tok, count) in df {
            let v = ((1.0 + n) / (1.0 + count as f32)).ln().max(0.0) + 1.0;
            self.idf.insert(tok, v);
        }
    }

    /// Number of distinct terms learned during fit.
    pub fn vocab_size(&self) -> usize {
        self.idf.len()
    }

    /// IDF weight for a single term (1.0 for OOV).
    pub fn idf_for(&self, term: &str) -> f32 {
        self.idf.get(term).copied().unwrap_or(1.0)
    }
}

impl Embedder for TfIdfEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        let tokens = tokenise(text);
        if tokens.is_empty() {
            return v;
        }
        let mut tf: HashMap<&str, u32> = HashMap::new();
        for t in &tokens {
            *tf.entry(t.as_str()).or_insert(0) += 1;
        }
        let total = tokens.len() as f32;
        for (tok, count) in tf {
            let tf_val = count as f32 / total;
            let idf = self.idf_for(tok);
            let weight = tf_val * idf;
            let mut h = Sha256::new();
            h.update(tok.as_bytes());
            let digest = h.finalize();
            let bucket =
                (u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as usize) % self.dim;
            let sign = if (digest[4] & 1) == 0 { 1.0f32 } else { -1.0f32 };
            v[bucket] += sign * weight;
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

fn tokenise(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recall::cosine_similarity;

    #[test]
    fn fit_learns_vocab_from_corpus() {
        let mut e = TfIdfEmbedder::new(64);
        e.fit(&["apple banana", "banana cherry", "cherry date"]);
        assert!(e.vocab_size() >= 4);
        assert!(e.idf_for("apple") > e.idf_for("banana"));
    }

    #[test]
    fn embed_empty_text_returns_zero_vector() {
        let e = TfIdfEmbedder::new(16);
        let v = e.embed("");
        assert_eq!(v.len(), 16);
        assert!(v.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn embed_is_deterministic() {
        let e = TfIdfEmbedder::new(32);
        let a = e.embed("hello world");
        let b = e.embed("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn embed_is_l2_normalised_for_nonempty_input() {
        let e = TfIdfEmbedder::new(64);
        let v = e.embed("rust embeddings are great");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4 || norm == 0.0);
    }

    #[test]
    fn cosine_similar_for_overlapping_text() {
        let mut e = TfIdfEmbedder::new(128);
        e.fit(&[
            "rust is a systems programming language",
            "rust embeds cleanly in higher-level apps",
            "ocaml is a different functional language",
        ]);
        let a = e.embed("rust systems programming");
        let b = e.embed("rust programming language");
        let c = e.embed("ocaml functional language");
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ac = cosine_similarity(&a, &c);
        assert!(
            sim_ab > sim_ac,
            "rust-related queries should be more similar than rust↔ocaml ({} vs {})",
            sim_ab,
            sim_ac
        );
    }

    #[test]
    fn idf_for_oov_is_one() {
        let e = TfIdfEmbedder::new(16);
        assert_eq!(e.idf_for("nonexistent_token_xyz"), 1.0);
    }

    #[test]
    fn idf_for_rare_term_is_higher_than_common() {
        let mut e = TfIdfEmbedder::new(64);
        e.fit(&["x x x x", "x y", "x z"]);
        let idf_x = e.idf_for("x");
        let idf_z = e.idf_for("z");
        assert!(idf_z > idf_x);
    }

    #[test]
    fn stop_words_dropped() {
        let toks = tokenise("the rust and the embedder");
        assert!(!toks.iter().any(|t| t == "the" || t == "and"));
        assert!(toks.iter().any(|t| t == "rust"));
    }

    #[test]
    fn corpus_fit_then_embed_round_trip() {
        let mut e = TfIdfEmbedder::new(64);
        let docs = ["foo bar baz", "foo bar"];
        e.fit(&docs);
        let v = e.embed(docs[0]);
        assert!(v.iter().any(|x| *x != 0.0));
    }

    #[test]
    fn dim_is_preserved() {
        let e = TfIdfEmbedder::new(200);
        assert_eq!(e.dim(), 200);
        assert_eq!(e.embed("x").len(), 200);
    }
}
