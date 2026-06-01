// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Reranking — Cohere/infinity `POST /rerank` contract.
//!
//! Upstream: infinity's `/rerank` endpoint + `CrossEncoder`. A reranker scores
//! each (query, document) pair and returns the documents sorted by relevance,
//! optionally truncated to `top_n` and echoing the document text. The concrete
//! cross-encoder model is a scope-cut; we ship a deterministic cosine reranker
//! built on the reference embedder so the contract is fully exercisable.

use crate::backend::{self, HashingEmbedder};
use crate::error::{EmbedError, EmbedResult};
use crate::pooling::PoolingStrategy;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// `POST /rerank` request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RerankRequest {
    /// Reranker model id.
    pub model: String,
    /// Query to rank documents against.
    pub query: String,
    /// Candidate documents.
    pub documents: Vec<String>,
    /// Return only the top-N results (after sorting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<usize>,
    /// Echo the document text in each result.
    #[serde(default)]
    pub return_documents: bool,
}

/// One reranked result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RerankResult {
    /// Index into the original `documents` array.
    pub index: usize,
    /// Relevance score (higher is more relevant).
    pub relevance_score: f32,
    /// The document text, present iff `return_documents` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

/// `POST /rerank` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResponse {
    /// Model id.
    pub model: String,
    /// Results, sorted by descending relevance.
    pub results: Vec<RerankResult>,
}

/// A reranker scores a (query, document) pair.
#[async_trait]
pub trait Reranker: Send + Sync {
    /// Relevance score; higher = more relevant.
    async fn score(&self, query: &str, document: &str) -> EmbedResult<f32>;
}

/// Reference reranker: cosine similarity of mean-pooled reference embeddings.
pub struct CosineReranker {
    embedder: HashingEmbedder,
}

impl Default for CosineReranker {
    fn default() -> Self {
        Self {
            embedder: HashingEmbedder::new("rerank-ref", 256),
        }
    }
}

#[async_trait]
impl Reranker for CosineReranker {
    async fn score(&self, query: &str, document: &str) -> EmbedResult<f32> {
        let q = backend::embed_with(&self.embedder, PoolingStrategy::Mean, true, None, query).await?;
        let d =
            backend::embed_with(&self.embedder, PoolingStrategy::Mean, true, None, document).await?;
        Ok(q.iter().zip(&d).map(|(a, b)| a * b).sum())
    }
}

/// Service wrapping a reranker.
pub struct RerankService<R: Reranker = CosineReranker> {
    reranker: R,
}

impl Default for RerankService {
    fn default() -> Self {
        Self {
            reranker: CosineReranker::default(),
        }
    }
}

impl<R: Reranker> RerankService<R> {
    /// Build from an explicit reranker.
    pub fn new(reranker: R) -> Self {
        Self { reranker }
    }

    /// Run a rerank request.
    pub async fn rerank(&self, req: &RerankRequest) -> EmbedResult<RerankResponse> {
        if req.documents.is_empty() {
            return Err(EmbedError::EmptyInput);
        }
        let mut results: Vec<RerankResult> = Vec::with_capacity(req.documents.len());
        for (index, doc) in req.documents.iter().enumerate() {
            let score = self.reranker.score(&req.query, doc).await?;
            results.push(RerankResult {
                index,
                relevance_score: score,
                document: req.return_documents.then(|| doc.clone()),
            });
        }
        // Sort by descending relevance; ties broken by original index for
        // determinism.
        results.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.index.cmp(&b.index))
        });
        if let Some(n) = req.top_n {
            results.truncate(n);
        }
        Ok(RerankResponse {
            model: req.model.clone(),
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(docs: Vec<&str>) -> RerankRequest {
        RerankRequest {
            model: "rerank-ref".into(),
            query: "red blue green".into(),
            documents: docs.into_iter().map(String::from).collect(),
            top_n: None,
            return_documents: false,
        }
    }

    #[tokio::test]
    async fn empty_documents_errors() {
        let svc = RerankService::default();
        assert!(matches!(
            svc.rerank(&req(vec![])).await,
            Err(EmbedError::EmptyInput)
        ));
    }

    #[tokio::test]
    async fn results_sorted_descending() {
        let svc = RerankService::default();
        let r = svc
            .rerank(&req(vec!["totally unrelated terms", "red blue green", "red blue"]))
            .await
            .unwrap();
        for w in r.results.windows(2) {
            assert!(w[0].relevance_score >= w[1].relevance_score);
        }
    }

    #[tokio::test]
    async fn identical_to_query_ranks_first() {
        let svc = RerankService::default();
        let r = svc
            .rerank(&req(vec!["apple orange", "red blue green", "nothing here"]))
            .await
            .unwrap();
        // The document identical to the query is original index 1.
        assert_eq!(r.results[0].index, 1);
    }

    #[tokio::test]
    async fn top_n_limits_results() {
        let svc = RerankService::default();
        let mut request = req(vec!["a b", "red", "green blue", "x y z"]);
        request.top_n = Some(2);
        let r = svc.rerank(&request).await.unwrap();
        assert_eq!(r.results.len(), 2);
    }

    #[tokio::test]
    async fn return_documents_attaches_text() {
        let svc = RerankService::default();
        let mut request = req(vec!["red blue green", "apple"]);
        request.return_documents = true;
        let r = svc.rerank(&request).await.unwrap();
        assert!(r.results.iter().all(|x| x.document.is_some()));
        // top result's attached text matches its original-index document.
        let top = &r.results[0];
        assert_eq!(top.document.as_deref(), Some(request.documents[top.index].as_str()));
    }

    #[tokio::test]
    async fn without_return_documents_text_absent() {
        let svc = RerankService::default();
        let r = svc.rerank(&req(vec!["red", "blue"])).await.unwrap();
        assert!(r.results.iter().all(|x| x.document.is_none()));
    }
}
