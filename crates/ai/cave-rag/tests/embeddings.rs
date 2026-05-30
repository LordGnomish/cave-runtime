// SPDX-License-Identifier: AGPL-3.0-or-later
//! Embeddings backends + the embedding-driven semantic splitter.

use cave_rag::embedding::{Embeddings, HashingEmbedder, SemanticSplitter, TfIdfEmbedder};
use cave_rag::math::cosine_similarity;

#[test]
fn hashing_embedder_is_deterministic_and_normalized() {
    let e = HashingEmbedder::new(64);
    let a = e.embed_query("machine learning is powerful").unwrap();
    let b = e.embed_query("machine learning is powerful").unwrap();
    assert_eq!(a.len(), 64);
    assert_eq!(a, b, "same text -> identical vector");
    let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "L2-normalized, got {norm}");
}

#[test]
fn hashing_embedder_places_related_text_closer() {
    let e = HashingEmbedder::new(256);
    let anchor = e.embed_query("machine learning is powerful").unwrap();
    let related = e.embed_query("machine learning is useful").unwrap();
    let unrelated = e.embed_query("banana smoothie tropical fruit").unwrap();
    assert!(
        cosine_similarity(&anchor, &related) > cosine_similarity(&anchor, &unrelated),
        "shared tokens should raise cosine similarity"
    );
}

#[test]
fn tfidf_embedder_fits_vocab_and_zeroes_unseen_terms() {
    let corpus: Vec<String> = ["the cat sat", "the dog ran", "the cat ran"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut t = TfIdfEmbedder::new();
    t.fit(&corpus);
    assert!(t.dimension() > 0);
    assert_eq!(t.dimension(), t.vocab_size());
    let cat = t.embed_query("cat").unwrap();
    assert!(cat.iter().any(|&x| x > 0.0), "known term -> nonzero");
    let unseen = t.embed_query("elephant rhinoceros").unwrap();
    assert!(unseen.iter().all(|&x| x == 0.0), "unseen terms -> zero vector");
    // embed_documents returns one vector per doc.
    let m = t.embed_documents(&corpus).unwrap();
    assert_eq!(m.len(), 3);
}

#[test]
fn tfidf_weights_rare_terms_above_common_ones() {
    let corpus: Vec<String> = ["the cat sat", "the dog ran", "the bird flew"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut t = TfIdfEmbedder::new();
    t.fit(&corpus);
    let v = t.embed_query("the cat").unwrap();
    let the_idx = t.term_index("the").unwrap();
    let cat_idx = t.term_index("cat").unwrap();
    // "the" is in every doc (idf ~ low), "cat" in one (idf high).
    assert!(v[cat_idx] > v[the_idx], "rare term must outweigh common term");
}

#[test]
fn semantic_splitter_breaks_at_topic_shift() {
    let e = HashingEmbedder::new(256);
    let text = "Cats are great pets. Cats purr when they are happy. \
                Kittens love to play with yarn. \
                Rockets burn fuel to reach orbit. Orbital mechanics is complex. \
                The rocket launch was a success.";
    let s = SemanticSplitter::new(&e).with_breakpoint_percentile(50.0);
    let chunks = s.split_text(text);
    assert!(chunks.len() >= 2, "topic shift should create a boundary");
    assert!(chunks[0].to_lowercase().contains("cat"));
    assert!(
        chunks
            .iter()
            .any(|c| c.to_lowercase().contains("rocket") || c.to_lowercase().contains("orbit")),
        "rocket topic must land in a later chunk"
    );
}
