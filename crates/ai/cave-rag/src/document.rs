// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The unit of currency in a RAG pipeline: a [`Document`] (a chunk of text
//! plus [`Metadata`]).
//!
//! Mirrors `langchain_core.documents.Document` and llama_index's `TextNode`:
//! a content string, a free-form metadata bag, and a content-addressed id.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Free-form, source-tracking metadata attached to a [`Document`].
///
/// `source` is promoted to a named field because every loader sets it and
/// citation tracking keys off it; everything else lives in `extra`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Origin of the document (file path, URL, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Arbitrary string key/value pairs (page number, section, language, …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl Metadata {
    /// Set a custom metadata field.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.extra.insert(key.into(), value.into());
    }

    /// Read a custom metadata field.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.extra.get(key).map(String::as_str)
    }
}

/// A chunk of text plus its provenance metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// The text payload.
    pub content: String,
    /// Provenance / annotation metadata.
    pub metadata: Metadata,
}

impl Document {
    /// Construct a document from raw content with empty metadata.
    pub fn new(content: impl Into<String>) -> Self {
        Document {
            content: content.into(),
            metadata: Metadata::default(),
        }
    }

    /// Builder: stamp the `source` metadata field.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.metadata.source = Some(source.into());
        self
    }

    /// Builder: set a custom metadata field.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.set(key, value);
        self
    }

    /// Content-addressed id: the SHA-256 hex digest of the content.
    ///
    /// Stable across runs, so dedup / incremental-index logic can key off it
    /// (the langchain `RecordManager` / llama_index `hash` pattern).
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.content.as_bytes());
        format!("{:x}", h.finalize())
    }
}
