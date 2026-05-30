// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Re-rankers.
//!
//! Re-ranking is the second-stage precision pass over a first-stage
//! retriever's recall set (the langchain `ContextualCompressionRetriever` /
//! llama_index `node_postprocessors` pattern). Two rerankers ship:
//!
//! * [`LexicalCrossEncoder`] — a deterministic cross-encoder surrogate that
//!   scores the *(query, document)* pair jointly on query-term coverage and
//!   Jaccard overlap. Swap in a real neural cross-encoder by implementing
//!   [`Reranker`].
//! * [`LlmJudgeReranker`] — LLM-as-judge: prompt an [`LlmClient`] to score
//!   each document's relevance and re-order by the parsed score.

use std::collections::BTreeSet;

use crate::embedding::tokenize;
use crate::error::{RagError, Result};
use crate::vectorstore::ScoredDocument;

/// Re-score and re-order a candidate set against the query.
pub trait Reranker {
    /// Re-rank `docs` for `query` and return the top `top_n`, best first.
    fn rerank(
        &self,
        query: &str,
        docs: Vec<ScoredDocument>,
        top_n: usize,
    ) -> Result<Vec<ScoredDocument>>;
}

/// Deterministic cross-encoder surrogate scoring `(query, doc)` jointly.
#[derive(Debug, Clone, Default)]
pub struct LexicalCrossEncoder;

impl LexicalCrossEncoder {
    /// Construct a cross-encoder reranker.
    pub fn new() -> Self {
        LexicalCrossEncoder
    }

    /// Joint relevance score: query-term coverage weighted up by Jaccard
    /// overlap, so a document answering more of the query ranks higher.
    fn pair_score(query_terms: &BTreeSet<String>, doc: &str) -> f32 {
        if query_terms.is_empty() {
            return 0.0;
        }
        let doc_terms: BTreeSet<String> = tokenize(doc).into_iter().collect();
        let inter = query_terms.intersection(&doc_terms).count() as f32;
        let union = query_terms.union(&doc_terms).count() as f32;
        let coverage = inter / query_terms.len() as f32;
        let jaccard = if union == 0.0 { 0.0 } else { inter / union };
        coverage * (1.0 + jaccard)
    }
}

impl Reranker for LexicalCrossEncoder {
    fn rerank(
        &self,
        query: &str,
        docs: Vec<ScoredDocument>,
        top_n: usize,
    ) -> Result<Vec<ScoredDocument>> {
        let qterms: BTreeSet<String> = tokenize(query).into_iter().collect();
        let mut scored: Vec<ScoredDocument> = docs
            .into_iter()
            .map(|mut d| {
                d.score = Self::pair_score(&qterms, &d.document.content);
                d
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_n);
        Ok(scored)
    }
}

/// A minimal text-completion seam to a large language model.
///
/// Implement this over cave-llm-gateway / cave-local-llm to drive the
/// [`LlmJudgeReranker`] and the generation chain with a real model.
pub trait LlmClient {
    /// Complete `prompt` and return the model's text response.
    fn complete(&self, prompt: &str) -> Result<String>;
}

/// LLM-as-judge reranker.
pub struct LlmJudgeReranker<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> LlmJudgeReranker<'a> {
    /// Build over an [`LlmClient`].
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        LlmJudgeReranker { llm }
    }

    fn prompt(query: &str, doc: &str) -> String {
        format!(
            "You are scoring search results. On a scale of 0 to 10, how \
             relevant is the following document to the query? Reply with only \
             the number.\n\nQuery: {query}\n\nDocument:\n{doc}\n\nScore:"
        )
    }
}

impl Reranker for LlmJudgeReranker<'_> {
    fn rerank(
        &self,
        query: &str,
        docs: Vec<ScoredDocument>,
        top_n: usize,
    ) -> Result<Vec<ScoredDocument>> {
        let mut scored: Vec<ScoredDocument> = Vec::with_capacity(docs.len());
        for mut d in docs {
            let prompt = Self::prompt(query, &d.document.content);
            let reply = self.llm.complete(&prompt)?;
            d.score = parse_first_number(&reply)
                .ok_or_else(|| RagError::Rerank(format!("no score in LLM reply: {reply:?}")))?;
            scored.push(d);
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_n);
        Ok(scored)
    }
}

/// Pull the first (possibly decimal) number out of free text.
fn parse_first_number(s: &str) -> Option<f32> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || (bytes[i] == b'.' && i + 1 < bytes.len()) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            return s[start..i].parse::<f32>().ok();
        }
        i += 1;
    }
    None
}
