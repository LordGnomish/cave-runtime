// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type shared across cave-embed.

/// Errors raised by the embedding/rerank engine and its OpenAI-compatible layer.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbedError {
    /// The requested model id is not present in the catalog / backend registry.
    #[error("unknown model: {0}")]
    UnknownModel(String),

    /// The request carried no input to embed.
    #[error("empty input")]
    EmptyInput,

    /// A requested output dimensionality is incompatible with the model.
    #[error("invalid dimensions: requested {requested}, model native {native}")]
    InvalidDimensions { requested: usize, native: usize },

    /// Token-embedding matrix and attention mask disagree on length.
    #[error("shape mismatch: tokens={tokens}, mask={mask}")]
    ShapeMismatch { tokens: usize, mask: usize },

    /// A pooling/normalize operation received a degenerate (all-zero / empty) input.
    #[error("degenerate vector: {0}")]
    Degenerate(String),

    /// Generic invalid-argument error with a human-readable message.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

/// Convenience result alias.
pub type EmbedResult<T> = Result<T, EmbedError>;
