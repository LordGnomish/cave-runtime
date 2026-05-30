// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 8 — cross-encoder reranker + /v1/rerank API.
//
// infinity exposes a reranker endpoint: given a query and N documents, score
// each (query, document) pair and return them sorted by relevance. A concrete
// cross-encoder plugs in through the CrossEncoder trait; the reference scorer is
// a real lexical-overlap relevance signal (shared query terms), deterministic
// and dependency-free.

use cave_embed::rerank::{
    CrossEncoder, LexicalCrossEncoder, RerankRequest, Reranker,
};

#[test]
fn reference_scorer_rewards_query_overlap() {
    let ce = LexicalCrossEncoder::new();
    let relevant = ce.score("rust async runtime", "the rust async runtime tokio");
    let irrelevant = ce.score("rust async runtime", "banana bread recipe");
    assert!(relevant > irrelevant);
    assert!(relevant > 0.0 && irrelevant == 0.0);
}

#[test]
fn rerank_sorts_by_relevance_descending() {
    let rr = Reranker::new(Box::new(LexicalCrossEncoder::new()));
    let docs = vec![
        "completely unrelated text".to_string(),
        "vector database for embeddings".to_string(),
        "embeddings and vector search".to_string(),
    ];
    let out = rr.rerank("vector embeddings", &docs, None);
    // most-overlapping doc first.
    assert_eq!(out[0].index, 2);
    assert!(out[0].relevance_score >= out[1].relevance_score);
    assert!(out[1].relevance_score >= out[2].relevance_score);
    // original index 0 (no overlap) ranks last.
    assert_eq!(out.last().unwrap().index, 0);
}

#[test]
fn top_n_limits_results() {
    let rr = Reranker::new(Box::new(LexicalCrossEncoder::new()));
    let docs = vec!["a b".to_string(), "b c".to_string(), "c d".to_string()];
    let out = rr.rerank("b c", &docs, Some(2));
    assert_eq!(out.len(), 2);
}

#[test]
fn empty_documents_yields_empty() {
    let rr = Reranker::new(Box::new(LexicalCrossEncoder::new()));
    let out = rr.rerank("q", &[], None);
    assert!(out.is_empty());
}

#[test]
fn rerank_request_deserializes() {
    let req: RerankRequest = serde_json::from_str(
        r#"{"model":"reranker","query":"q","documents":["a","b"],"top_n":1,"return_documents":true}"#,
    )
    .unwrap();
    assert_eq!(req.query, "q");
    assert_eq!(req.documents.len(), 2);
    assert_eq!(req.top_n, Some(1));
    assert!(req.return_documents);
}

#[test]
fn response_includes_documents_when_requested() {
    let rr = Reranker::new(Box::new(LexicalCrossEncoder::new()));
    let req = RerankRequest {
        model: "reranker".into(),
        query: "vector".into(),
        documents: vec!["vector db".into(), "unrelated".into()],
        top_n: None,
        return_documents: true,
    };
    let resp = rr.serve(&req);
    assert_eq!(resp.model, "reranker");
    assert_eq!(resp.results[0].document.as_deref(), Some("vector db"));
    // without return_documents the text is omitted.
    let mut req2 = req.clone();
    req2.return_documents = false;
    let resp2 = rr.serve(&req2);
    assert!(resp2.results[0].document.is_none());
}
