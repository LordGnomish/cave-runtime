// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token-embedding pooling — sentence-transformers `Pooling` module.
//!
//! Upstream: `sentence_transformers/models/Pooling.py`. Given a transformer's
//! per-token hidden states and the attention mask, collapse them into a single
//! sentence embedding. We implement the common modes plus L2 normalization
//! (`sentence_transformers/models/Normalize.py`).

use crate::error::{EmbedError, EmbedResult};

/// Pooling mode applied to token embeddings to form a sentence embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolingStrategy {
    /// Masked mean over tokens (`pooling_mode_mean_tokens`). The default.
    Mean,
    /// First token (`pooling_mode_cls_token`) — the `[CLS]`/`<s>` position.
    Cls,
    /// Elementwise max over masked tokens (`pooling_mode_max_tokens`).
    Max,
    /// Last non-padding token (`pooling_mode_lasttoken`) — used by E5/Mistral
    /// decoder-style embedders.
    LastToken,
    /// Mean divided by sqrt(token count) (`pooling_mode_mean_sqrt_len_tokens`).
    MeanSqrtLen,
}

impl PoolingStrategy {
    /// Parse a sentence-transformers-style mode name.
    pub fn from_str(s: &str) -> EmbedResult<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mean" | "mean_tokens" | "pooling_mode_mean_tokens" => Ok(Self::Mean),
            "cls" | "cls_token" | "pooling_mode_cls_token" => Ok(Self::Cls),
            "max" | "max_tokens" | "pooling_mode_max_tokens" => Ok(Self::Max),
            "last" | "lasttoken" | "last_token" | "pooling_mode_lasttoken" => Ok(Self::LastToken),
            "mean_sqrt_len" | "pooling_mode_mean_sqrt_len_tokens" => Ok(Self::MeanSqrtLen),
            other => Err(EmbedError::InvalidArgument(format!(
                "unknown pooling mode: {other}"
            ))),
        }
    }
}

/// Pool a `[seq_len][hidden]` token-embedding matrix into a `[hidden]` vector
/// using `mask` (1 = real token, 0 = padding).
pub fn pool(
    strategy: PoolingStrategy,
    token_embeddings: &[Vec<f32>],
    mask: &[u32],
) -> EmbedResult<Vec<f32>> {
    if token_embeddings.is_empty() {
        return Err(EmbedError::EmptyInput);
    }
    if token_embeddings.len() != mask.len() {
        return Err(EmbedError::ShapeMismatch {
            tokens: token_embeddings.len(),
            mask: mask.len(),
        });
    }
    // PLACEHOLDER (RED): return the first token regardless of strategy.
    let _ = strategy;
    Ok(token_embeddings[0].clone())
}

/// L2-normalize a vector in place-style (returns a new vector). Returns
/// `Degenerate` if the norm is ~zero.
pub fn l2_normalize(v: &[f32]) -> EmbedResult<Vec<f32>> {
    // PLACEHOLDER (RED): identity, no normalization.
    Ok(v.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks() -> Vec<Vec<f32>> {
        vec![
            vec![1.0, 0.0, 0.0],
            vec![3.0, 4.0, 0.0],
            vec![0.0, 0.0, 9.0], // padding
        ]
    }
    fn mask() -> Vec<u32> {
        vec![1, 1, 0]
    }

    #[test]
    fn mean_excludes_padding() {
        let out = pool(PoolingStrategy::Mean, &toks(), &mask()).unwrap();
        // mean of rows 0,1 = ([1+3]/2, [0+4]/2, 0) = (2,2,0)
        assert_eq!(out, vec![2.0, 2.0, 0.0]);
    }

    #[test]
    fn cls_is_first_token() {
        let out = pool(PoolingStrategy::Cls, &toks(), &mask()).unwrap();
        assert_eq!(out, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn max_excludes_padding() {
        let out = pool(PoolingStrategy::Max, &toks(), &mask()).unwrap();
        // elementwise max of rows 0,1 = (3,4,0); padding row's 9.0 ignored
        assert_eq!(out, vec![3.0, 4.0, 0.0]);
    }

    #[test]
    fn last_token_is_last_unmasked() {
        let out = pool(PoolingStrategy::LastToken, &toks(), &mask()).unwrap();
        // last mask==1 is row index 1
        assert_eq!(out, vec![3.0, 4.0, 0.0]);
    }

    #[test]
    fn mean_sqrt_len_divides_by_sqrt_count() {
        let out = pool(PoolingStrategy::MeanSqrtLen, &toks(), &mask()).unwrap();
        // sum=(4,4,0) / sqrt(2)
        let s = 2.0_f32.sqrt();
        assert!((out[0] - 4.0 / s).abs() < 1e-6);
        assert!((out[1] - 4.0 / s).abs() < 1e-6);
        assert_eq!(out[2], 0.0);
    }

    #[test]
    fn shape_mismatch_errors() {
        let err = pool(PoolingStrategy::Mean, &toks(), &[1, 1]).unwrap_err();
        assert!(matches!(err, EmbedError::ShapeMismatch { .. }));
    }

    #[test]
    fn normalize_unit_length() {
        let out = l2_normalize(&[3.0, 4.0]).unwrap();
        assert!((out[0] - 0.6).abs() < 1e-6);
        assert!((out[1] - 0.8).abs() < 1e-6);
        let norm = (out[0] * out[0] + out[1] * out[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_zero_is_degenerate() {
        assert!(matches!(
            l2_normalize(&[0.0, 0.0]),
            Err(EmbedError::Degenerate(_))
        ));
    }

    #[test]
    fn parse_modes() {
        assert_eq!(PoolingStrategy::from_str("mean").unwrap(), PoolingStrategy::Mean);
        assert_eq!(PoolingStrategy::from_str("CLS").unwrap(), PoolingStrategy::Cls);
        assert!(PoolingStrategy::from_str("bogus").is_err());
    }
}
