// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multimodal (CLIP) modality routing.
//!
//! infinity serves CLIP-style models that embed both text and images into a
//! single shared space (so a text query can be matched against image vectors).
//! We model the **routing + shared-space contract**: each input is classified
//! as text or image and dispatched to the matching branch, both producing
//! vectors of identical dimensionality. Actual image decoding / a real vision
//! tower is a scope-cut; the image branch hashes the raw bytes deterministically
//! as a faithful stand-in.

use crate::backend::{self, EmbeddingBackend, HashingEmbedder};
use crate::error::{EmbedError, EmbedResult};
use crate::pooling::{self, PoolingStrategy};
use serde::{Deserialize, Serialize};

/// Modality of a CLIP input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    /// Text branch.
    Text,
    /// Image branch.
    Image,
}

/// A single CLIP input.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum ClipInput {
    /// A text string.
    Text(String),
    /// Raw image bytes.
    ImageBytes(Vec<u8>),
    /// An image URL (bytes are not fetched here — see module docs).
    ImageUrl(String),
}

/// Classify an input's modality.
pub fn classify_modality(input: &ClipInput) -> Modality {
    // PLACEHOLDER (RED): everything is text.
    let _ = input;
    Modality::Text
}

/// CLIP embedder — a text tower and an image tower sharing one dimensionality.
pub struct ClipEmbedder {
    dims: usize,
    text_tower: HashingEmbedder,
}

impl ClipEmbedder {
    /// Build a CLIP embedder with the given shared dimensionality.
    pub fn new(dims: usize) -> Self {
        Self {
            dims,
            text_tower: HashingEmbedder::new("clip-text", dims),
        }
    }

    /// Shared-space dimensionality.
    pub fn dims(&self) -> usize {
        self.dims
    }

    /// Deterministic image-tower embedding from raw bytes.
    fn image_embedding(&self, bytes: &[u8]) -> EmbedResult<Vec<f32>> {
        if bytes.is_empty() {
            return Err(EmbedError::EmptyInput);
        }
        // FNV-1a over the bytes seeds a SplitMix64 stream of `dims` values.
        let mut state: u64 = 0xcbf29ce484222325;
        for b in bytes {
            state ^= *b as u64;
            state = state.wrapping_mul(0x100000001b3);
        }
        let mut v = Vec::with_capacity(self.dims);
        for _ in 0..self.dims {
            state = state.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^= z >> 31;
            let unit = (z >> 11) as f32 / (1u64 << 53) as f32;
            v.push(unit * 2.0 - 1.0);
        }
        pooling::l2_normalize(&v)
    }

    /// Embed a single input into the shared space.
    pub async fn embed(&self, input: &ClipInput) -> EmbedResult<Vec<f32>> {
        // PLACEHOLDER (RED): empty vector regardless of modality.
        let _ = input;
        Ok(Vec::new())
    }

    /// Embed a batch of mixed-modality inputs.
    pub async fn embed_batch(&self, inputs: &[ClipInput]) -> EmbedResult<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Err(EmbedError::EmptyInput);
        }
        let mut out = Vec::with_capacity(inputs.len());
        for i in inputs {
            out.push(self.embed(i).await?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modality_routing() {
        assert_eq!(classify_modality(&ClipInput::Text("hi".into())), Modality::Text);
        assert_eq!(
            classify_modality(&ClipInput::ImageBytes(vec![1, 2, 3])),
            Modality::Image
        );
        assert_eq!(
            classify_modality(&ClipInput::ImageUrl("http://x/y.png".into())),
            Modality::Image
        );
    }

    #[tokio::test]
    async fn text_and_image_share_dimensionality() {
        let clip = ClipEmbedder::new(128);
        let t = clip.embed(&ClipInput::Text("a red apple".into())).await.unwrap();
        let i = clip
            .embed(&ClipInput::ImageBytes(vec![9, 8, 7, 6, 5]))
            .await
            .unwrap();
        assert_eq!(t.len(), 128);
        assert_eq!(i.len(), 128, "image tower shares text dims");
    }

    #[tokio::test]
    async fn embeddings_are_unit_length() {
        let clip = ClipEmbedder::new(64);
        let t = clip.embed(&ClipInput::Text("normalize".into())).await.unwrap();
        let n: f32 = t.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-4);
    }

    #[tokio::test]
    async fn deterministic_per_modality() {
        let clip = ClipEmbedder::new(32);
        let a = clip.embed(&ClipInput::ImageBytes(vec![1, 2, 3])).await.unwrap();
        let b = clip.embed(&ClipInput::ImageBytes(vec![1, 2, 3])).await.unwrap();
        assert_eq!(a, b);
        let c = clip.embed(&ClipInput::ImageBytes(vec![4, 5, 6])).await.unwrap();
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn empty_image_bytes_errors() {
        let clip = ClipEmbedder::new(32);
        assert!(matches!(
            clip.embed(&ClipInput::ImageBytes(vec![])).await,
            Err(EmbedError::EmptyInput)
        ));
    }

    #[tokio::test]
    async fn batch_returns_one_vector_per_input() {
        let clip = ClipEmbedder::new(16);
        let out = clip
            .embed_batch(&[
                ClipInput::Text("cat".into()),
                ClipInput::ImageBytes(vec![1, 1, 1]),
                ClipInput::Text("dog".into()),
            ])
            .await
            .unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|v| v.len() == 16));
    }
}
