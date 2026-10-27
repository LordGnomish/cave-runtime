// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GraphRAG.
//!
//! Ports the microsoft/graphrag shape to Rust: extract a [`KnowledgeGraph`]
//! of entities and relationships from text, group it into communities
//! (connected components), and answer queries with *local search* — seed on
//! the entities named in the query and gather their neighbourhood.
//!
//! Two extractors ship:
//!
//! * [`extract_graph`] / [`HeuristicGraphExtractor`] — model-free: entities
//!   are maximal runs of capitalised words, relationships are intra-sentence
//!   co-occurrences.
//! * [`LlmGraphExtractor`] — prompts an [`LlmClient`] for
//!   `Entity | RELATION | Entity` triples (the graphrag extraction prompt) and
//!   parses them into the graph.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::error::Result;
use crate::rerank::LlmClient;

/// A knowledge graph of entities (nodes) and relationships (edges).
#[derive(Debug, Clone, Default)]
pub struct KnowledgeGraph {
    entities: BTreeSet<String>,
    /// undirected adjacency
    adjacency: BTreeMap<String, BTreeSet<String>>,
    /// (source, target, description) triples in insertion order
    relationships: Vec<(String, String, String)>,
}

impl KnowledgeGraph {
    /// Empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an entity (idempotent).
    pub fn add_entity(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.adjacency.entry(name.clone()).or_default();
        self.entities.insert(name);
    }

    /// Add an undirected relationship, creating endpoints as needed.
    pub fn add_relationship(
        &mut self,
        source: impl Into<String>,
        target: impl Into<String>,
        description: impl Into<String>,
    ) {
        let (s, t, d) = (source.into(), target.into(), description.into());
        if s == t {
            return;
        }
        self.add_entity(s.clone());
        self.add_entity(t.clone());
        self.adjacency.get_mut(&s).unwrap().insert(t.clone());
        self.adjacency.get_mut(&t).unwrap().insert(s.clone());
        self.relationships.push((s, t, d));
    }

    /// Is this entity present?
    pub fn has_entity(&self, name: &str) -> bool {
        self.entities.contains(name)
    }

    /// Number of entities.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Number of stored relationships.
    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }

    /// Direct neighbours of an entity (sorted).
    pub fn neighbors(&self, name: &str) -> Vec<String> {
        self.adjacency
            .get(name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Connected components — graphrag's communities, simplest form.
    pub fn communities(&self) -> Vec<Vec<String>> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out = Vec::new();
        for start in &self.entities {
            if seen.contains(start) {
                continue;
            }
            let mut component = Vec::new();
            let mut queue = VecDeque::from([start.clone()]);
            seen.insert(start.clone());
            while let Some(node) = queue.pop_front() {
                component.push(node.clone());
                for nb in self.adjacency.get(&node).into_iter().flatten() {
                    if seen.insert(nb.clone()) {
                        queue.push_back(nb.clone());
                    }
                }
            }
            component.sort();
            out.push(component);
        }
        out
    }

    /// Local search: seed on entities named in `query`, then BFS out to
    /// `depth` hops; return the reachable neighbourhood (excluding the seeds).
    pub fn local_search(&self, query: &str, depth: usize) -> Vec<String> {
        let lower = query.to_lowercase();
        let seeds: Vec<String> = self
            .entities
            .iter()
            .filter(|e| lower.contains(&e.to_lowercase()))
            .cloned()
            .collect();
        let seed_set: BTreeSet<String> = seeds.iter().cloned().collect();
        let mut visited: BTreeSet<String> = seed_set.clone();
        let mut frontier: Vec<String> = seeds;
        let mut result: BTreeSet<String> = BTreeSet::new();
        for _ in 0..depth {
            let mut next = Vec::new();
            for node in &frontier {
                for nb in self.neighbors(node) {
                    if visited.insert(nb.clone()) {
                        result.insert(nb.clone());
                        next.push(nb);
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        result.into_iter().collect()
    }
}

/// Pluggable graph extraction strategy.
pub trait GraphExtractor {
    /// Extract a knowledge graph from `text`.
    fn extract(&self, text: &str) -> Result<KnowledgeGraph>;
}

/// Model-free extractor: capitalised-word runs are entities, intra-sentence
/// co-occurrences are relationships.
#[derive(Debug, Clone, Default)]
pub struct HeuristicGraphExtractor;

impl GraphExtractor for HeuristicGraphExtractor {
    fn extract(&self, text: &str) -> Result<KnowledgeGraph> {
        Ok(extract_graph(text))
    }
}

/// Convenience wrapper around [`HeuristicGraphExtractor`].
pub fn extract_graph(text: &str) -> KnowledgeGraph {
    let mut g = KnowledgeGraph::new();
    for sentence in split_sentences(text) {
        let entities = capitalized_runs(&sentence);
        for e in &entities {
            g.add_entity(e.clone());
        }
        // Link every pair co-occurring in the sentence.
        for i in 0..entities.len() {
            for j in (i + 1)..entities.len() {
                g.add_relationship(
                    entities[i].clone(),
                    entities[j].clone(),
                    "co-occurrence".to_string(),
                );
            }
        }
    }
    g
}

/// LLM-driven extractor: parses `Entity | RELATION | Entity` triples.
pub struct LlmGraphExtractor<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> LlmGraphExtractor<'a> {
    /// Build over an [`LlmClient`].
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        LlmGraphExtractor { llm }
    }

    fn prompt(text: &str) -> String {
        format!(
            "Extract a knowledge graph from the text. Output one relationship \
             per line as `Entity | RELATION | Entity`. Text:\n{text}"
        )
    }
}

impl GraphExtractor for LlmGraphExtractor<'_> {
    fn extract(&self, text: &str) -> Result<KnowledgeGraph> {
        let reply = self.llm.complete(&Self::prompt(text))?;
        let mut g = KnowledgeGraph::new();
        for line in reply.lines() {
            let parts: Vec<&str> = line.split('|').map(|p| p.trim()).collect();
            if parts.len() == 3 && !parts[0].is_empty() && !parts[2].is_empty() {
                g.add_relationship(parts[0], parts[2], parts[1]);
            }
        }
        Ok(g)
    }
}

/// Maximal runs of capitalised, alphanumeric tokens within a sentence.
fn capitalized_runs(sentence: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for raw in sentence.split_whitespace() {
        let tok = raw.trim_matches(|c: char| !c.is_alphanumeric());
        let is_cap = tok
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);
        if is_cap {
            current.push(tok);
        } else if !current.is_empty() {
            runs.push(current.join(" "));
            current.clear();
        }
    }
    if !current.is_empty() {
        runs.push(current.join(" "));
    }
    // De-duplicate within a sentence while preserving order.
    let mut seen = BTreeSet::new();
    runs.into_iter().filter(|r| seen.insert(r.clone())).collect()
}

/// Naive sentence segmentation on `.`, `!`, `?`.
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
