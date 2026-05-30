// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The RAG generation chain.
//!
//! [`RagPipeline`] is the end-to-end orchestrator (langchain
//! `RetrievalQA` / llama_index `QueryEngine`): retrieve → optionally
//! rerank → generate → cite. It is generic over a [`Retriever`], an optional
//! [`Reranker`], and a [`Generator`].
//!
//! Two generators ship:
//!
//! * [`ExtractiveGenerator`] — model-free: returns the context sentences most
//!   relevant to the query. Fully offline and deterministic.
//! * [`LlmGenerator`] — "stuff" chain: packs numbered context into a prompt
//!   and delegates synthesis to an [`LlmClient`].

use crate::citation::{attribute_answer, build_citations, Attribution, Citation};
use crate::embedding::tokenize;
use crate::error::Result;
use crate::rerank::{LlmClient, Reranker};
use crate::retriever::Retriever;
use crate::vectorstore::ScoredDocument;
use std::collections::BTreeSet;

/// Synthesize an answer string from a query and retrieved context.
pub trait Generator {
    /// Produce an answer to `query` grounded in `context`.
    fn generate(&self, query: &str, context: &[ScoredDocument]) -> Result<String>;
}

/// The full result of a RAG query: the answer plus its provenance.
#[derive(Debug, Clone)]
pub struct RagAnswer {
    /// The generated answer text.
    pub answer: String,
    /// Numbered, source-tagged references to the retrieved context.
    pub citations: Vec<Citation>,
    /// Per-citation grounding strength for the answer.
    pub attributions: Vec<Attribution>,
}

/// Model-free extractive generator: returns the most query-relevant sentences
/// from the retrieved context, in context order.
#[derive(Debug, Clone, Default)]
pub struct ExtractiveGenerator {
    max_sentences: usize,
}

impl ExtractiveGenerator {
    /// Construct with a default of 3 extracted sentences.
    pub fn new() -> Self {
        ExtractiveGenerator { max_sentences: 3 }
    }

    /// Set how many sentences to extract.
    pub fn with_max_sentences(mut self, n: usize) -> Self {
        self.max_sentences = n;
        self
    }
}

impl Generator for ExtractiveGenerator {
    fn generate(&self, query: &str, context: &[ScoredDocument]) -> Result<String> {
        let qterms: BTreeSet<String> = tokenize(query).into_iter().collect();
        // (overlap, original_order, sentence)
        let mut scored: Vec<(f32, usize, String)> = Vec::new();
        let mut order = 0usize;
        for sd in context {
            for sentence in split_sentences(&sd.document.content) {
                let terms: BTreeSet<String> = tokenize(&sentence).into_iter().collect();
                let overlap = qterms.intersection(&terms).count() as f32;
                if overlap > 0.0 {
                    scored.push((overlap, order, sentence));
                }
                order += 1;
            }
        }
        // Best overlap first; ties keep context order.
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        scored.truncate(self.max_sentences);
        // Re-emit in context order for readability.
        scored.sort_by_key(|s| s.1);
        Ok(scored
            .into_iter()
            .map(|(_, _, s)| s)
            .collect::<Vec<_>>()
            .join(" "))
    }
}

/// "Stuff" generator: builds a numbered-context prompt and asks an LLM.
pub struct LlmGenerator<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> LlmGenerator<'a> {
    /// Build over an [`LlmClient`].
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        LlmGenerator { llm }
    }

    /// Render the stuffed-context prompt (public so callers can audit it).
    pub fn build_prompt(query: &str, context: &[ScoredDocument]) -> String {
        let mut p = String::from(
            "Answer the question using only the numbered context below. Cite \
             sources inline as [n]. If the context is insufficient, say so.\n\nContext:\n",
        );
        for (i, sd) in context.iter().enumerate() {
            let src = sd.document.metadata.source.as_deref().unwrap_or("unknown");
            p.push_str(&format!("[{}] ({}) {}\n", i + 1, src, sd.document.content));
        }
        p.push_str(&format!("\nQuestion: {query}\n\nAnswer:"));
        p
    }
}

impl Generator for LlmGenerator<'_> {
    fn generate(&self, query: &str, context: &[ScoredDocument]) -> Result<String> {
        self.llm.complete(&Self::build_prompt(query, context))
    }
}

/// End-to-end RAG orchestrator.
pub struct RagPipeline<'a> {
    retriever: &'a dyn Retriever,
    reranker: Option<&'a dyn Reranker>,
    generator: &'a dyn Generator,
    top_k: usize,
    /// When a reranker is present, retrieve this multiple of `top_k` first.
    fetch_multiplier: usize,
}

impl<'a> RagPipeline<'a> {
    /// Build a pipeline from a retriever and a generator.
    pub fn new(retriever: &'a dyn Retriever, generator: &'a dyn Generator) -> Self {
        RagPipeline {
            retriever,
            reranker: None,
            generator,
            top_k: 4,
            fetch_multiplier: 3,
        }
    }

    /// Insert a second-stage reranker.
    pub fn with_reranker(mut self, reranker: &'a dyn Reranker) -> Self {
        self.reranker = Some(reranker);
        self
    }

    /// Number of context documents fed to the generator.
    pub fn with_top_k(mut self, k: usize) -> Self {
        self.top_k = k;
        self
    }

    /// Run retrieve → (rerank) → generate → cite for `query`.
    pub fn query(&self, query: &str) -> Result<RagAnswer> {
        let fetch = if self.reranker.is_some() {
            self.top_k * self.fetch_multiplier
        } else {
            self.top_k
        };
        let mut hits = self.retriever.retrieve(query, fetch)?;
        if let Some(rr) = self.reranker {
            hits = rr.rerank(query, hits, self.top_k)?;
        } else {
            hits.truncate(self.top_k);
        }
        let answer = self.generator.generate(query, &hits)?;
        let citations = build_citations(&hits);
        let attributions = attribute_answer(&answer, &hits);
        Ok(RagAnswer {
            answer,
            citations,
            attributions,
        })
    }
}

/// Naive sentence segmentation on `.`, `!`, `?` (shared with the splitter).
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in text.chars() {
        buf.push(c);
        if matches!(c, '.' | '!' | '?') {
            let s = buf.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
            buf.clear();
        }
    }
    let tail = buf.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}
