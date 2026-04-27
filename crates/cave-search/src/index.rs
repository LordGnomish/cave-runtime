//! Inverted index + posting lists.
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/index/

use crate::tenant::TenantId;

pub struct Index {
    _tenant: TenantId,
}

impl Index {
    pub fn new(tenant_id: &TenantId) -> Self {
        Index { _tenant: tenant_id.clone() }
    }

    pub fn id(&self) -> &str {
        unimplemented!("cave-search::index::Index::id")
    }

    pub fn add_document(&mut self, _doc_id: u32, _text: &str) {
        unimplemented!("cave-search::index::Index::add_document")
    }

    pub fn delete_document(&mut self, _doc_id: u32) {
        unimplemented!("cave-search::index::Index::delete_document")
    }

    pub fn get_doc_ids_for_term(&self, _term: &str) -> Vec<u32> {
        unimplemented!("cave-search::index::Index::get_doc_ids_for_term")
    }
}

pub struct PostingList;

impl PostingList {
    pub fn new() -> Self {
        PostingList
    }

    pub fn add_doc(&mut self, _doc_id: u32, _term_freq: u32) {
        unimplemented!("cave-search::index::PostingList::add_doc")
    }

    pub fn remove_doc(&mut self, _doc_id: u32) {
        unimplemented!("cave-search::index::PostingList::remove_doc")
    }

    pub fn doc_freq(&self) -> u32 {
        unimplemented!("cave-search::index::PostingList::doc_freq")
    }

    pub fn total_term_freq(&self) -> u32 {
        unimplemented!("cave-search::index::PostingList::total_term_freq")
    }

    pub fn get_doc_freq(&self, _doc_id: u32) -> u32 {
        unimplemented!("cave-search::index::PostingList::get_doc_freq")
    }

    pub fn merge(_lists: Vec<PostingList>) -> PostingList {
        unimplemented!("cave-search::index::PostingList::merge")
    }

    pub fn iter(&self) -> std::vec::IntoIter<(u32, u32)> {
        unimplemented!("cave-search::index::PostingList::iter")
    }
}

impl Default for PostingList {
    fn default() -> Self {
        Self::new()
    }
}
