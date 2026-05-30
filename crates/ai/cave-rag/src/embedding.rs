// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embeddings.
//!
//! The pipeline talks to embedders through the [`Embeddings`] trait
//! (langchain's `Embeddings` / llama_index's `BaseEmbedding`). Two
//! deterministic, model-free backends ship here so the whole stack runs
//! offline with reproducible vectors:
//!
//! * [`HashingEmbedder`] — the feature-hashing trick (sklearn
//!   `HashingVectorizer`): tokens are hashed into a fixed-width bag and
//!   L2-normalized. No fitting required, fixed dimension.
//! * [`TfIdfEmbedder`] — classic TF-IDF over a vocabulary fitted from a
//!   corpus. Dimension equals the vocabulary size.
//!
//! Swap in a neural sentence-transformer by implementing [`Embeddings`].
//!
//! [`SemanticSplitter`] lives here too: it chunks text at the points where
//! the embedding of one sentence diverges sharply from the next (the
//! Kamradt / llama_index `SemanticSplitterNodeParser` heuristic).

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::math::{cosine_distance, l2_normalize, percentile};

/// Turn text into embedding vectors.
pub trait Embeddings {
    /// Embed a single query string.
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed many documents. Defaults to mapping [`embed_query`](Self::embed_query).
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_query(t)).collect()
    }

    /// Dimensionality of the produced vectors.
    fn dimension(&self) -> usize;
}

/// Lowercase alphanumeric word tokenizer shared by the embedders.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Feature-hashing bag-of-words embedder (no fitting needed).
#[derive(Debug, Clone)]
pub struct HashingEmbedder {
    dim: usize,
}

impl HashingEmbedder {
    /// Create with a fixed output dimension.
    pub fn new(dim: usize) -> Self {
        HashingEmbedder { dim: dim.max(1) }
    }

    fn bucket(&self, token: &str) -> usize {
        let mut h = Sha256::new();
        h.update(token.as_bytes());
        let digest = h.finalize();
        let n = u64::from_be_bytes(digest[0..8].try_into().unwrap());
        (n % self.dim as u64) as usize
    }
}

impl Embeddings for HashingEmbedder {
    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let mut v = vec![0.0f32; self.dim];
        for tok in tokenize(text) {
            let b = self.bucket(&tok);
            v[b] += 1.0;
        }
        l2_normalize(&mut v);
        Ok(v)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// TF-IDF embedder fitted against a corpus.
#[derive(Debug, Clone, Default)]
pub struct TfIdfEmbedder {
    /// term -> column index
    vocab: BTreeMap<String, usize>,
    /// idf weight per column index
    idf: Vec<f32>,
}

impl TfIdfEmbedder {
    /// New, unfitted embedder. Call [`fit`](Self::fit) before embedding.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fit the vocabulary and inverse-document-frequencies from `corpus`.
    /// Uses the smoothed idf `ln((1 + N) / (1 + df)) + 1`.
    pub fn fit(&mut self, corpus: &[String]) {
        let n_docs = corpus.len().max(1) as f32;
        let mut df: BTreeMap<String, usize> = BTreeMap::new();
        for doc in corpus {
            let mut seen = std::collections::BTreeSet::new();
            for tok in tokenize(doc) {
                if seen.insert(tok.clone()) {
                    *df.entry(tok).or_insert(0) += 1;
                }
            }
        }
        self.vocab.clear();
        self.idf = vec![0.0; df.len()];
        for (idx, (term, count)) in df.into_iter().enumerate() {
            let idf = ((1.0 + n_docs) / (1.0 + count as f32)).ln() + 1.0;
            self.idf[idx] = idf;
            self.vocab.insert(term, idx);
        }
    }

    /// Number of terms in the fitted vocabulary.
    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    /// Column index of a term, if present in the vocabulary.
    pub fn term_index(&self, term: &str) -> Option<usize> {
        self.vocab.get(&term.to_ascii_lowercase()).copied()
    }
}

impl Embeddings for TfIdfEmbedder {
    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let mut v = vec![0.0f32; self.vocab.len()];
        let toks = tokenize(text);
        let total = toks.len().max(1) as f32;
        let mut counts: BTreeMap<usize, f32> = BTreeMap::new();
        for tok in toks {
            if let Some(&idx) = self.vocab.get(&tok) {
                *counts.entry(idx).or_insert(0.0) += 1.0;
            }
        }
        for (idx, c) in counts {
            let tf = c / total;
            v[idx] = tf * self.idf[idx];
        }
        l2_normalize(&mut v);
        Ok(v)
    }

    fn dimension(&self) -> usize {
        self.vocab.len()
    }
}

/// Splits text at semantic breakpoints: sentences are embedded, the cosine
/// distance between consecutive sentences is computed, and a chunk boundary
/// is placed wherever that distance exceeds the configured percentile.
pub struct SemanticSplitter<'e> {
    embedder: &'e dyn Embeddings,
    breakpoint_percentile: f32,
}

impl<'e> SemanticSplitter<'e> {
    /// Build over an embedder (default breakpoint = 75th percentile).
    pub fn new(embedder: &'e dyn Embeddings) -> Self {
        SemanticSplitter {
            embedder,
            breakpoint_percentile: 75.0,
        }
    }

    /// Distances above this percentile (0..=100) start a new chunk.
    pub fn with_breakpoint_percentile(mut self, p: f32) -> Self {
        self.breakpoint_percentile = p;
        self
    }

    /// Split `text` into semantically coherent chunks.
    pub fn split_text(&self, text: &str) -> Vec<String> {
        let sentences = split_sentences(text);
        if sentences.len() <= 1 {
            return sentences;
        }
        let vecs: Vec<Vec<f32>> = sentences
            .iter()
            .map(|s| self.embedder.embed_query(s).unwrap_or_default())
            .collect();
        let distances: Vec<f32> = vecs
            .windows(2)
            .map(|w| cosine_distance(&w[0], &w[1]))
            .collect();
        let threshold = percentile(&distances, self.breakpoint_percentile);

        let mut chunks = Vec::new();
        let mut current = vec![sentences[0].clone()];
        for (i, dist) in distances.iter().enumerate() {
            // Break at-or-above the percentile, but never on a zero distance
            // (identical neighbours stay together even when the threshold
            // collapses to 0). Using `>=` keeps the heuristic robust to the
            // tied distances that sparse bag-of-words vectors produce.
            if *dist > 0.0 && *dist >= threshold {
                chunks.push(current.join(" "));
                current = Vec::new();
            }
            current.push(sentences[i + 1].clone());
        }
        if !current.is_empty() {
            chunks.push(current.join(" "));
        }
        chunks
    }
}

/// Naive sentence segmentation on `.`, `!`, `?` terminators.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut buf = String::new();
    for c in text.chars() {
        buf.push(c);
        if matches!(c, '.' | '!' | '?') {
            let s = buf.trim();
            if !s.is_empty() {
                sentences.push(s.to_string());
            }
            buf.clear();
        }
    }
    let tail = buf.trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    sentences
}
