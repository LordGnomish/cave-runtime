// SPDX-License-Identifier: AGPL-3.0-or-later
//! GraphRAG: knowledge-graph extraction, communities, local search.

use cave_rag::error::Result;
use cave_rag::graphrag::{
    extract_graph, GraphExtractor, KnowledgeGraph, LlmGraphExtractor,
};
use cave_rag::rerank::LlmClient;

#[test]
fn heuristic_extraction_builds_entities_and_edges() {
    let text = "Alice works with Bob at Acme. Carol leads the team at Globex with Dave.";
    let g: KnowledgeGraph = extract_graph(text);
    for e in ["Alice", "Bob", "Acme", "Carol", "Globex", "Dave"] {
        assert!(g.has_entity(e), "missing entity {e}");
    }
    // Co-occurring entities in a sentence are linked.
    let neighbors = g.neighbors("Alice");
    assert!(neighbors.contains(&"Bob".to_string()));
    assert!(neighbors.contains(&"Acme".to_string()));
    assert!(!neighbors.contains(&"Globex".to_string()), "cross-sentence, no edge");
}

#[test]
fn communities_are_connected_components() {
    let text = "Alice works with Bob at Acme. Carol leads the team at Globex with Dave.";
    let g = extract_graph(text);
    let communities = g.communities();
    assert_eq!(communities.len(), 2, "two disconnected clusters");
    // Each community is internally consistent.
    let acme = communities
        .iter()
        .find(|c| c.contains(&"Acme".to_string()))
        .unwrap();
    assert!(acme.contains(&"Alice".to_string()) && acme.contains(&"Bob".to_string()));
    assert!(!acme.contains(&"Globex".to_string()));
}

#[test]
fn local_search_returns_neighborhood() {
    let text = "Alice works with Bob at Acme. Carol leads the team at Globex with Dave.";
    let g = extract_graph(text);
    let ctx = g.local_search("tell me about Alice", 1);
    assert!(ctx.contains(&"Bob".to_string()));
    assert!(ctx.contains(&"Acme".to_string()));
    assert!(!ctx.contains(&"Dave".to_string()));
}

/// Scripted LLM emitting graphrag-style `Entity | RELATION | Entity` triples.
struct TripleLlm;
impl LlmClient for TripleLlm {
    fn complete(&self, _prompt: &str) -> Result<String> {
        Ok("Einstein | DEVELOPED | Relativity\n\
            Relativity | PUBLISHED_IN | 1915\n\
            garbage line without delimiters"
            .to_string())
    }
}

#[test]
fn llm_extractor_parses_triples() {
    let llm = TripleLlm;
    let extractor = LlmGraphExtractor::new(&llm);
    let g = extractor.extract("(ignored — scripted)").unwrap();
    assert!(g.has_entity("Einstein"));
    assert!(g.has_entity("Relativity"));
    assert!(g.neighbors("Einstein").contains(&"Relativity".to_string()));
    // The malformed line is skipped, not fatal.
    assert!(g.relationship_count() == 2);
}
