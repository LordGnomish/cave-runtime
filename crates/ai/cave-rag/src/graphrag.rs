// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GraphRAG: knowledge-graph extraction, communities, local search.
//!
//! Ports the `microsoft/graphrag` indexing idea — turn a corpus into an
//! entity/relationship graph, partition it into communities, then answer a
//! query from the *local neighborhood* of the entities it mentions rather
//! than a flat top-k of chunks.
//!
//! Two extractors ship:
//!
//! * [`extract_graph`] — a dependency-free heuristic: capitalized tokens are
//!   entities, and entities co-occurring inside one sentence are linked. Good
//!   enough to demo community detection and local search fully offline.
//! * [`LlmGraphExtractor`] — the real graphrag pattern: prompt an
//!   [`LlmClient`] to emit `Entity | RELATION | Entity` triples and parse the
//!   graph out of them. Malformed lines are skipped, never fatal.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::error::Result;
use crate::rerank::LlmClient;

/// A directed, labeled relationship between two entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    /// Source entity name.
    pub source: String,
    /// Relationship label (e.g. `DEVELOPED`, `RELATED`).
    pub relation: String,
    /// Target entity name.
    pub target: String,
}

/// An entity/relationship knowledge graph.
///
/// Adjacency is stored undirected so [`neighbors`](Self::neighbors),
/// [`communities`](Self::communities) and [`local_search`](Self::local_search)
/// traverse the graph regardless of edge direction, while
/// [`relationships`](Self::relationships) keeps the original directed,
/// labeled triples.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeGraph {
    entities: BTreeSet<String>,
    adjacency: BTreeMap<String, BTreeSet<String>>,
    relationships: Vec<Relationship>,
}

impl KnowledgeGraph {
    /// An empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an entity (idempotent).
    pub fn add_entity(&mut self, name: impl Into<String>) {
        self.entities.insert(name.into());
    }

    /// Add a labeled, directed relationship, registering both endpoints and
    /// recording the undirected adjacency. Self-loops are ignored.
    pub fn add_relationship(
        &mut self,
        source: impl Into<String>,
        relation: impl Into<String>,
        target: impl Into<String>,
    ) {
        let (source, relation, target) = (source.into(), relation.into(), target.into());
        self.add_entity(source.clone());
        self.add_entity(target.clone());
        if source == target {
            return;
        }
        self.adjacency
            .entry(source.clone())
            .or_default()
            .insert(target.clone());
        self.adjacency
            .entry(target.clone())
            .or_default()
            .insert(source.clone());
        self.relationships
            .push(Relationship { source, relation, target });
    }

    /// True if `name` is a known entity.
    pub fn has_entity(&self, name: &str) -> bool {
        self.entities.contains(name)
    }

    /// Number of registered entities.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Number of recorded relationships (directed triples).
    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }

    /// All recorded relationships.
    pub fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }

    /// Directly linked entities (undirected), sorted by name.
    pub fn neighbors(&self, name: &str) -> Vec<String> {
        self.adjacency
            .get(name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Partition the graph into connected components (graphrag "communities").
    ///
    /// Each community is the sorted entity set of one connected component;
    /// isolated entities form singletons. Components are returned sorted by
    /// their smallest member for determinism.
    pub fn communities(&self) -> Vec<Vec<String>> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out: Vec<Vec<String>> = Vec::new();
        for start in &self.entities {
            if seen.contains(start) {
                continue;
            }
            let mut component: BTreeSet<String> = BTreeSet::new();
            let mut queue: VecDeque<String> = VecDeque::new();
            queue.push_back(start.clone());
            seen.insert(start.clone());
            while let Some(node) = queue.pop_front() {
                component.insert(node.clone());
                for nbr in self.adjacency.get(&node).into_iter().flatten() {
                    if seen.insert(nbr.clone()) {
                        queue.push_back(nbr.clone());
                    }
                }
            }
            out.push(component.into_iter().collect());
        }
        out.sort_by(|a, b| a.first().cmp(&b.first()));
        out
    }

    /// Local search: gather the `hops`-neighborhood of every entity named in
    /// `query`. Returns the entity names reachable within `hops` edges of any
    /// matched seed (seeds included), sorted for determinism.
    pub fn local_search(&self, query: &str, hops: usize) -> Vec<String> {
        // Seed on entities whose name appears verbatim in the query.
        let seeds: Vec<String> = self
            .entities
            .iter()
            .filter(|e| query.contains(e.as_str()))
            .cloned()
            .collect();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut frontier: BTreeSet<String> = seeds.into_iter().collect();
        for s in &frontier {
            visited.insert(s.clone());
        }
        for _ in 0..hops {
            let mut next: BTreeSet<String> = BTreeSet::new();
            for node in &frontier {
                for nbr in self.adjacency.get(node).into_iter().flatten() {
                    if !visited.contains(nbr) {
                        next.insert(nbr.clone());
                    }
                }
            }
            for n in &next {
                visited.insert(n.clone());
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        visited.into_iter().collect()
    }
}

/// Extract a [`KnowledgeGraph`] from raw text.
pub trait GraphExtractor {
    /// Build a graph from `text`.
    fn extract(&self, text: &str) -> Result<KnowledgeGraph>;
}

/// Heuristic, dependency-free graph extraction.
///
/// Capitalized word tokens are treated as entities; entities co-occurring in
/// the same sentence are linked with a generic `RELATED` edge. Sentence
/// boundaries (`.`, `!`, `?`) bound co-occurrence, so entities in different
/// sentences are *not* linked.
pub fn extract_graph(text: &str) -> KnowledgeGraph {
    let mut g = KnowledgeGraph::new();
    for sentence in split_sentences(text) {
        let entities = capitalized_entities(&sentence);
        for e in &entities {
            g.add_entity(e.clone());
        }
        // Link every unordered pair within the sentence.
        for i in 0..entities.len() {
            for j in (i + 1)..entities.len() {
                g.add_relationship(entities[i].clone(), "RELATED", entities[j].clone());
            }
        }
    }
    g
}

/// LLM-driven extractor: prompts a model for `Entity | RELATION | Entity`
/// triples (the graphrag extraction prompt) and parses the graph out.
pub struct LlmGraphExtractor<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> LlmGraphExtractor<'a> {
    /// Build over an [`LlmClient`].
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        LlmGraphExtractor { llm }
    }

    /// Render the triple-extraction prompt (public so callers can audit it).
    pub fn build_prompt(text: &str) -> String {
        format!(
            "Extract a knowledge graph from the text. Emit one relationship per \
             line as `Entity | RELATION | Entity`. Use only information present \
             in the text.\n\nText:\n{text}\n\nTriples:"
        )
    }
}

impl GraphExtractor for LlmGraphExtractor<'_> {
    fn extract(&self, text: &str) -> Result<KnowledgeGraph> {
        let reply = self.llm.complete(&Self::build_prompt(text))?;
        Ok(parse_triples(&reply))
    }
}

/// Parse `Entity | RELATION | Entity` lines into a graph. Lines that do not
/// have exactly three `|`-separated non-empty fields are skipped.
pub fn parse_triples(text: &str) -> KnowledgeGraph {
    let mut g = KnowledgeGraph::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
            continue;
        }
        g.add_relationship(parts[0], parts[1], parts[2]);
    }
    g
}

/// Pull capitalized word tokens out of a sentence, in order, de-duplicated
/// while preserving first-occurrence order.
fn capitalized_entities(sentence: &str) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for raw in sentence.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let first = raw.chars().next().unwrap();
        if first.is_ascii_uppercase() && seen.insert(raw.to_string()) {
            out.push(raw.to_string());
        }
    }
    out
}

/// Naive sentence segmentation on `.`, `!`, `?`.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in text.chars() {
        if matches!(c, '.' | '!' | '?') {
            let s = buf.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
            buf.clear();
        } else {
            buf.push(c);
        }
    }
    let tail = buf.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}
