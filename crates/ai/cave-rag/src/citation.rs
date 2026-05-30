// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Citation tracking and answer attribution.
//!
//! A trustworthy RAG answer must say *where it came from*. [`build_citations`]
//! turns the retrieved context into numbered, source-tagged [`Citation`]s, and
//! [`attribute_answer`] measures how strongly each citation supports the
//! generated answer (token-overlap grounding) so callers can surface
//! "[1][3]"-style references and flag unsupported claims.

use serde::{Deserialize, Serialize};

use crate::embedding::tokenize;
use crate::vectorstore::ScoredDocument;
use std::collections::BTreeSet;

/// A numbered, source-tagged reference to a retrieved context document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Citation {
    /// Stable 0-based index used for inline `[n]` references.
    pub index: usize,
    /// Origin of the cited document, if known.
    pub source: Option<String>,
    /// A short snippet of the cited content.
    pub snippet: String,
    /// The retriever/reranker relevance score of the cited document.
    pub score: f32,
}

/// How strongly a given citation grounds the generated answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attribution {
    /// Index into the citations list.
    pub citation_index: usize,
    /// Support in `[0, 1]`: fraction of the answer's terms found in the source.
    pub support: f32,
}

/// Maximum snippet length kept in a [`Citation`].
const SNIPPET_LEN: usize = 280;

/// Build one [`Citation`] per retrieved context document.
pub fn build_citations(context: &[ScoredDocument]) -> Vec<Citation> {
    context
        .iter()
        .enumerate()
        .map(|(i, sd)| {
            let content = &sd.document.content;
            let snippet = if content.chars().count() > SNIPPET_LEN {
                let truncated: String = content.chars().take(SNIPPET_LEN).collect();
                format!("{truncated}…")
            } else {
                content.clone()
            };
            Citation {
                index: i,
                source: sd.document.metadata.source.clone(),
                snippet,
                score: sd.score,
            }
        })
        .collect()
}

/// Attribute `answer` to each context document by token overlap.
///
/// Support is the fraction of the answer's distinct terms that also appear in
/// the source — a cheap, deterministic grounding signal that lets callers
/// rank supporting sources and detect hallucinated (unsupported) answers.
pub fn attribute_answer(answer: &str, context: &[ScoredDocument]) -> Vec<Attribution> {
    let answer_terms: BTreeSet<String> = tokenize(answer).into_iter().collect();
    context
        .iter()
        .enumerate()
        .map(|(i, sd)| {
            let support = if answer_terms.is_empty() {
                0.0
            } else {
                let doc_terms: BTreeSet<String> =
                    tokenize(&sd.document.content).into_iter().collect();
                let overlap = answer_terms.intersection(&doc_terms).count() as f32;
                overlap / answer_terms.len() as f32
            };
            Attribution {
                citation_index: i,
                support,
            }
        })
        .collect()
}
