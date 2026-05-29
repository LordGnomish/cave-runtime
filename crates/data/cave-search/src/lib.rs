// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-search: full-text + semantic search engine (skeleton — impl pending).
//!
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/

pub mod analyzer;
pub mod embeddings;
pub mod index;
pub mod models;
pub mod query;
pub mod scoring;
pub mod tenant;
