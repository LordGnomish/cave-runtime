// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lightweight word tokenizer for token accounting and context truncation.
//!
//! A real serving deployment plugs a model's own subword tokenizer in through
//! the backend; this dependency-free word tokenizer gives the batcher and the
//! `usage.total_tokens` accounting a deterministic, model-agnostic length
//! signal and lets the registry's `max_seq_len` truncate over-long inputs. It
//! lowercases and splits on any non-alphanumeric boundary (matching the
//! whitespace/punctuation segmentation the reference [`crate::backend`] uses).

/// Split text into lowercase alphanumeric tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Number of tokens in `text`.
pub fn count_tokens(text: &str) -> usize {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .count()
}

/// Truncate `text` to at most `max_tokens` tokens, preserving the original
/// surface tokens (not the lowercased form) joined by single spaces. A no-op
/// when already within the limit.
pub fn truncate(text: &str, max_tokens: usize) -> String {
    let toks: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if toks.len() <= max_tokens {
        return text.to_string();
    }
    toks[..max_tokens].join(" ")
}
