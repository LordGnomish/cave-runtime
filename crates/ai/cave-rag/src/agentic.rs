// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Agentic RAG: plan → search → evaluate → iterate → synthesize.
//!
//! Where a plain [`RagPipeline`](crate::chain::RagPipeline) does a single
//! retrieve→generate pass, an agentic loop (the llama_index
//! `ReActAgent` / langchain self-querying pattern) first *plans* the question
//! into sub-queries, retrieves for each, then asks the model whether the
//! gathered context is *sufficient* — iterating until it is or an iteration
//! budget is hit — before synthesizing a final, grounded answer.
//!
//! Every decision is recorded as a [`Step`] so the whole [`AgentTrace`] is
//! auditable: which sub-queries were planned, what was retrieved, and where
//! the loop stopped.

use std::collections::BTreeSet;

use crate::document::Document;
use crate::error::Result;
use crate::rerank::LlmClient;
use crate::retriever::Retriever;

/// The kind of action an agent took at a given step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Decomposed the question into sub-queries.
    Plan,
    /// Retrieved context for a (sub-)query.
    Retrieve,
    /// Judged whether the gathered context is sufficient.
    Evaluate,
    /// Synthesized the final answer.
    Synthesize,
}

/// One recorded action in an agent run.
#[derive(Debug, Clone)]
pub struct Step {
    /// What kind of action this was.
    pub kind: StepKind,
    /// Human-readable detail (the sub-query, the verdict, …).
    pub detail: String,
}

/// The full, auditable result of an agentic run.
#[derive(Debug, Clone)]
pub struct AgentTrace {
    /// The synthesized final answer.
    pub answer: String,
    /// Ordered record of every action taken.
    pub steps: Vec<Step>,
    /// De-duplicated context accumulated across all retrievals.
    pub context: Vec<Document>,
}

/// Plan→search→evaluate→iterate→synthesize agent over a [`Retriever`] and an
/// [`LlmClient`].
pub struct AgenticRag<'a> {
    retriever: &'a dyn Retriever,
    llm: &'a dyn LlmClient,
    max_iterations: usize,
    top_k: usize,
}

impl<'a> AgenticRag<'a> {
    /// Build over a retriever and an LLM (defaults: 3 iterations, top_k 4).
    pub fn new(retriever: &'a dyn Retriever, llm: &'a dyn LlmClient) -> Self {
        AgenticRag {
            retriever,
            llm,
            max_iterations: 3,
            top_k: 4,
        }
    }

    /// Cap the number of plan→retrieve→evaluate iterations.
    pub fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n.max(1);
        self
    }

    /// Documents retrieved per (sub-)query.
    pub fn with_top_k(mut self, k: usize) -> Self {
        self.top_k = k.max(1);
        self
    }

    /// Run the agentic loop for `question` and return its trace.
    pub fn run(&self, question: &str) -> Result<AgentTrace> {
        let mut steps: Vec<Step> = Vec::new();
        let mut context: Vec<Document> = Vec::new();
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();

        // 1. Plan: decompose into sub-queries (fall back to the question).
        let plan_reply = self.llm.complete(&plan_prompt(question))?;
        let mut subqueries: Vec<String> = plan_reply
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        steps.push(Step {
            kind: StepKind::Plan,
            detail: plan_reply.trim().to_string(),
        });
        if subqueries.is_empty() {
            subqueries = vec![question.to_string()];
        }

        // 2. Iterate: retrieve for each sub-query, then ask if it's enough.
        for _ in 0..self.max_iterations {
            for sq in &subqueries {
                let hits = self.retriever.retrieve(sq, self.top_k)?;
                steps.push(Step {
                    kind: StepKind::Retrieve,
                    detail: sq.clone(),
                });
                for h in hits {
                    if seen_ids.insert(h.document.id()) {
                        context.push(h.document);
                    }
                }
            }
            let verdict = self.llm.complete(&sufficiency_prompt(question, &context))?;
            let sufficient = verdict.to_lowercase().contains("yes");
            steps.push(Step {
                kind: StepKind::Evaluate,
                detail: verdict.trim().to_string(),
            });
            if sufficient {
                break;
            }
        }

        // 3. Synthesize the final grounded answer.
        let answer = self.llm.complete(&synthesis_prompt(question, &context))?;
        steps.push(Step {
            kind: StepKind::Synthesize,
            detail: answer.trim().to_string(),
        });

        Ok(AgentTrace {
            answer,
            steps,
            context,
        })
    }
}

/// Prompt that asks the model to *Decompose* the question into sub-questions.
fn plan_prompt(question: &str) -> String {
    format!(
        "Decompose the question into a short list of standalone sub-questions, \
         one per line, that together answer it. Emit only the sub-questions.\n\n\
         Question: {question}"
    )
}

/// Prompt that asks whether the gathered context is *sufficient* (yes/no).
fn sufficiency_prompt(question: &str, context: &[Document]) -> String {
    let mut p = String::from(
        "Given the context, is it sufficient to fully answer the question? \
         Reply with only `yes` or `no`.\n\nContext:\n",
    );
    for (i, d) in context.iter().enumerate() {
        p.push_str(&format!("[{}] {}\n", i + 1, d.content));
    }
    p.push_str(&format!("\nQuestion: {question}\n\nSufficient:"));
    p
}

/// Prompt that asks the model to write the *final answer* from the context.
fn synthesis_prompt(question: &str, context: &[Document]) -> String {
    let mut p = String::from(
        "Write the final answer to the question using only the context below. \
         Be concise and grounded.\n\nContext:\n",
    );
    for (i, d) in context.iter().enumerate() {
        let src = d.metadata.source.as_deref().unwrap_or("unknown");
        p.push_str(&format!("[{}] ({}) {}\n", i + 1, src, d.content));
    }
    p.push_str(&format!("\nQuestion: {question}\n\nFinal answer:"));
    p
}
