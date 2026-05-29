// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query AST: Term, Boolean (AND/OR/NOT), Phrase queries with execution.
//!
//! Implements Manticore-equivalent query execution:
//! - `Query::Term`   — single-term posting-list lookup
//! - `Query::Phrase` — ordered phrase matching using positional analysis
//! - `Query::Bool`   — compound boolean: must (AND) + should (OR) + must_not (NOT)
//!
//! upstream: manticoresoftware/manticoresearch v25.8.2 — src/sphinxsearch.cpp

use crate::index::Index;
use std::collections::HashSet;

/// Query AST node.
#[derive(Debug)]
pub enum Query {
    /// Single-term lookup: return all docs containing this term.
    Term(String),
    /// Phrase query: docs where tokens appear contiguously in given order.
    Phrase(Vec<String>),
    /// Boolean compound: must (AND) + should (OR) + must_not (NOT exclusion).
    Bool(BoolNode),
}

/// Boolean compound query node.
#[derive(Debug)]
pub struct BoolNode {
    /// All of these sub-queries must match (AND logic).
    pub must: Vec<Query>,
    /// At least one of these must match (OR logic; used only if `must` is empty).
    pub should: Vec<Query>,
    /// Docs matching any of these are excluded.
    pub must_not: Vec<Query>,
}

impl Query {
    /// Execute this query against `index` and return matching doc IDs.
    pub fn execute(&self, index: &Index) -> Vec<u32> {
        match self {
            Query::Term(term) => index.get_doc_ids_for_term(term),
            Query::Phrase(terms) => phrase_match(index, terms),
            Query::Bool(node) => execute_bool(index, node),
        }
    }
}

/// Execute a boolean compound query.
fn execute_bool(index: &Index, node: &BoolNode) -> Vec<u32> {
    // Collect must_not doc set first.
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
        // No must/should: return all docs (subject to must_not exclusion).
        index.all_doc_ids()
    };

    let mut result: Vec<u32> = candidates
        .into_iter()
        .filter(|id| !excluded.contains(id))
        .collect();
    result.sort_unstable();
    result
}

/// Phrase matching: find docs where `terms` appear consecutively and in order.
fn phrase_match(index: &Index, terms: &[String]) -> Vec<u32> {
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
        Query::Bool(BoolNode { must: subs, should: vec![], must_not: vec![] })
    }

    /// Create an OR query (any sub-query must match).
    pub fn or(subs: Vec<Query>) -> Query {
        Query::Bool(BoolNode { must: vec![], should: subs, must_not: vec![] })
    }

    /// Create a NOT query (complement: exclude all docs matching sub-query).
    pub fn not(sub: Query) -> Query {
        Query::Bool(BoolNode { must: vec![], should: vec![], must_not: vec![sub] })
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
