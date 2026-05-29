// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inverted index: stub.

use crate::tenant::TenantId;
use std::collections::HashMap;

pub struct PostingList;
pub struct Index { name: String, tenant: TenantId, _ph: HashMap<(),()> }

impl PostingList {
    pub fn new() -> Self { PostingList }
    pub fn add_doc(&mut self, _doc_id: u32, _tf: u32) {}
    pub fn remove_doc(&mut self, _doc_id: u32) {}
    pub fn doc_freq(&self) -> u32 { 0 }
    pub fn total_term_freq(&self) -> u32 { 0 }
    pub fn get_doc_freq(&self, _doc_id: u32) -> u32 { 0 }
    pub fn iter(&self) -> std::vec::IntoIter<(u32, u32)> { Vec::new().into_iter() }
    pub fn merge(_lists: Vec<PostingList>) -> PostingList { PostingList }
}

impl Default for PostingList { fn default() -> Self { Self::new() } }

impl Index {
    pub fn new(tenant_id: &TenantId, name: &str) -> Self {
        Index { name: name.to_string(), tenant: tenant_id.clone(), _ph: HashMap::new() }
    }
    pub fn id(&self) -> &str { &self.name }
    pub fn tenant(&self) -> &TenantId { &self.tenant }
    pub fn add_document(&mut self, _doc_id: u32, _text: &str) {}
    pub fn delete_document(&mut self, _doc_id: u32) {}
    pub fn all_doc_ids(&self) -> Vec<u32> { Vec::new() }
    pub fn phrase_candidates(&self, _terms: &[String]) -> Vec<u32> { Vec::new() }
    pub fn check_phrase(&self, _doc_id: u32, _terms: &[String]) -> bool { false }
    pub fn get_doc_ids_for_term(&self, _term: &str) -> Vec<u32> { Vec::new() }
    pub fn get_posting_list(&self, _term: &str) -> Option<&PostingList> { None }
    pub fn doc_count(&self) -> usize { 0 }
    pub fn avg_doc_len(&self) -> f64 { 0.0 }
    pub fn doc_len(&self, _doc_id: u32) -> u32 { 0 }
}
