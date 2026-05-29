// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Search models: stub.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType { Text, Keyword, Integer, Float, Bool, Date, Vector }

#[derive(Debug, Clone)]
pub struct FieldMapping { pub name: String, pub field_type: FieldType }

#[derive(Debug, Clone)]
pub struct IndexSchema { name: String, fields: Vec<FieldMapping> }

impl IndexSchema {
    pub fn new(name: &str) -> Self { IndexSchema { name: name.to_string(), fields: Vec::new() } }
    pub fn name(&self) -> &str { &self.name }
    pub fn add_field(&mut self, _name: &str, _ft: FieldType) {}
    pub fn field_type(&self, _name: &str) -> Option<FieldType> { None }
    pub fn field_names(&self) -> Vec<&str> { Vec::new() }
    pub fn fields(&self) -> &[FieldMapping] { &self.fields }
}

#[derive(Debug, Clone)]
pub struct Document { id: u32, fields: HashMap<String, String> }

impl Document {
    pub fn new(id: u32) -> Self { Document { id, fields: HashMap::new() } }
    pub fn id(&self) -> u32 { self.id }
    pub fn set_field(&mut self, _name: &str, _value: &str) {}
    pub fn get_field(&self, _name: &str) -> Option<&str> { None }
    pub fn fields(&self) -> &HashMap<String, String> { &self.fields }
}

#[derive(Debug, Clone)]
pub struct SearchRequest { query_text: String, index: Option<String>, from: usize, size: usize }

impl SearchRequest {
    pub fn new(query_text: &str) -> Self { SearchRequest { query_text: query_text.to_string(), index: None, from: 0, size: 10 } }
    pub fn with_index(mut self, index: &str) -> Self { self.index = Some(index.to_string()); self }
    pub fn with_from(mut self, from: usize) -> Self { self.from = from; self }
    pub fn with_size(mut self, size: usize) -> Self { self.size = size; self }
    pub fn query_text(&self) -> &str { &self.query_text }
    pub fn index(&self) -> Option<&str> { self.index.as_deref() }
    pub fn from(&self) -> usize { self.from }
    pub fn size(&self) -> usize { self.size }
}

#[derive(Debug, Clone)]
pub struct HitDocument { pub doc_id: u32, pub score: f64, pub fields: HashMap<String, String> }

#[derive(Debug, Clone)]
pub struct SearchResponse { hits: Vec<HitDocument>, total: u64 }

impl SearchResponse {
    pub fn new(hits: Vec<HitDocument>, total: u64) -> Self { SearchResponse { hits, total } }
    pub fn total(&self) -> u64 { self.total }
    pub fn hits(&self) -> &[HitDocument] { &self.hits }
}

#[derive(Debug, Clone)]
pub struct HighlightSnippet { pub fragment: String, pub offset: usize }

pub fn highlight_field(_text: &str, _term: &str, _pre: &str, _post: &str) -> Vec<HighlightSnippet> { Vec::new() }
pub fn paginate<T: Clone>(_items: &[T], _from: usize, _size: usize) -> Vec<T> { Vec::new() }
