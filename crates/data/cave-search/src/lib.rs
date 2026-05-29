// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-search: full-text + semantic search engine.
//!
//! upstream: manticoresoftware/manticoresearch v25.8.2
//!   https://github.com/manticoresoftware/manticoresearch

pub mod analyzer;
pub mod embeddings;
pub mod index;
pub mod models;
pub mod query;
pub mod scoring;
pub mod tenant;
