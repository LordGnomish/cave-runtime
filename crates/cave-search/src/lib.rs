// SPDX-License-Identifier: AGPL-3.0-or-later

//! cave-search: full-text + semantic search engine.
//!
//! In-memory inverted index (`index`) with BM25 scoring (`scoring`),
//! lowercase analyser + English stop-word filter (`analyzer`), and a
//! deterministic hashed embedding for semantic similarity (`embeddings`).
//! `query::Query` exposes a term / phrase / boolean AST executed set-wise
//! against the index.
//!
//! upstream: manticoresoftware/manticoresearch 25.8.2 (full-text + analytics)
//! plus opensearch v3.0/server/src/main/java/org/opensearch/ for the
//! Lucene-derived `Similarity` / posting-list shape.

pub mod analyzer;
pub mod embeddings;
pub mod index;
pub mod query;
pub mod scoring;
pub mod tenant;
