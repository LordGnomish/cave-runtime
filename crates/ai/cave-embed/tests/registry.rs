// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 2 — model registry.
//
// infinity serves many embedding models; each carries metadata the serving
// path needs: output dimensionality, context window, the pooling default, and
// whether outputs are L2-normalized. Asymmetric models (E5, BGE, nomic) also
// prepend role prefixes to queries vs. documents.

use cave_embed::pooling::Pooling;
use cave_embed::registry::{Modality, ModelRegistry};

#[test]
fn builtin_models_are_present() {
    let r = ModelRegistry::with_builtins();
    // A representative model from each family the priority list names.
    for id in [
        "sentence-transformers/all-MiniLM-L6-v2",
        "BAAI/bge-base-en-v1.5",
        "intfloat/e5-base-v2",
        "intfloat/multilingual-e5-large",
        "mistralai/mistral-embed",
        "nomic-ai/nomic-embed-text-v1.5",
        "jinaai/jina-embeddings-v2-base-en",
    ] {
        assert!(r.get(id).is_some(), "missing builtin model {id}");
    }
}

#[test]
fn minilm_card_metadata() {
    let r = ModelRegistry::with_builtins();
    let c = r.get("sentence-transformers/all-MiniLM-L6-v2").unwrap();
    assert_eq!(c.dimensions, 384);
    assert_eq!(c.max_seq_len, 256);
    assert_eq!(c.pooling, Pooling::Mean);
    assert!(c.normalize);
    assert_eq!(c.modality, Modality::Text);
}

#[test]
fn bge_uses_cls_pooling() {
    let r = ModelRegistry::with_builtins();
    let c = r.get("BAAI/bge-base-en-v1.5").unwrap();
    assert_eq!(c.dimensions, 768);
    assert_eq!(c.pooling, Pooling::Cls);
}

#[test]
fn e5_prefixes_query_and_passage() {
    let r = ModelRegistry::with_builtins();
    let c = r.get("intfloat/e5-base-v2").unwrap();
    assert_eq!(c.format_query("how tall"), "query: how tall");
    assert_eq!(c.format_passage("the tower"), "passage: the tower");
}

#[test]
fn nomic_uses_search_prefixes() {
    let r = ModelRegistry::with_builtins();
    let c = r.get("nomic-ai/nomic-embed-text-v1.5").unwrap();
    assert_eq!(c.format_query("cats"), "search_query: cats");
    assert_eq!(c.format_passage("dogs"), "search_document: dogs");
}

#[test]
fn model_without_prefix_is_identity() {
    let r = ModelRegistry::with_builtins();
    let c = r.get("sentence-transformers/all-MiniLM-L6-v2").unwrap();
    assert_eq!(c.format_query("hi"), "hi");
    assert_eq!(c.format_passage("hi"), "hi");
}

#[test]
fn lookup_by_alias() {
    let r = ModelRegistry::with_builtins();
    // short alias resolves to the canonical card.
    let by_alias = r.get("all-MiniLM-L6-v2").unwrap();
    assert_eq!(by_alias.id, "sentence-transformers/all-MiniLM-L6-v2");
}

#[test]
fn register_custom_model() {
    let mut r = ModelRegistry::new();
    assert!(r.get("custom/foo").is_none());
    r.register(cave_embed::registry::ModelCard::text(
        "custom/foo",
        128,
        64,
        Pooling::Mean,
        true,
    ));
    let c = r.get("custom/foo").unwrap();
    assert_eq!(c.dimensions, 128);
}

#[test]
fn clip_is_multimodal() {
    let r = ModelRegistry::with_builtins();
    let c = r.get("openai/clip-vit-base-patch32").unwrap();
    assert_eq!(c.modality, Modality::Multimodal);
}

#[test]
fn list_is_sorted_and_nonempty() {
    let r = ModelRegistry::with_builtins();
    let ids = r.list_ids();
    assert!(ids.len() >= 7);
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "list_ids must be sorted for stable /v1/models");
}
