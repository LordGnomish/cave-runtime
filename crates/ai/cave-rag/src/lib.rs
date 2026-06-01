// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-rag — a retrieval-augmented-generation framework for Cave Runtime.
//!
//! Ports the mature, framework-defining patterns from four MIT/Apache-2.0
//! upstreams into one dependency-light, offline-first Rust crate:
//!
//! | concern                    | reference upstream                              |
//! |----------------------------|-------------------------------------------------|
//! | loaders / splitters        | `langchain-ai/langchain` (MIT)                  |
//! | indexing / query engine    | `run-llama/llama_index` (MIT)                   |
//! | knowledge-graph extraction | `microsoft/graphrag` (MIT)                      |
//! | pipeline component model   | `deepset-ai/haystack` (Apache-2.0)              |

#![forbid(unsafe_code)]

pub mod agentic;
pub mod chain;
pub mod citation;
pub mod document;
pub mod embedding;
pub mod error;
pub mod graphrag;
pub mod loaders;
pub mod math;
pub mod rerank;
pub mod retriever;
pub mod splitter;
pub mod vectorstore;

pub use document::{Document, Metadata};
pub use error::{RagError, Result};
