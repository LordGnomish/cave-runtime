// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory vector store: cosine similarity search + JSON persistence.

use cave_rag::document::Document;
use cave_rag::embedding::{Embeddings, HashingEmbedder};
use cave_rag::vectorstore::{InMemoryVectorStore, VectorStore};

fn sample_store() -> (InMemoryVectorStore, HashingEmbedder) {
    let e = HashingEmbedder::new(256);
    let mut store = InMemoryVectorStore::new();
    let docs = vec![
        Document::new("python is a high level programming language").with_source("a"),
        Document::new("the recipe needs two eggs and a cup of flour").with_source("b"),
        Document::new("rust is a systems programming language").with_source("c"),
    ];
    store.add(docs, &e).unwrap();
    (store, e)
}

#[test]
fn similarity_search_ranks_relevant_doc_first() {
    let (store, e) = sample_store();
    assert_eq!(store.len(), 3);
    let q = e.embed_query("which programming language should I use").unwrap();
    let hits = store.similarity_search(&q, 2);
    assert_eq!(hits.len(), 2);
    assert!(
        hits[0].document.content.contains("programming"),
        "top hit should be a programming doc, got {:?}",
        hits[0].document.content
    );
    assert!(hits[0].score >= hits[1].score, "scores sorted descending");
}

#[test]
fn json_persistence_roundtrips() {
    let (store, e) = sample_store();
    let json = store.to_json().unwrap();
    let restored = InMemoryVectorStore::from_json(&json).unwrap();
    assert_eq!(restored.len(), 3);
    let q = e.embed_query("systems programming in rust").unwrap();
    let before = store.similarity_search(&q, 1);
    let after = restored.similarity_search(&q, 1);
    assert_eq!(before[0].document.content, after[0].document.content);
}

#[test]
fn add_returns_content_addressed_ids() {
    let (store, _e) = sample_store();
    let ids = store.ids();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|id| id.len() == 64));
}
