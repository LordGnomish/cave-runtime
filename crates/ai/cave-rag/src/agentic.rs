// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Agentic RAG: plan → search → evaluate → iterate → synthesize.
//!
//! Unlike a single-shot pipeline, [`AgenticRag`] decomposes the question into
//! sub-queries (the planning step), retrieves for each, then loops: it asks
//! the LLM whether the accumulated context is *sufficient* and, if not,
//! requests a follow-up query and retrieves again — up to `max_iterations` —
//! before synthesising a final answer. Every move is recorded in an
//! [`AgentTrace`] for inspection.
//!
//! It is generic over a [`Retriever`] and an [`LlmClient`].

use crate::document::Document;
use crate::error::Result;
use crate::rerank::LlmClient;
use crate::retriever::Retriever;
use crate::vectorstore::ScoredDocument;

/// What an [`AgentStep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Decomposed the question into sub-queries.
    Plan,
    /// Ran a retrieval for a (sub-)query.
    Retrieve,
    /// Judged whether the context is sufficient.
    Evaluate,
    /// Produced the final answer.
    Synthesize,
}

/// One recorded action in an agent run.
#[derive(Debug, Clone)]
pub struct AgentStep {
    /// The kind of action.
    pub kind: StepKind,
    /// Human-readable detail (the query run, the verdict, …).
    pub detail: String,
}

/// The full record of an agentic run.
#[derive(Debug, Clone)]
pub struct AgentTrace {
    /// The synthesised final answer.
    pub answer: String,
    /// Every step taken, in order.
    pub steps: Vec<AgentStep>,
    /// All context documents accumulated across iterations.
    pub context: Vec<Document>,
}

/// An iterative retrieval agent.
pub struct AgenticRag<'a> {
    retriever: &'a dyn Retriever,
    llm: &'a dyn LlmClient,
    max_iterations: usize,
    top_k: usize,
}

impl<'a> AgenticRag<'a> {
    /// Build over a retriever and an LLM (default 2 iterations, top_k 3).
    pub fn new(retriever: &'a dyn Retriever, llm: &'a dyn LlmClient) -> Self {
        AgenticRag {
            retriever,
            llm,
            max_iterations: 2,
            top_k: 3,
        }
    }

    /// Max follow-up iterations after the initial plan.
    pub fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    /// Documents retrieved per (sub-)query.
    pub fn with_top_k(mut self, k: usize) -> Self {
        self.top_k = k;
        self
    }

    /// Run the full agentic loop for `question`.
    pub fn run(&self, question: &str) -> Result<AgentTrace> {
        let mut steps = Vec::new();
        let mut context: Vec<Document> = Vec::new();

        // --- Plan -------------------------------------------------------
        let plan_reply = self.llm.complete(&plan_prompt(question))?;
        let mut sub_queries: Vec<String> = plan_reply
            .lines()
            .map(|l| {
                l.trim()
                    .trim_start_matches(|c: char| {
                        c == '-' || c == '*' || c == '.' || c == ')' || c.is_ascii_digit() || c == ' '
                    })
                    .trim()
                    .to_string()
            })
            .filter(|l| !l.is_empty())
            .collect();
        if sub_queries.is_empty() {
            // Fall back to the original question.
            sub_queries.push(question.to_string());
        }
        steps.push(AgentStep {
            kind: StepKind::Plan,
            detail: sub_queries.join(" | "),
        });

        // --- Initial searches ------------------------------------------
        for sq in &sub_queries {
            self.search_into(sq, &mut context, &mut steps)?;
        }

        // --- Evaluate / iterate ----------------------------------------
        for _ in 0..self.max_iterations {
            let verdict = self.llm.complete(&sufficiency_prompt(question, &context))?;
            steps.push(AgentStep {
                kind: StepKind::Evaluate,
                detail: verdict.trim().to_string(),
            });
            if is_sufficient(&verdict) {
                break;
            }
            let followup = self.llm.complete(&followup_prompt(question, &context))?;
            let followup = followup.trim().to_string();
            if followup.is_empty() || followup == "n/a" {
                break;
            }
            self.search_into(&followup, &mut context, &mut steps)?;
        }

        // --- Synthesize -------------------------------------------------
        let answer = self.llm.complete(&synthesis_prompt(question, &context))?;
        steps.push(AgentStep {
            kind: StepKind::Synthesize,
            detail: answer.clone(),
        });

        Ok(AgentTrace {
            answer,
            steps,
            context,
        })
    }

    fn search_into(
        &self,
        query: &str,
        context: &mut Vec<Document>,
        steps: &mut Vec<AgentStep>,
    ) -> Result<()> {
        let hits = self.retriever.retrieve(query, self.top_k)?;
        steps.push(AgentStep {
            kind: StepKind::Retrieve,
            detail: query.to_string(),
        });
        for ScoredDocument { document, .. } in hits {
            // Dedup by content id so repeated queries don't bloat context.
            if !context.iter().any(|d| d.id() == document.id()) {
                context.push(document);
            }
        }
        Ok(())
    }
}

fn render_context(context: &[Document]) -> String {
    context
        .iter()
        .enumerate()
        .map(|(i, d)| format!("[{}] {}", i + 1, d.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn plan_prompt(question: &str) -> String {
    format!(
        "Decompose the question into a short list of focused search queries, \
         one per line.\n\nQuestion: {question}"
    )
}

fn sufficiency_prompt(question: &str, context: &[Document]) -> String {
    format!(
        "Is the retrieved context sufficient to answer the question? Reply \
         'yes' or 'no'.\n\nQuestion: {question}\n\nContext:\n{}",
        render_context(context)
    )
}

fn followup_prompt(question: &str, context: &[Document]) -> String {
    format!(
        "The context is insufficient. Write one follow-up search query that \
         would fill the gap.\n\nQuestion: {question}\n\nContext:\n{}",
        render_context(context)
    )
}

fn synthesis_prompt(question: &str, context: &[Document]) -> String {
    format!(
        "Using the context, write a final answer to the question, citing \
         sources as [n].\n\nQuestion: {question}\n\nContext:\n{}",
        render_context(context)
    )
}

fn is_sufficient(verdict: &str) -> bool {
    let v = verdict.trim().to_lowercase();
    v.starts_with("yes") || v.contains("sufficient") && !v.contains("insufficient")
}
