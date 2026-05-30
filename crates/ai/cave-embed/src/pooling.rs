// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token embedding pooling strategies.
//!
//! Transformer encoders emit one vector per token; an embedding model collapses
//! the `[seq_len][hidden]` matrix into a single `[hidden]` sentence vector. The
//! reduction is governed by an attention mask (1 = real token, 0 = padding) so
//! pooling never folds in pad positions. This mirrors sentence-transformers'
//! `Pooling` module and the strategies infinity exposes per model.

use std::str::FromStr;

/// Pooling strategy selecting how token vectors collapse to one sentence vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pooling {
    /// Mask-aware arithmetic mean over real tokens (the most common default).
    Mean,
    /// Take the `[CLS]` token vector (row 0) — BERT-style classification head.
    Cls,
    /// Mask-aware element-wise maximum over real tokens.
    Max,
    /// Take the last non-padding token (decoder/causal embedders, e.g. E5-mistral).
    LastToken,
}

/// Error raised when a token matrix cannot be pooled.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PoolError {
    /// The attention mask selected zero real tokens, so there is nothing to pool.
    #[error("no unmasked tokens to pool")]
    EmptyMask,
    /// `tokens` and `mask` lengths disagree.
    #[error("token/mask length mismatch: {tokens} tokens vs {mask} mask entries")]
    LengthMismatch {
        /// Number of token rows supplied.
        tokens: usize,
        /// Number of mask entries supplied.
        mask: usize,
    },
    /// The token matrix has no rows.
    #[error("empty token matrix")]
    EmptyMatrix,
}

impl FromStr for Pooling {
    type Err = PoolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize: lowercase, drop separators so "last_token"/"last-token"/
        // "lasttoken"/"mean_tokens" all resolve.
        let norm: String = s
            .chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match norm.as_str() {
            "mean" | "meantokens" | "avg" | "average" => Ok(Pooling::Mean),
            "cls" | "clstoken" | "first" => Ok(Pooling::Cls),
            "max" | "maxtokens" => Ok(Pooling::Max),
            "lasttoken" | "last" | "eos" => Ok(Pooling::LastToken),
            _ => Err(PoolError::EmptyMask),
        }
    }
}

/// Collapse a `[seq_len][hidden]` token matrix to a `[hidden]` vector.
///
/// `mask[i] == 0` marks `tokens[i]` as padding and excludes it from mask-aware
/// strategies. Returns [`PoolError`] if the inputs disagree in length or select
/// no real tokens.
pub fn pool(strategy: Pooling, tokens: &[Vec<f32>], mask: &[u32]) -> Result<Vec<f32>, PoolError> {
    if tokens.is_empty() {
        return Err(PoolError::EmptyMatrix);
    }
    if tokens.len() != mask.len() {
        return Err(PoolError::LengthMismatch {
            tokens: tokens.len(),
            mask: mask.len(),
        });
    }
    let hidden = tokens[0].len();
    let real: Vec<usize> = (0..tokens.len()).filter(|&i| mask[i] != 0).collect();
    if real.is_empty() {
        return Err(PoolError::EmptyMask);
    }
    let out = match strategy {
        Pooling::Mean => {
            let mut acc = vec![0.0f32; hidden];
            for &i in &real {
                for (a, &x) in acc.iter_mut().zip(tokens[i].iter()) {
                    *a += x;
                }
            }
            let n = real.len() as f32;
            acc.iter().map(|x| x / n).collect()
        }
        Pooling::Cls => {
            // CLS uses row 0 regardless of mask (the prefix token is always real).
            tokens[0].clone()
        }
        Pooling::Max => {
            let mut acc = vec![f32::NEG_INFINITY; hidden];
            for &i in &real {
                for (a, &x) in acc.iter_mut().zip(tokens[i].iter()) {
                    if x > *a {
                        *a = x;
                    }
                }
            }
            acc
        }
        Pooling::LastToken => {
            // Last index whose mask is set.
            let last = *real.last().expect("real non-empty checked above");
            tokens[last].clone()
        }
    };
    Ok(out)
}

/// L2-normalize a vector to unit length. A zero vector is returned unchanged
/// (its norm is 0; dividing would produce NaNs).
pub fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Cosine similarity between two equal-length vectors. Returns 0.0 if either is
/// the zero vector.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}
