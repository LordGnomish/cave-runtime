// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Inverted index + posting lists.
//!
//! Upstream reference: Lucene `IndexWriter` + `PostingsEnum` (Apache 2.0)
//! and Manticore's `RtIndex_c` real-time segment.  This in-memory
//! implementation is deliberately simple — one `HashMap<term,PostingList>`
//! per index, one `FnvHashMap<doc_id,term_freq>` per posting list — so the
//! rest of cave-search (query, scoring, embeddings) can run unit tests
//! against a real index.  Durability + segments + merges land in a later
//! sweep when on-disk storage is plumbed through `cave-kernel::codec`.

use crate::analyzer::tokenize;
use crate::tenant::TenantId;
use fnv::FnvHashMap;
use std::collections::HashMap;

pub struct Index {
    tenant: TenantId,
    /// term → posting list (doc_id → term-frequency in that doc)
    postings: HashMap<String, PostingList>,
}

impl Index {
    pub fn new(tenant_id: &TenantId) -> Self {
        Index {
            tenant: tenant_id.clone(),
            postings: HashMap::new(),
        }
    }

    pub fn id(&self) -> &str {
        self.tenant.as_str()
    }

    pub fn add_document(&mut self, doc_id: u32, text: &str) {
        // Per-doc term frequencies, then push into postings.
        let mut tf: FnvHashMap<String, u32> = FnvHashMap::default();
        for tok in tokenize(text, &self.tenant) {
            *tf.entry(tok).or_insert(0) += 1;
        }
        for (term, freq) in tf {
            self.postings
                .entry(term)
                .or_default()
                .add_doc(doc_id, freq);
        }
    }

    pub fn delete_document(&mut self, doc_id: u32) {
        // Walk every posting list; drop empty lists so doc_freq() stays honest.
        self.postings.retain(|_, p| {
            p.remove_doc(doc_id);
            p.doc_freq() > 0
        });
    }

    pub fn get_doc_ids_for_term(&self, term: &str) -> Vec<u32> {
        // Query side normalises identically to add-side so callers can
        // pass raw user input ("ALPHA") and still hit lowercased postings.
        let key = term.to_ascii_lowercase();
        match self.postings.get(&key) {
            Some(p) => p.docs.keys().copied().collect(),
            None => Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct PostingList {
    docs: FnvHashMap<u32, u32>,
}

impl PostingList {
    pub fn new() -> Self {
        PostingList::default()
    }

    /// Insert / replace the term-frequency for `doc_id`.  Re-adding the
    /// same doc is last-write-wins (matches Lucene segment-merge semantics
    /// after a delete-and-reindex cycle).
    pub fn add_doc(&mut self, doc_id: u32, term_freq: u32) {
        self.docs.insert(doc_id, term_freq);
    }

    pub fn remove_doc(&mut self, doc_id: u32) {
        self.docs.remove(&doc_id);
    }

    /// Number of distinct docs containing this term.
    pub fn doc_freq(&self) -> u32 {
        self.docs.len() as u32
    }

    /// Sum of per-doc term frequencies — the corpus-wide token count for
    /// this term.  Used by BM25 callers needing collection statistics.
    pub fn total_term_freq(&self) -> u32 {
        self.docs.values().copied().sum()
    }

    /// Term frequency for a specific doc, or 0 if absent.
    pub fn get_doc_freq(&self, doc_id: u32) -> u32 {
        self.docs.get(&doc_id).copied().unwrap_or(0)
    }

    /// Merge several posting lists into one, summing overlapping frequencies
    /// (Lucene `MultiPostingsEnum` / segment-merge semantics).
    pub fn merge(lists: Vec<PostingList>) -> PostingList {
        let mut out = PostingList::default();
        for l in lists {
            for (doc, f) in l.docs {
                *out.docs.entry(doc).or_insert(0) += f;
            }
        }
        out
    }

    pub fn iter(&self) -> std::vec::IntoIter<(u32, u32)> {
        let v: Vec<(u32, u32)> = self.docs.iter().map(|(d, f)| (*d, *f)).collect();
        v.into_iter()
    }
}
