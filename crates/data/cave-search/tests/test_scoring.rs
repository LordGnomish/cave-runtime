// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for BM25 relevance scoring — formula correctness, ranking, multi-term search.

use cave_search::scoring::{bm25_score, BM25Params, rank_results, ScoredDoc};
use cave_search::index::Index;
use cave_search::tenant::TenantId;
use std::str::FromStr;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

#[test]
fn bm25_zero_term_freq_gives_zero_score() {
    // If the term doesn't appear in the doc, score must be 0.0
    let score = bm25_score(0, 10, 10.0, 5, 100);
    assert_eq!(score, 0.0);
}

#[test]
fn bm25_higher_tf_gives_higher_score() {
    let avg_len = 10.0;
    let doc_freq = 5u32;
    let num_docs = 100u32;
    let doc_len = 10u32;
    let s1 = bm25_score(1, doc_len, avg_len, doc_freq, num_docs);
    let s2 = bm25_score(5, doc_len, avg_len, doc_freq, num_docs);
    assert!(s2 > s1, "TF=5 should score higher than TF=1; s1={}, s2={}", s1, s2);
}

#[test]
fn bm25_rarer_term_gives_higher_score() {
    // Lower doc_freq (rarer term) → higher IDF → higher BM25
    let avg_len = 10.0;
    let tf = 2u32;
    let doc_len = 10u32;
    let num_docs = 1000u32;
    let s_rare = bm25_score(tf, doc_len, avg_len, 5, num_docs);
    let s_common = bm25_score(tf, doc_len, avg_len, 500, num_docs);
    assert!(s_rare > s_common, "rare term should score higher; s_rare={}, s_common={}", s_rare, s_common);
}

#[test]
fn bm25_longer_doc_penalized() {
    // A longer document with the same TF gets a lower score (length normalization)
    let avg_len = 10.0;
    let df = 5u32;
    let num_docs = 100u32;
    let tf = 2u32;
    let s_short = bm25_score(tf, 5, avg_len, df, num_docs);
    let s_long = bm25_score(tf, 50, avg_len, df, num_docs);
    assert!(s_short > s_long, "shorter doc should score higher; s_short={}, s_long={}", s_short, s_long);
}

#[test]
fn bm25_score_is_positive_for_matching_term() {
    let score = bm25_score(3, 10, 10.0, 5, 100);
    assert!(score > 0.0, "score must be positive; got {}", score);
}

#[test]
fn bm25_params_default_k1_b() {
    let p = BM25Params::default();
    // BM25 standard defaults
    assert!((p.k1 - 1.2).abs() < 1e-9);
    assert!((p.b - 0.75).abs() < 1e-9);
}

#[test]
fn rank_results_sorts_by_score_descending() {
    let docs = vec![
        ScoredDoc { doc_id: 1, score: 2.5 },
        ScoredDoc { doc_id: 2, score: 10.0 },
        ScoredDoc { doc_id: 3, score: 1.0 },
    ];
    let ranked = rank_results(docs, 10);
    assert_eq!(ranked[0].doc_id, 2);
    assert_eq!(ranked[1].doc_id, 1);
    assert_eq!(ranked[2].doc_id, 3);
}

#[test]
fn rank_results_respects_top_k() {
    let docs: Vec<ScoredDoc> = (0..20).map(|i| ScoredDoc { doc_id: i, score: i as f64 }).collect();
    let ranked = rank_results(docs, 5);
    assert_eq!(ranked.len(), 5);
    // Top score should be doc_id 19 (score 19.0)
    assert_eq!(ranked[0].doc_id, 19);
}

#[test]
fn score_term_against_index() {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    idx.add_document(1, "rust programming language for systems");
    idx.add_document(2, "python web programming frameworks");
    idx.add_document(3, "java enterprise programming patterns");

    let params = BM25Params::default();
    let results = cave_search::scoring::search_bm25(&idx, "programming", &params);
    // All 3 docs have "programming" — should get results
    assert_eq!(results.len(), 3);
    // All scores positive
    assert!(results.iter().all(|d| d.score > 0.0));
}
