// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query AST: term, boolean, phrase queries with execution against the inverted index.
//!
//! Implements Manticore-equivalent query execution:
//! - `Query::Term`   — single-term lookup via posting list
//! - `Query::Phrase` — ordered phrase matching using positional analysis
//! - `Query::Bool`   — compound boolean: must (AND) + should (OR) + must_not (NOT)
//!
//! upstream: manticoresoftware/manticoresearch — src/searchdaemon.cpp query parsing
//!           and src/sphinxsearch.cpp for matching logic.

use crate::index::Index;
use std::collections::HashSet;

/// Query AST node.
#[derive(Debug)]
pub enum Query {
    /// Single-term lookup: return all docs containing this term.
    Term(String),
    /// Phrase query: return docs where all tokens appear in the given order consecutively.
    Phrase(Vec<String>),
    /// Boolean compound: intersection of must, union of should, exclusion of must_not.
    Bool(BoolNode),
}

/// Boolean compound query node.
#[derive(Debug)]
pub struct BoolNode {
    /// All of these sub-queries must match (AND logic).
    pub must: Vec<Query>,
    /// At least one of these sub-queries should match (OR logic, used if must is empty).
    pub should: Vec<Query>,
    /// Docs matching any of these are excluded.
    pub must_not: Vec<Query>,
}

impl Query {
    /// Execute this query against `index` and return matching doc IDs.
    pub fn execute(&self, index: &Index) -> Vec<u32> {
        match self {
            Query::Term(term) => index.get_doc_ids_for_term(term),

            Query::Phrase(terms) => {
                if terms.is_empty() {
                    return Vec::new();
                }
                // Phrase matching: find all docs that contain the phrase as a
                // contiguous ordered sequence of tokens.
                //
                // Strategy: intersect posting lists for all terms first to get
                // candidate docs, then verify phrase order using the stored
                // document text (re-tokenize from the index raw text is not
                // available in this in-memory model, so we use a positional
                // approach: verify term order via posting list positions).
                //
                // Since our PostingList stores only per-doc TF (no position
                // vectors), we do a simpler but honest approximation: use the
                // document source text that's stored in the index phrase store.
                phrase_match(index, terms)
            }

            Query::Bool(node) => {
                execute_bool(index, node)
            }
        }
    }
}

/// Execute a boolean compound query.
fn execute_bool(index: &Index, node: &BoolNode) -> Vec<u32> {
    // Collect must_not doc set first (we need it for filtering).
    let excluded: HashSet<u32> = node
        .must_not
        .iter()
        .flat_map(|q| q.execute(index))
        .collect();

    let candidates: Vec<u32> = if !node.must.is_empty() {
        // AND: intersect all must results.
        let mut iter = node.must.iter().map(|q| {
            q.execute(index).into_iter().collect::<HashSet<u32>>()
        });
        let first = iter.next().unwrap_or_default();
        iter.fold(first, |acc, set| acc.intersection(&set).copied().collect())
            .into_iter()
            .collect()
    } else if !node.should.is_empty() {
        // OR: union all should results.
        node.should
            .iter()
            .flat_map(|q| q.execute(index))
            .collect::<HashSet<u32>>()
            .into_iter()
            .collect()
    } else {
        // No must or should — return all docs in index (subject to must_not).
        // We can enumerate all doc IDs that appear in any posting list.
        index.all_doc_ids()
    };

    // Apply must_not exclusion.
    let mut result: Vec<u32> = candidates
        .into_iter()
        .filter(|doc_id| !excluded.contains(doc_id))
        .collect();
    result.sort_unstable();
    result
}

/// Phrase matching: find docs where `terms` appear consecutively and in order.
///
/// Relies on `Index::phrase_candidates` which stores raw document tokens
/// for positional matching.
fn phrase_match(index: &Index, terms: &[String]) -> Vec<u32> {
    // Use the phrase token store from the index.
    let candidates = index.phrase_candidates(terms);
    candidates
        .into_iter()
        .filter(|&doc_id| index.check_phrase(doc_id, terms))
        .collect()
}

/// Convenience constructor helpers (mirrors Manticore query builder API).
pub struct BooleanQuery;

impl BooleanQuery {
    /// Create an AND query (all sub-queries must match).
    pub fn and(subs: Vec<Query>) -> Query {
        Query::Bool(BoolNode {
            must: subs,
            should: vec![],
            must_not: vec![],
        })
    }

    /// Create an OR query (any sub-query must match).
    pub fn or(subs: Vec<Query>) -> Query {
        Query::Bool(BoolNode {
            must: vec![],
            should: subs,
            must_not: vec![],
        })
    }

    /// Create a NOT query (complement: all docs NOT matching sub-query).
    pub fn not(sub: Query) -> Query {
        Query::Bool(BoolNode {
            must: vec![],
            should: vec![],
            must_not: vec![sub],
        })
    }
}

/// Phrase query helper.
pub struct PhraseQuery;

impl PhraseQuery {
    /// Build a phrase query from a list of terms.
    pub fn of(terms: Vec<String>) -> Query {
        Query::Phrase(terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::TenantId;
    use std::str::FromStr;

    fn build_idx() -> Index {
        let t = TenantId::from_str("default").unwrap();
        let mut idx = Index::new(&t, "test");
        idx.add_document(1, "quick brown fox");
        idx.add_document(2, "slow brown dog");
        idx.add_document(3, "quick slow runner");
        idx
    }

    #[test]
    fn term_finds_docs() {
        let idx = build_idx();
        let r = Query::Term("fox".to_string()).execute(&idx);
        assert_eq!(r, vec![1]);
    }

    #[test]
    fn and_intersects() {
        let idx = build_idx();
        let q = BooleanQuery::and(vec![
            Query::Term("quick".to_string()),
            Query::Term("slow".to_string()),
        ]);
        let r = q.execute(&idx);
        assert_eq!(r, vec![3]);
    }

    #[test]
    fn or_unions() {
        let idx = build_idx();
        let q = BooleanQuery::or(vec![
            Query::Term("fox".to_string()),
            Query::Term("dog".to_string()),
        ]);
        let mut r = q.execute(&idx);
        r.sort();
        assert_eq!(r, vec![1, 2]);
    }

    #[test]
    fn not_excludes() {
        let idx = build_idx();
        let q = BooleanQuery::not(Query::Term("quick".to_string()));
        let mut r = q.execute(&idx);
        r.sort();
        assert!(!r.contains(&1));
        assert!(!r.contains(&3));
        assert!(r.contains(&2));
    }
}
