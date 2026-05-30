// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 4 — embedding backend trait + reference HashEmbedder.
//
// A concrete weight-loading backend (ONNX/candle/burn) plugs in through the
// EmbeddingBackend trait. The crate ships a deterministic, dependency-free
// reference embedder so the whole pipeline is exercisable without model
// weights: tokenize → per-token signed feature-hash vectors → pool → normalize.
// It is a genuine bag-of-words embedding (shared vocabulary ⇒ higher cosine),
// not a placeholder.

use cave_embed::backend::{BackendRegistry, EmbeddingBackend, HashEmbedder};
use cave_embed::pooling::{cosine, Pooling};
use cave_embed::registry::ModelCard;

fn card(dim: usize) -> ModelCard {
    ModelCard::text("test/model", dim, 256, Pooling::Mean, true)
}

#[test]
fn output_matches_card_dimensions() {
    let be = HashEmbedder::new();
    let out = be.embed(&["hello world".into()], &card(384)).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), 384);
}

#[test]
fn deterministic() {
    let be = HashEmbedder::new();
    let a = be.embed(&["the quick brown fox".into()], &card(128)).unwrap();
    let b = be.embed(&["the quick brown fox".into()], &card(128)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn normalized_when_card_requests() {
    let be = HashEmbedder::new();
    let out = be.embed(&["alpha beta gamma".into()], &card(64)).unwrap();
    let mag: f32 = out[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((mag - 1.0).abs() < 1e-5, "expected unit length, got {mag}");
}

#[test]
fn not_normalized_when_card_disables() {
    let be = HashEmbedder::new();
    let c = ModelCard::text("t", 64, 256, Pooling::Mean, false);
    let out = be.embed(&["alpha beta gamma".into()], &c).unwrap();
    let mag: f32 = out[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    // raw pooled vector is essentially never exactly unit length.
    assert!((mag - 1.0).abs() > 1e-3);
}

#[test]
fn shared_vocabulary_is_more_similar() {
    let be = HashEmbedder::new();
    let c = card(256);
    let q = be.embed(&["machine learning models".into()], &c).unwrap()[0].clone();
    let near = be.embed(&["learning machine systems".into()], &c).unwrap()[0].clone();
    let far = be.embed(&["banana orange fruit".into()], &c).unwrap()[0].clone();
    assert!(
        cosine(&q, &near) > cosine(&q, &far),
        "shared-token doc must rank above disjoint doc"
    );
}

#[test]
fn empty_text_yields_zero_vector() {
    let be = HashEmbedder::new();
    let out = be.embed(&["".into()], &card(32)).unwrap();
    assert_eq!(out[0].len(), 32);
    assert!(out[0].iter().all(|&x| x == 0.0));
}

#[test]
fn batch_returns_one_per_input() {
    let be = HashEmbedder::new();
    let out = be
        .embed(&["a".into(), "b".into(), "c".into()], &card(16))
        .unwrap();
    assert_eq!(out.len(), 3);
}

#[test]
fn registry_resolves_default_backend() {
    let mut reg = BackendRegistry::new();
    reg.register(Box::new(HashEmbedder::new()));
    reg.set_default("hash-embedder");
    let be = reg.default().unwrap();
    assert_eq!(be.id(), "hash-embedder");
    let out = be.embed(&["x".into()], &card(8)).unwrap();
    assert_eq!(out[0].len(), 8);
}
