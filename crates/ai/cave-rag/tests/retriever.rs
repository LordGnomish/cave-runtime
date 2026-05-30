// SPDX-License-Identifier: AGPL-3.0-or-later
//! Retrievers: similarity, MMR (diversity), BM25 (sparse), hybrid (RRF).

use cave_rag::document::Document;
use cave_rag::embedding::HashingEmbedder;
use cave_rag::retriever::{
    maximal_marginal_relevance, reciprocal_rank_fusion, Bm25Retriever, HybridRetriever,
    MmrRetriever, Retriever, SimilarityRetriever,
};
use cave_rag::vectorstore::{InMemoryVectorStore, VectorStore};

fn programming_corpus() -> Vec<Document> {
    vec![
        Document::new("rust is a systems programming language").with_source("d1"),
        Document::new("rust is a fast safe systems programming language").with_source("d2"),
        Document::new("the cat sat on the warm windowsill mat").with_source("d3"),
    ]
}

fn build_store(docs: Vec<Document>, e: &HashingEmbedder) -> InMemoryVectorStore {
    let mut s = InMemoryVectorStore::new();
    s.add(docs, e).unwrap();
    s
}

#[test]
fn similarity_retriever_returns_top_k_relevant() {
    let e = HashingEmbedder::new(256);
    let store = build_store(programming_corpus(), &e);
    let r = SimilarityRetriever::new(&store, &e);
    let hits = r.retrieve("which systems programming language", 2).unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| h.document.content.contains("programming")));
}

#[test]
fn mmr_retriever_favors_diversity() {
    let e = HashingEmbedder::new(256);
    let store = build_store(programming_corpus(), &e);

    // Pure similarity returns the two near-duplicate programming docs.
    let sim = SimilarityRetriever::new(&store, &e)
        .retrieve("rust programming", 2)
        .unwrap();
    assert!(!sim.iter().any(|h| h.document.content.contains("cat")));

    // MMR with a low lambda pulls in the diverse (cat) document.
    let mmr = MmrRetriever::new(&store, &e)
        .with_lambda(0.3)
        .with_fetch_k(3)
        .retrieve("rust programming", 2)
        .unwrap();
    assert_eq!(mmr.len(), 2);
    assert!(
        mmr.iter().any(|h| h.document.content.contains("cat")),
        "diversity should surface the unrelated doc"
    );
}

#[test]
fn bm25_ranks_exact_lexical_match_first() {
    let corpus = vec![
        Document::new("the quick brown fox jumps over").with_source("d1"),
        Document::new("the lazy dog sleeps all day long").with_source("d2"),
        Document::new("quick brown animals run very fast").with_source("d3"),
    ];
    let r = Bm25Retriever::new(corpus);
    let hits = r.retrieve("quick fox", 2).unwrap();
    assert_eq!(hits.len(), 2);
    assert!(
        hits[0].document.content.contains("fox"),
        "the doc with the rare exact term 'fox' should rank first"
    );
    assert!(hits[0].score >= hits[1].score);
}

#[test]
fn rrf_fuses_two_rankings_deterministically() {
    let r1 = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let r2 = vec!["a".to_string(), "d".to_string(), "b".to_string()];
    let fused = reciprocal_rank_fusion(&[r1, r2], 60);
    let order: Vec<&str> = fused.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(order, vec!["a", "b", "d", "c"]);
}

#[test]
fn mmr_function_picks_relevant_then_diverse() {
    // query at origin direction; two candidates aligned with query, one orthogonal.
    let query = vec![1.0, 0.0];
    let cands = vec![
        ("near1".to_string(), vec![1.0, 0.0]),
        ("near2".to_string(), vec![0.99, 0.01]),
        ("ortho".to_string(), vec![0.0, 1.0]),
    ];
    let picked = maximal_marginal_relevance(&query, &cands, 0.3, 2);
    assert_eq!(picked.len(), 2);
    assert_eq!(picked[0], "near1");
    assert_eq!(picked[1], "ortho", "low lambda should pick the diverse one second");
}

#[test]
fn hybrid_retriever_combines_dense_and_sparse() {
    let e = HashingEmbedder::new(256);
    let docs = programming_corpus();
    let store = build_store(docs.clone(), &e);
    let dense = SimilarityRetriever::new(&store, &e);
    let sparse = Bm25Retriever::new(docs);
    let hybrid = HybridRetriever::new(&dense, &sparse);
    let hits = hybrid.retrieve("rust programming language", 3).unwrap();
    assert_eq!(hits.len(), 3);
    assert!(hits[0].document.content.contains("programming"));
}
