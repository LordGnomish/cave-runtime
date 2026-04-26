//! Query AST: term, boolean, phrase.
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/index/query/

use crate::index::Index;

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
    pub fn execute(&self, _index: &Index) -> Vec<u32> {
        unimplemented!("cave-search::query::Query::execute")
    }
}

pub struct BooleanQuery;

impl BooleanQuery {
    pub fn and(_subs: Vec<Query>) -> Query {
        unimplemented!("cave-search::query::BooleanQuery::and")
    }

    pub fn or(_subs: Vec<Query>) -> Query {
        unimplemented!("cave-search::query::BooleanQuery::or")
    }

    pub fn not(_sub: Query) -> Query {
        unimplemented!("cave-search::query::BooleanQuery::not")
    }
}

pub struct PhraseQuery;
