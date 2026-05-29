// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Search request/response models, field schema, document store, and highlighting.
//!
//! Implements the core API surface:
//! - `IndexSchema` / `FieldMapping` — typed field definitions (text/int/float/bool/keyword)
//! - `Document`       — a stored document with typed field values
//! - `SearchRequest`  — query parameters (query text, index name, pagination, filters)
//! - `SearchResponse` / `HitDocument` — ranked result set
//! - `highlight_field` — term highlighting in text snippets
//! - `paginate`        — generic slice windowing helper
//!
//! Manticore upstream analog: src/sphinxquery.h + src/searchdaemon.h
//! upstream: manticoresoftware/manticoresearch 25.8.2

use std::collections::HashMap;

// ── Field schema ─────────────────────────────────────────────────────────────

/// Supported field types in the index schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// Full-text analysed field.
    Text,
    /// Exact-match string (no analysis).
    Keyword,
    /// 64-bit signed integer.
    Integer,
    /// 64-bit IEEE 754 float.
    Float,
    /// Boolean.
    Bool,
    /// Date/time stored as ISO-8601 string.
    Date,
    /// Dense float vector (for KNN / semantic search).
    Vector,
}

/// A single field definition within an `IndexSchema`.
#[derive(Debug, Clone)]
pub struct FieldMapping {
    pub name: String,
    pub field_type: FieldType,
}

/// Schema for one index: a named collection of field mappings.
#[derive(Debug, Clone)]
pub struct IndexSchema {
    name: String,
    fields: Vec<FieldMapping>,
}

impl IndexSchema {
    /// Create a new schema with the given index name and no fields.
    pub fn new(name: &str) -> Self {
        IndexSchema {
            name: name.to_string(),
            fields: Vec::new(),
        }
    }

    /// Return the index name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Add a field to the schema.
    pub fn add_field(&mut self, field_name: &str, field_type: FieldType) {
        // Replace if already present.
        if let Some(f) = self.fields.iter_mut().find(|f| f.name == field_name) {
            f.field_type = field_type;
        } else {
            self.fields.push(FieldMapping {
                name: field_name.to_string(),
                field_type,
            });
        }
    }

    /// Return the `FieldType` for `field_name`, or `None` if not found.
    pub fn field_type(&self, field_name: &str) -> Option<FieldType> {
        self.fields
            .iter()
            .find(|f| f.name == field_name)
            .map(|f| f.field_type)
    }

    /// Return all field names in definition order.
    pub fn field_names(&self) -> Vec<&str> {
        self.fields.iter().map(|f| f.name.as_str()).collect()
    }

    /// Return all field mappings.
    pub fn fields(&self) -> &[FieldMapping] {
        &self.fields
    }
}

// ── Document ──────────────────────────────────────────────────────────────────

/// A stored document: an ID + a string-valued field map.
///
/// In production, field values would carry typed variants; for the MVP
/// we use `String` for all field values (matching common search-engine
/// JSON/string serialization pattern).
#[derive(Debug, Clone)]
pub struct Document {
    id: u32,
    fields: HashMap<String, String>,
}

impl Document {
    /// Create a new document with the given `id`.
    pub fn new(id: u32) -> Self {
        Document {
            id,
            fields: HashMap::new(),
        }
    }

    /// Return the document ID.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Set a string field value.
    pub fn set_field(&mut self, name: &str, value: &str) {
        self.fields.insert(name.to_string(), value.to_string());
    }

    /// Get the value of a field, or `None` if not present.
    pub fn get_field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(|s| s.as_str())
    }

    /// Return all field entries.
    pub fn fields(&self) -> &HashMap<String, String> {
        &self.fields
    }
}

// ── SearchRequest ─────────────────────────────────────────────────────────────

/// A search query request.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    query_text: String,
    index: Option<String>,
    from: usize,
    size: usize,
}

impl SearchRequest {
    /// Create a new search request for `query_text` with default pagination.
    pub fn new(query_text: &str) -> Self {
        SearchRequest {
            query_text: query_text.to_string(),
            index: None,
            from: 0,
            size: 10,
        }
    }

    /// Set the target index name.
    pub fn with_index(mut self, index: &str) -> Self {
        self.index = Some(index.to_string());
        self
    }

    /// Set the pagination offset (skip first `from` results).
    pub fn with_from(mut self, from: usize) -> Self {
        self.from = from;
        self
    }

    /// Set the maximum number of results to return.
    pub fn with_size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    /// Return the query text.
    pub fn query_text(&self) -> &str {
        &self.query_text
    }

    /// Return the target index name, if set.
    pub fn index(&self) -> Option<&str> {
        self.index.as_deref()
    }

    /// Return the pagination offset.
    pub fn from(&self) -> usize {
        self.from
    }

    /// Return the page size.
    pub fn size(&self) -> usize {
        self.size
    }
}

// ── SearchResponse / HitDocument ─────────────────────────────────────────────

/// One document hit in a search response.
#[derive(Debug, Clone)]
pub struct HitDocument {
    pub doc_id: u32,
    pub score: f64,
    pub fields: HashMap<String, String>,
}

/// A ranked search response.
#[derive(Debug, Clone)]
pub struct SearchResponse {
    hits: Vec<HitDocument>,
    total: u64,
}

impl SearchResponse {
    /// Create a search response with the given hits and total count.
    pub fn new(hits: Vec<HitDocument>, total: u64) -> Self {
        SearchResponse { hits, total }
    }

    /// Return the total number of matching documents (may exceed `hits.len()`
    /// when pagination is applied).
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Return the hits for this page.
    pub fn hits(&self) -> &[HitDocument] {
        &self.hits
    }
}

// ── Highlighting ──────────────────────────────────────────────────────────────

/// A highlighted text snippet.
#[derive(Debug, Clone)]
pub struct HighlightSnippet {
    /// The text fragment (with highlight markup applied).
    pub fragment: String,
    /// Byte offset of the fragment start in the source field.
    pub offset: usize,
}

/// Highlight occurrences of `term` in `text` by wrapping matches with
/// `pre_tag` and `post_tag`.
///
/// Returns one snippet per window of text around each match.
/// If `term` is not found, returns one snippet containing the raw text
/// with no markup.
pub fn highlight_field(text: &str, term: &str, pre_tag: &str, post_tag: &str) -> Vec<HighlightSnippet> {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();

    let mut snippets = Vec::new();
    let mut search_start = 0usize;
    let mut found = false;

    while let Some(pos) = lower_text[search_start..].find(&lower_term) {
        let abs_pos = search_start + pos;
        // Extract a window of ±50 chars around the match for context.
        let window_start = abs_pos.saturating_sub(50);
        let window_end = (abs_pos + lower_term.len() + 50).min(text.len());

        let before = &text[window_start..abs_pos];
        let matched = &text[abs_pos..abs_pos + lower_term.len()];
        let after = &text[abs_pos + lower_term.len()..window_end];

        snippets.push(HighlightSnippet {
            fragment: format!("{}{}{}{}{}", before, pre_tag, matched, post_tag, after),
            offset: window_start,
        });

        search_start = abs_pos + lower_term.len();
        found = true;

        if search_start >= text.len() {
            break;
        }
    }

    if !found {
        // Return the original text (up to 200 chars) as a no-match snippet.
        let fragment = if text.len() > 200 {
            text[..200].to_string()
        } else {
            text.to_string()
        };
        snippets.push(HighlightSnippet { fragment, offset: 0 });
    }

    snippets
}

// ── Pagination ────────────────────────────────────────────────────────────────

/// Return a slice of `items` starting at `from` with at most `size` elements.
pub fn paginate<T: Clone>(items: &[T], from: usize, size: usize) -> Vec<T> {
    if from >= items.len() {
        return Vec::new();
    }
    let end = (from + size).min(items.len());
    items[from..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_field_type_lookup() {
        let mut s = IndexSchema::new("test");
        s.add_field("foo", FieldType::Text);
        assert_eq!(s.field_type("foo"), Some(FieldType::Text));
        assert_eq!(s.field_type("bar"), None);
    }

    #[test]
    fn document_get_set() {
        let mut d = Document::new(1);
        d.set_field("k", "v");
        assert_eq!(d.get_field("k"), Some("v"));
    }

    #[test]
    fn paginate_basic() {
        let v: Vec<u32> = (0..10).collect();
        assert_eq!(paginate(&v, 2, 3), vec![2, 3, 4]);
    }

    #[test]
    fn highlight_found() {
        let s = highlight_field("hello world", "world", "<b>", "</b>");
        assert!(s.iter().any(|h| h.fragment.contains("<b>world</b>")));
    }

    #[test]
    fn search_request_defaults() {
        let r = SearchRequest::new("q");
        assert_eq!(r.from(), 0);
        assert_eq!(r.size(), 10);
    }
}
