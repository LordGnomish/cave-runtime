// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cross-encoder reranker + `/v1/rerank` API.
//!
//! A reranker scores `(query, document)` pairs jointly and returns the
//! documents sorted by relevance — the second-stage refinement after a
//! bi-encoder recall. A concrete cross-encoder (e.g. a BGE-reranker checkpoint)
//! plugs in through [`CrossEncoder`]; [`LexicalCrossEncoder`] is the reference
//! scorer: the fraction of the query's unique terms the document covers, a
//! genuine (if simple) relevance signal that needs no model weights.

use crate::tokenize::tokenize;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Scores the relevance of a document to a query (higher = more relevant).
pub trait CrossEncoder: Send + Sync {
    /// Relevance score for `(query, document)`.
    fn score(&self, query: &str, document: &str) -> f32;
}

/// Reference cross-encoder: the fraction of unique query terms present in the
/// document (query coverage), in `[0, 1]`.
#[derive(Debug, Default, Clone)]
pub struct LexicalCrossEncoder;

impl LexicalCrossEncoder {
    /// Construct the reference scorer.
    pub fn new() -> Self {
        LexicalCrossEncoder
    }
}

impl CrossEncoder for LexicalCrossEncoder {
    fn score(&self, query: &str, document: &str) -> f32 {
        let q: HashSet<String> = tokenize(query).into_iter().collect();
        if q.is_empty() {
            return 0.0;
        }
        let d: HashSet<String> = tokenize(document).into_iter().collect();
        let shared = q.iter().filter(|t| d.contains(*t)).count();
        shared as f32 / q.len() as f32
    }
}

/// One reranked result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResult {
    /// Position of this document in the original request list.
    pub index: usize,
    /// Relevance score from the cross-encoder.
    pub relevance_score: f32,
    /// The document text, present only when `return_documents` was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

/// A `/v1/rerank` request (Cohere/infinity-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    /// Reranker model id.
    pub model: String,
    /// The search query.
    pub query: String,
    /// Candidate documents to rank.
    pub documents: Vec<String>,
    /// Return at most this many results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<usize>,
    /// Echo the document text back in each result.
    #[serde(default)]
    pub return_documents: bool,
}

/// A `/v1/rerank` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResponse {
    /// Reranker model id that served the request.
    pub model: String,
    /// Results sorted by descending relevance.
    pub results: Vec<RerankResult>,
}

/// Ranks documents against a query using a pluggable [`CrossEncoder`].
pub struct Reranker {
    encoder: Box<dyn CrossEncoder>,
}

impl Reranker {
    /// Build a reranker around a cross-encoder.
    pub fn new(encoder: Box<dyn CrossEncoder>) -> Self {
        Reranker { encoder }
    }

    /// Score and sort `documents` by relevance to `query`, keeping at most
    /// `top_n` results. Results carry the original document index.
    pub fn rerank(
        &self,
        query: &str,
        documents: &[String],
        top_n: Option<usize>,
    ) -> Vec<RerankResult> {
        let mut scored: Vec<RerankResult> = documents
            .iter()
            .enumerate()
            .map(|(index, doc)| RerankResult {
                index,
                relevance_score: self.encoder.score(query, doc),
                document: None,
            })
            .collect();
        // Sort by descending score; ties keep original order (stable + index tiebreak).
        scored.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.index.cmp(&b.index))
        });
        if let Some(n) = top_n {
            scored.truncate(n);
        }
        scored
    }

    /// Serve a `/v1/rerank` request, optionally echoing document text.
    pub fn serve(&self, req: &RerankRequest) -> RerankResponse {
        let mut results = self.rerank(&req.query, &req.documents, req.top_n);
        if req.return_documents {
            for r in &mut results {
                r.document = req.documents.get(r.index).cloned();
            }
        }
        RerankResponse {
            model: req.model.clone(),
            results,
        }
    }
}
