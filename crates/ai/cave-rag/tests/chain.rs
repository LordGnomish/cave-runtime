// SPDX-License-Identifier: AGPL-3.0-or-later
//! RAG generation chain + citation tracking + answer attribution.

use cave_rag::chain::{ExtractiveGenerator, Generator, LlmGenerator, RagPipeline};
use cave_rag::citation::{attribute_answer, build_citations};
use cave_rag::document::Document;
use cave_rag::embedding::HashingEmbedder;
use cave_rag::error::Result;
use cave_rag::rerank::{LexicalCrossEncoder, LlmClient};
use cave_rag::retriever::SimilarityRetriever;
use cave_rag::vectorstore::{InMemoryVectorStore, ScoredDocument, VectorStore};

fn store() -> (InMemoryVectorStore, HashingEmbedder) {
    let e = HashingEmbedder::new(256);
    let mut s = InMemoryVectorStore::new();
    s.add(
        vec![
            Document::new("Rust ensures memory safety without a garbage collector.")
                .with_source("rust-book"),
            Document::new("Python uses reference counting and a cyclic garbage collector.")
                .with_source("py-docs"),
            Document::new("The Eiffel Tower is a landmark located in Paris, France.")
                .with_source("wiki"),
        ],
        &e,
    )
    .unwrap();
    (s, e)
}

#[test]
fn extractive_generator_pulls_relevant_sentences() {
    let (store, e) = store();
    let r = SimilarityRetriever::new(&store, &e);
    let gen = ExtractiveGenerator::new();
    let pipeline = RagPipeline::new(&r, &gen).with_top_k(2);
    let answer = pipeline.query("how does rust handle memory safety").unwrap();
    assert!(
        answer.answer.to_lowercase().contains("memory safety"),
        "answer: {:?}",
        answer.answer
    );
    assert!(!answer.citations.is_empty());
    // The rust-book source should be cited and attributed as supporting.
    assert!(answer
        .citations
        .iter()
        .any(|c| c.source.as_deref() == Some("rust-book")));
}

/// Deterministic answer generator for the chain test.
struct ScriptedLlm;
impl LlmClient for ScriptedLlm {
    fn complete(&self, _prompt: &str) -> Result<String> {
        Ok("Rust ensures memory safety without a garbage collector.".to_string())
    }
}

#[test]
fn llm_pipeline_with_reranker_produces_grounded_answer() {
    let (store, e) = store();
    let r = SimilarityRetriever::new(&store, &e);
    let rr = LexicalCrossEncoder::new();
    let llm = ScriptedLlm;
    let gen = LlmGenerator::new(&llm);
    let pipeline = RagPipeline::new(&r, &gen).with_reranker(&rr).with_top_k(2);
    let answer = pipeline.query("memory safety in rust").unwrap();
    assert!(answer.answer.contains("memory safety"));
    // Attribution must flag the rust doc as the strongest support.
    assert!(!answer.attributions.is_empty());
    let best = answer
        .attributions
        .iter()
        .max_by(|a, b| a.support.partial_cmp(&b.support).unwrap())
        .unwrap();
    let cited = &answer.citations[best.citation_index];
    assert_eq!(cited.source.as_deref(), Some("rust-book"));
    assert!(best.support > 0.0);
}

#[test]
fn build_citations_one_per_context_doc_with_source() {
    let ctx = vec![
        ScoredDocument {
            document: Document::new("alpha content").with_source("a"),
            score: 0.9,
        },
        ScoredDocument {
            document: Document::new("beta content").with_source("b"),
            score: 0.5,
        },
    ];
    let cites = build_citations(&ctx);
    assert_eq!(cites.len(), 2);
    assert_eq!(cites[0].index, 0);
    assert_eq!(cites[0].source.as_deref(), Some("a"));
    assert_eq!(cites[1].source.as_deref(), Some("b"));
}

#[test]
fn attribution_grounds_answer_in_supporting_source() {
    let ctx = vec![
        ScoredDocument {
            document: Document::new("The Eiffel Tower is located in Paris.").with_source("wiki"),
            score: 0.9,
        },
        ScoredDocument {
            document: Document::new("Bananas are a tropical fruit rich in potassium.")
                .with_source("food"),
            score: 0.4,
        },
    ];
    let attrs = attribute_answer("The Eiffel Tower is in Paris.", &ctx);
    assert_eq!(attrs.len(), 2);
    let wiki = attrs.iter().find(|a| a.citation_index == 0).unwrap();
    let food = attrs.iter().find(|a| a.citation_index == 1).unwrap();
    assert!(
        wiki.support > food.support,
        "the Paris source must outscore the banana source"
    );
}
