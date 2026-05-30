// SPDX-License-Identifier: AGPL-3.0-or-later
//! Agentic RAG: plan -> search -> evaluate -> iterate -> synthesize.

use cave_rag::agentic::{AgenticRag, StepKind};
use cave_rag::document::Document;
use cave_rag::embedding::HashingEmbedder;
use cave_rag::error::Result;
use cave_rag::rerank::LlmClient;
use cave_rag::retriever::SimilarityRetriever;
use cave_rag::vectorstore::{InMemoryVectorStore, VectorStore};

/// Scripted planner/judge/synthesizer keyed on prompt markers.
struct AgentLlm;
impl LlmClient for AgentLlm {
    fn complete(&self, prompt: &str) -> Result<String> {
        if prompt.contains("Decompose") {
            Ok("What is rust memory safety?\nDoes rust use a garbage collector?".to_string())
        } else if prompt.contains("sufficient") {
            Ok("yes".to_string())
        } else if prompt.contains("final answer") {
            Ok("Rust provides memory safety without a garbage collector.".to_string())
        } else {
            Ok("n/a".to_string())
        }
    }
}

fn corpus_store() -> (InMemoryVectorStore, HashingEmbedder) {
    let e = HashingEmbedder::new(256);
    let mut s = InMemoryVectorStore::new();
    s.add(
        vec![
            Document::new("Rust ensures memory safety via ownership and borrowing.")
                .with_source("rust-1"),
            Document::new("Rust has no garbage collector; memory is freed deterministically.")
                .with_source("rust-2"),
            Document::new("Bananas are a tropical fruit.").with_source("noise"),
        ],
        &e,
    )
    .unwrap();
    (s, e)
}

#[test]
fn agent_plans_searches_and_synthesizes() {
    let (store, e) = corpus_store();
    let r = SimilarityRetriever::new(&store, &e);
    let llm = AgentLlm;
    let agent = AgenticRag::new(&r, &llm).with_max_iterations(2).with_top_k(2);
    let trace = agent.run("how does rust manage memory safety").unwrap();

    assert!(trace.answer.to_lowercase().contains("memory safety"));
    // The plan produced two sub-queries -> at least two retrieval steps.
    let retrievals = trace
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::Retrieve)
        .count();
    assert!(retrievals >= 2, "expected >=2 retrieval steps, got {retrievals}");
    // A planning step was recorded.
    assert!(trace.steps.iter().any(|s| s.kind == StepKind::Plan));
    // Accumulated context surfaced the relevant rust docs.
    assert!(trace
        .context
        .iter()
        .any(|d| d.content.to_lowercase().contains("ownership")));
}

#[test]
fn agent_falls_back_to_question_when_plan_is_empty() {
    struct EmptyPlanLlm;
    impl LlmClient for EmptyPlanLlm {
        fn complete(&self, prompt: &str) -> Result<String> {
            if prompt.contains("sufficient") {
                Ok("yes".to_string())
            } else if prompt.contains("final answer") {
                Ok("answer text".to_string())
            } else {
                Ok("   ".to_string()) // empty/blank plan
            }
        }
    }
    let (store, e) = corpus_store();
    let r = SimilarityRetriever::new(&store, &e);
    let llm = EmptyPlanLlm;
    let agent = AgenticRag::new(&r, &llm);
    let trace = agent.run("rust memory").unwrap();
    let retrievals = trace
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::Retrieve)
        .count();
    assert!(retrievals >= 1, "must retrieve at least once using the question itself");
}
