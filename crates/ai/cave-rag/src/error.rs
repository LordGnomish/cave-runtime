// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type shared across the cave-rag pipeline.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, RagError>;

/// Errors raised by the RAG pipeline.
#[derive(Debug, Error)]
pub enum RagError {
    /// A document loader failed (unreadable file, unsupported format, …).
    #[error("loader error: {0}")]
    Loader(String),

    /// A text splitter received an invalid configuration.
    #[error("splitter error: {0}")]
    Splitter(String),

    /// An embedding backend failed to produce a vector.
    #[error("embedding error: {0}")]
    Embedding(String),

    /// A vector-store operation failed.
    #[error("vector store error: {0}")]
    VectorStore(String),

    /// A retriever could not satisfy a query.
    #[error("retriever error: {0}")]
    Retriever(String),

    /// A reranker failed.
    #[error("rerank error: {0}")]
    Rerank(String),

    /// The generation step failed.
    #[error("generation error: {0}")]
    Generation(String),

    /// Knowledge-graph extraction / traversal failed.
    #[error("graph error: {0}")]
    Graph(String),

    /// Underlying I/O failure.
    #[error("io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for RagError {
    fn from(e: std::io::Error) -> Self {
        RagError::Io(e.to_string())
    }
}
