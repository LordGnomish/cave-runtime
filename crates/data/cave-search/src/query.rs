// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query engine: stub.

use crate::index::Index;

pub enum Query { Term(String), Phrase(Vec<String>), Bool(BoolNode) }
pub struct BoolNode { pub must: Vec<Query>, pub should: Vec<Query>, pub must_not: Vec<Query> }

impl Query {
    pub fn execute(&self, _index: &Index) -> Vec<u32> { Vec::new() }
}

pub struct BooleanQuery;
impl BooleanQuery {
    pub fn and(subs: Vec<Query>) -> Query { Query::Bool(BoolNode { must: subs, should: vec![], must_not: vec![] }) }
    pub fn or(subs: Vec<Query>) -> Query { Query::Bool(BoolNode { must: vec![], should: subs, must_not: vec![] }) }
    pub fn not(sub: Query) -> Query { Query::Bool(BoolNode { must: vec![], should: vec![], must_not: vec![sub] }) }
}

pub struct PhraseQuery;
impl PhraseQuery {
    pub fn of(terms: Vec<String>) -> Query { Query::Phrase(terms) }
}
