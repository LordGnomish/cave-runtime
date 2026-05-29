// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inverted index + posting lists.
//!
//! Implements the core data structures for full-text retrieval:
//! - `PostingList`: maps doc_id → term_frequency for a single term
//! - `Index`: holds the inverted index (term → PostingList mapping)
//!   with per-document length tracking for BM25 scoring and
//!   per-document token sequences for phrase matching.
//!
//! Manticore upstream analog: src/indexer.cpp + src/sphinxsearch.cpp
//! upstream: manticoresoftware/manticoresearch v25.8.2

use crate::analyzer::tokenize;
use crate::tenant::TenantId;
use std::collections::HashMap;

/// Per-document frequency record within a `PostingList`.
#[derive(Debug, Clone)]
struct DocEntry {
    term_freq: u32,
}

/// Posting list for a single term: maps doc_id → term frequency.
pub struct PostingList {
    entries: HashMap<u32, DocEntry>,
    total_tf: u32,
}

impl PostingList {
    /// Create an empty posting list.
    pub fn new() -> Self {
        PostingList { entries: HashMap::new(), total_tf: 0 }
    }

    /// Record that term appears `term_freq` times in `doc_id`.
    /// Replaces the old frequency if `doc_id` is already present.
    pub fn add_doc(&mut self, doc_id: u32, term_freq: u32) {
        if let Some(existing) = self.entries.get_mut(&doc_id) {
            self.total_tf = self.total_tf.saturating_sub(existing.term_freq);
            self.total_tf = self.total_tf.saturating_add(term_freq);
            existing.term_freq = term_freq;
        } else {
            self.entries.insert(doc_id, DocEntry { term_freq });
            self.total_tf = self.total_tf.saturating_add(term_freq);
        }
    }

    /// Remove `doc_id` from this posting list.
    pub fn remove_doc(&mut self, doc_id: u32) {
        if let Some(entry) = self.entries.remove(&doc_id) {
            self.total_tf = self.total_tf.saturating_sub(entry.term_freq);
        }
    }

    /// Number of documents that contain this term.
    pub fn doc_freq(&self) -> u32 {
        self.entries.len() as u32
    }

    /// Sum of all term frequencies across all documents.
    pub fn total_term_freq(&self) -> u32 {
        self.total_tf
    }

    /// Return the term frequency of `doc_id`, or 0 if not present.
    pub fn get_doc_freq(&self, doc_id: u32) -> u32 {
        self.entries.get(&doc_id).map(|e| e.term_freq).unwrap_or(0)
    }

    /// Return all (doc_id, term_freq) pairs sorted by doc_id.
    pub fn iter(&self) -> std::vec::IntoIter<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = self
            .entries
            .iter()
            .map(|(id, e)| (*id, e.term_freq))
            .collect();
        v.sort_by_key(|(id, _)| *id);
        v.into_iter()
    }

    /// Merge multiple posting lists; same doc_id frequencies are summed.
    pub fn merge(lists: Vec<PostingList>) -> PostingList {
        let mut result = PostingList::new();
        for list in lists {
            for (doc_id, tf) in list.iter() {
                let current = result.entries.get(&doc_id).map(|e| e.term_freq).unwrap_or(0);
                result.add_doc(doc_id, current + tf);
            }
        }
        result
    }
}

impl Default for PostingList {
    fn default() -> Self { Self::new() }
}

/// In-memory inverted index for a single tenant namespace.
///
/// Maps term strings → PostingList. Tracks per-document token counts for
/// BM25 average-document-length computation. Stores raw token sequences
/// for phrase matching.
pub struct Index {
    name: String,
    tenant: TenantId,
    postings: HashMap<String, PostingList>,
    doc_lengths: HashMap<u32, u32>,
    doc_tokens: HashMap<u32, Vec<String>>,
}

impl Index {
    /// Create a new empty index with the given `name` under `tenant_id`.
    pub fn new(tenant_id: &TenantId, name: &str) -> Self {
        Index {
            name: name.to_string(),
            tenant: tenant_id.clone(),
            postings: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_tokens: HashMap::new(),
        }
    }

    /// Return the index name.
    pub fn id(&self) -> &str { &self.name }

    /// Return the tenant that owns this index.
    pub fn tenant(&self) -> &TenantId { &self.tenant }

    /// Index `doc_id` with the given `text`. Re-indexing replaces old postings.
    pub fn add_document(&mut self, doc_id: u32, text: &str) {
        self.delete_document(doc_id);

        let tokens = tokenize(text, &self.tenant.clone());
        let token_count = tokens.len() as u32;

        // Build per-term frequencies.
        let mut term_freq: HashMap<String, u32> = HashMap::new();
        for token in &tokens {
            *term_freq.entry(token.clone()).or_insert(0) += 1;
        }

        for (term, tf) in term_freq {
            self.postings
                .entry(term)
                .or_insert_with(PostingList::new)
                .add_doc(doc_id, tf);
        }

        self.doc_tokens.insert(doc_id, tokens);
        self.doc_lengths.insert(doc_id, token_count);
    }

    /// Remove all postings for `doc_id`.
    pub fn delete_document(&mut self, doc_id: u32) {
        if !self.doc_lengths.contains_key(&doc_id) {
            return;
        }
        for pl in self.postings.values_mut() {
            pl.remove_doc(doc_id);
        }
        self.doc_lengths.remove(&doc_id);
        self.doc_tokens.remove(&doc_id);
    }

    /// Return all document IDs sorted ascending.
    pub fn all_doc_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.doc_lengths.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Return candidate doc IDs for a phrase: intersection of posting lists for all terms.
    pub fn phrase_candidates(&self, terms: &[String]) -> Vec<u32> {
        if terms.is_empty() { return Vec::new(); }
        let mut iter = terms.iter().map(|t| {
            self.get_doc_ids_for_term(t)
                .into_iter()
                .collect::<std::collections::HashSet<u32>>()
        });
        let first = iter.next().unwrap_or_default();
        let intersection = iter.fold(first, |acc, set| acc.intersection(&set).copied().collect());
        let mut result: Vec<u32> = intersection.into_iter().collect();
        result.sort_unstable();
        result
    }

    /// Check whether `doc_id` contains `terms` as a contiguous ordered phrase.
    pub fn check_phrase(&self, doc_id: u32, terms: &[String]) -> bool {
        let doc_toks = match self.doc_tokens.get(&doc_id) {
            Some(t) => t,
            None => return false,
        };
        if terms.is_empty() || terms.len() > doc_toks.len() { return false; }
        let first = terms[0].to_lowercase();
        doc_toks
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == first)
            .any(|(i, _)| {
                if i + terms.len() > doc_toks.len() { return false; }
                terms.iter().enumerate().all(|(j, term)| {
                    doc_toks[i + j].to_lowercase() == term.to_lowercase()
                })
            })
    }

    /// Return doc IDs that contain `term` (after normalization).
    pub fn get_doc_ids_for_term(&self, term: &str) -> Vec<u32> {
        let normalized = term.to_lowercase();
        match self.postings.get(&normalized) {
            Some(pl) => pl.iter().map(|(id, _)| id).collect(),
            None => Vec::new(),
        }
    }

    /// Return the posting list for `term`, if it exists.
    pub fn get_posting_list(&self, term: &str) -> Option<&PostingList> {
        let normalized = term.to_lowercase();
        self.postings.get(&normalized)
    }

    /// Number of live documents in this index.
    pub fn doc_count(&self) -> usize { self.doc_lengths.len() }

    /// Average document length (in tokens). Returns 0.0 if empty.
    pub fn avg_doc_len(&self) -> f64 {
        if self.doc_lengths.is_empty() { return 0.0; }
        let total: u32 = self.doc_lengths.values().sum();
        total as f64 / self.doc_lengths.len() as f64
    }

    /// Return the token length for `doc_id`, or 0 if not found.
    pub fn doc_len(&self, doc_id: u32) -> u32 {
        *self.doc_lengths.get(&doc_id).unwrap_or(&0)
    }
}
