// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Query AST: term, phrase, boolean.
//!
//! Upstream reference: Lucene `org.apache.lucene.search.Query` hierarchy and
//! OpenSearch `query/QueryBuilder`.  Execution here is set-based against the
//! in-memory `Index`: each leaf resolves to the doc-id set carrying the term,
//! and `Bool` combines them with intersect/union/subtract.
//!
//! NOTE — `Phrase` currently collapses to a conjunction of its terms because
//! `PostingList` does not yet carry per-doc positions.  This matches the
//! "OR a Phrase that is degraded to AND" behaviour Elasticsearch falls back
//! to when `index_options: docs`.  When positional postings land, the
//! `Phrase` arm will tighten without an API break.

use crate::index::Index;
use std::collections::BTreeSet;

pub enum Query {
    Term(String),
    Phrase(Vec<String>),
    Bool(BoolNode),
}

pub struct BoolNode {
    pub must: Vec<Query>,
    pub should: Vec<Query>,
    pub must_not: Vec<Query>,
}

impl Query {
    pub fn execute(&self, index: &Index) -> Vec<u32> {
        match self {
            Query::Term(t) => index.get_doc_ids_for_term(t),
            Query::Phrase(terms) => {
                if terms.is_empty() {
                    return Vec::new();
                }
                let mut it = terms.iter();
                let first: BTreeSet<u32> = index
                    .get_doc_ids_for_term(it.next().unwrap())
                    .into_iter()
                    .collect();
                let mut acc = first;
                for t in it {
                    let s: BTreeSet<u32> =
                        index.get_doc_ids_for_term(t).into_iter().collect();
                    acc = acc.intersection(&s).copied().collect();
                    if acc.is_empty() {
                        break;
                    }
                }
                acc.into_iter().collect()
            }
            Query::Bool(node) => {
                // must → intersection (start from first sub's set)
                let mut acc: Option<BTreeSet<u32>> = None;
                for sub in &node.must {
                    let s: BTreeSet<u32> = sub.execute(index).into_iter().collect();
                    acc = Some(match acc {
                        Some(prev) => prev.intersection(&s).copied().collect(),
                        None => s,
                    });
                }

                // should → union; only used as the result base when there
                // is no must clause (Lucene BooleanQuery semantics).
                if acc.is_none() {
                    let mut s = BTreeSet::new();
                    for sub in &node.should {
                        for d in sub.execute(index) {
                            s.insert(d);
                        }
                    }
                    acc = Some(s);
                }

                // must_not → subtract
                let mut out = acc.unwrap_or_default();
                for sub in &node.must_not {
                    for d in sub.execute(index) {
                        out.remove(&d);
                    }
                }
                out.into_iter().collect()
            }
        }
    }
}

pub struct BooleanQuery;

impl BooleanQuery {
    pub fn and(subs: Vec<Query>) -> Query {
        Query::Bool(BoolNode {
            must: subs,
            should: Vec::new(),
            must_not: Vec::new(),
        })
    }

    pub fn or(subs: Vec<Query>) -> Query {
        Query::Bool(BoolNode {
            must: Vec::new(),
            should: subs,
            must_not: Vec::new(),
        })
    }

    pub fn not(sub: Query) -> Query {
        Query::Bool(BoolNode {
            must: Vec::new(),
            should: Vec::new(),
            must_not: vec![sub],
        })
    }
}

pub struct PhraseQuery;
