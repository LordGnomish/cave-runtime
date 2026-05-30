// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vector store.
//!
//! The [`VectorStore`] trait is the langchain/llama_index storage seam.
//! [`InMemoryVectorStore`] is the reference implementation: an exhaustive
//! cosine-similarity index with JSON persistence — enough for tens of
//! thousands of chunks, and a drop-in for a FAISS/pgvector backend behind the
//! same trait.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::embedding::Embeddings;
use crate::error::{RagError, Result};
use crate::math::cosine_similarity;

/// A retrieved document with its similarity score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredDocument {
    /// The matched document.
    pub document: Document,
    /// Similarity score (higher = more relevant).
    pub score: f32,
}

/// A persistent association of a document with its embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    id: String,
    document: Document,
    vector: Vec<f32>,
}

/// Storage + similarity-search seam.
pub trait VectorStore {
    /// Embed and store documents; returns their ids.
    fn add(&mut self, docs: Vec<Document>, embeddings: &dyn Embeddings) -> Result<Vec<String>>;

    /// Store documents with pre-computed vectors; returns their ids.
    fn add_with_vectors(&mut self, items: Vec<(Document, Vec<f32>)>) -> Result<Vec<String>>;

    /// Top-`k` documents by cosine similarity to `query_vec`, sorted
    /// descending by score.
    fn similarity_search(&self, query_vec: &[f32], k: usize) -> Vec<ScoredDocument>;

    /// Number of stored documents.
    fn len(&self) -> usize;

    /// True when the store holds no documents.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Exhaustive in-memory cosine index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InMemoryVectorStore {
    entries: Vec<Entry>,
}

impl InMemoryVectorStore {
    /// New empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The ids of all stored documents, in insertion order.
    pub fn ids(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.id.clone()).collect()
    }

    /// Like [`similarity_search`](VectorStore::similarity_search) but also
    /// returns each hit's stored vector — needed by the MMR retriever, which
    /// must compare candidates against one another, not just the query.
    pub fn similarity_search_with_vectors(
        &self,
        query_vec: &[f32],
        k: usize,
    ) -> Vec<(ScoredDocument, Vec<f32>)> {
        let mut scored: Vec<(ScoredDocument, Vec<f32>)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    ScoredDocument {
                        document: e.document.clone(),
                        score: cosine_similarity(query_vec, &e.vector),
                    },
                    e.vector.clone(),
                )
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        scored
    }

    /// Serialize the whole index to JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| RagError::VectorStore(e.to_string()))
    }

    /// Reconstruct an index from JSON produced by [`to_json`](Self::to_json).
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| RagError::VectorStore(e.to_string()))
    }
}

impl VectorStore for InMemoryVectorStore {
    fn add(&mut self, docs: Vec<Document>, embeddings: &dyn Embeddings) -> Result<Vec<String>> {
        let texts: Vec<String> = docs.iter().map(|d| d.content.clone()).collect();
        let vectors = embeddings.embed_documents(&texts)?;
        self.add_with_vectors(docs.into_iter().zip(vectors).collect())
    }

    fn add_with_vectors(&mut self, items: Vec<(Document, Vec<f32>)>) -> Result<Vec<String>> {
        let mut ids = Vec::with_capacity(items.len());
        for (document, vector) in items {
            let id = document.id();
            ids.push(id.clone());
            self.entries.push(Entry {
                id,
                document,
                vector,
            });
        }
        Ok(ids)
    }

    fn similarity_search(&self, query_vec: &[f32], k: usize) -> Vec<ScoredDocument> {
        let mut scored: Vec<ScoredDocument> = self
            .entries
            .iter()
            .map(|e| ScoredDocument {
                document: e.document.clone(),
                score: cosine_similarity(query_vec, &e.vector),
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        scored
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}
