// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Integration tests for cave-search core primitives — pinning the contract
// that the previous skeleton modules (`unimplemented!()`) must satisfy.
//
// Scope:
//   * scoring::bm25_score                — formula + idf monotonicity
//   * analyzer::tokenize / filter_stop_words
//   * embeddings::compute_embedding / cosine_similarity
//   * index::Index / index::PostingList  — inverted index basics
//   * query::Query::execute              — term / phrase (AND collapse) / bool
//   * query::BooleanQuery::{and, or, not} constructors
//
// Upstream behaviour reference: Lucene `Similarity` BM25Similarity (k1=1.2,
// b=0.75) and `IndexReader`-level posting-list semantics.

use cave_kernel::ns::TenantId;
use cave_search::analyzer::{filter_stop_words, tokenize};
use cave_search::embeddings::{compute_embedding, cosine_similarity};
use cave_search::index::{Index, PostingList};
use cave_search::query::{BoolNode, BooleanQuery, Query};
use cave_search::scoring::bm25_score;

fn t() -> TenantId {
    TenantId::new("acme").expect("valid tenant id")
}

// ── scoring ───────────────────────────────────────────────────────────────

#[test]
fn bm25_score_zero_when_term_absent() {
    let s = bm25_score(0, 100, 100.0, 5, 1000);
    assert_eq!(s, 0.0, "tf=0 must give zero contribution");
}

#[test]
fn bm25_score_positive_when_term_present() {
    let s = bm25_score(3, 100, 100.0, 5, 1000);
    assert!(s > 0.0, "tf>0 with positive idf must score positive, got {s}");
}

#[test]
fn bm25_idf_is_monotone_in_rarity() {
    // Rarer term (smaller df) should outscore a common term, holding tf fixed.
    let rare = bm25_score(2, 100, 100.0, 1, 1000);
    let common = bm25_score(2, 100, 100.0, 500, 1000);
    assert!(rare > common, "rare={rare} should beat common={common}");
}

#[test]
fn bm25_length_normalisation_penalises_long_docs() {
    // Same tf+idf; longer doc relative to avg should score lower.
    let short = bm25_score(3, 50, 100.0, 5, 1000);
    let long = bm25_score(3, 400, 100.0, 5, 1000);
    assert!(short > long, "short={short} should beat long={long}");
}

// ── analyzer ──────────────────────────────────────────────────────────────

#[test]
fn tokenize_lowercases_and_splits() {
    let toks = tokenize("Hello, World! Foo-Bar 123", &t());
    assert_eq!(toks, vec!["hello", "world", "foo", "bar", "123"]);
}

#[test]
fn tokenize_empty_yields_empty() {
    assert!(tokenize("   ", &t()).is_empty());
    assert!(tokenize("", &t()).is_empty());
}

#[test]
fn filter_stop_words_drops_common_terms() {
    let words = vec!["the", "quick", "brown", "fox", "is", "fast"];
    let kept = filter_stop_words(words, &t());
    assert_eq!(kept, vec!["quick", "brown", "fox", "fast"]);
}

// ── embeddings ────────────────────────────────────────────────────────────

#[test]
fn embedding_is_deterministic() {
    let a = compute_embedding("the brown fox", &t());
    let b = compute_embedding("the brown fox", &t());
    assert_eq!(a, b, "embedding must be a pure function of (text, tenant)");
    assert!(!a.is_empty(), "embedding must have non-zero dimension");
}

#[test]
fn embedding_changes_with_text() {
    let a = compute_embedding("alpha beta", &t());
    let b = compute_embedding("gamma delta epsilon", &t());
    assert_ne!(a, b);
}

#[test]
fn cosine_similarity_identity_is_one() {
    let v = vec![0.5, -0.25, 1.0, 0.1];
    let s = cosine_similarity(&v, &v);
    assert!((s - 1.0).abs() < 1e-9, "cos(v,v) should be 1.0, got {s}");
}

#[test]
fn cosine_similarity_orthogonal_is_zero() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0];
    let s = cosine_similarity(&a, &b);
    assert!(s.abs() < 1e-12);
}

#[test]
fn cosine_similarity_handles_zero_and_mismatch() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
    assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
}

// ── index — Index ──────────────────────────────────────────────────────────

#[test]
fn index_id_returns_tenant_id() {
    let tenant = t();
    let idx = Index::new(&tenant);
    assert_eq!(idx.id(), "acme");
}

#[test]
fn index_add_then_lookup() {
    let tenant = t();
    let mut idx = Index::new(&tenant);
    idx.add_document(1, "The quick brown fox jumps");
    idx.add_document(2, "Slow brown turtle");
    let mut hits = idx.get_doc_ids_for_term("brown");
    hits.sort_unstable();
    assert_eq!(hits, vec![1, 2]);
    assert_eq!(idx.get_doc_ids_for_term("quick"), vec![1]);
    assert!(idx.get_doc_ids_for_term("nonexistent").is_empty());
}

#[test]
fn index_terms_are_normalised_to_lowercase() {
    let tenant = t();
    let mut idx = Index::new(&tenant);
    idx.add_document(1, "ALPHA Beta");
    assert_eq!(idx.get_doc_ids_for_term("alpha"), vec![1]);
    assert_eq!(idx.get_doc_ids_for_term("ALPHA"), vec![1]); // query side normalised too
    assert_eq!(idx.get_doc_ids_for_term("beta"), vec![1]);
}

#[test]
fn index_delete_document_removes_from_postings() {
    let tenant = t();
    let mut idx = Index::new(&tenant);
    idx.add_document(1, "alpha beta");
    idx.add_document(2, "alpha gamma");
    idx.delete_document(1);
    assert_eq!(idx.get_doc_ids_for_term("alpha"), vec![2]);
    assert!(idx.get_doc_ids_for_term("beta").is_empty());
    assert_eq!(idx.get_doc_ids_for_term("gamma"), vec![2]);
}

// ── index — PostingList ───────────────────────────────────────────────────

#[test]
fn posting_list_add_and_freqs() {
    let mut p = PostingList::new();
    p.add_doc(10, 3);
    p.add_doc(20, 1);
    p.add_doc(10, 2); // re-adding same doc replaces (last-write-wins)
    assert_eq!(p.doc_freq(), 2);
    assert_eq!(p.total_term_freq(), 2 + 1); // 10→2, 20→1
    assert_eq!(p.get_doc_freq(10), 2);
    assert_eq!(p.get_doc_freq(20), 1);
    assert_eq!(p.get_doc_freq(999), 0);
}

#[test]
fn posting_list_remove_doc() {
    let mut p = PostingList::new();
    p.add_doc(1, 2);
    p.add_doc(2, 5);
    p.remove_doc(1);
    assert_eq!(p.doc_freq(), 1);
    assert_eq!(p.total_term_freq(), 5);
    assert_eq!(p.get_doc_freq(1), 0);
    // removing absent is a no-op
    p.remove_doc(999);
    assert_eq!(p.doc_freq(), 1);
}

#[test]
fn posting_list_merge_combines_freqs() {
    let mut a = PostingList::new();
    a.add_doc(1, 2);
    a.add_doc(2, 1);
    let mut b = PostingList::new();
    b.add_doc(2, 3); // overlap
    b.add_doc(3, 4);
    let m = PostingList::merge(vec![a, b]);
    assert_eq!(m.doc_freq(), 3);
    assert_eq!(m.get_doc_freq(1), 2);
    assert_eq!(m.get_doc_freq(2), 4); // 1 + 3
    assert_eq!(m.get_doc_freq(3), 4);
    assert_eq!(m.total_term_freq(), 2 + 4 + 4);
}

#[test]
fn posting_list_iter_visits_all() {
    let mut p = PostingList::new();
    p.add_doc(7, 9);
    p.add_doc(3, 4);
    let mut got: Vec<(u32, u32)> = p.iter().collect();
    got.sort_unstable();
    assert_eq!(got, vec![(3, 4), (7, 9)]);
}

// ── query ─────────────────────────────────────────────────────────────────

fn sample_index() -> Index {
    let tenant = t();
    let mut idx = Index::new(&tenant);
    idx.add_document(1, "alpha beta gamma");
    idx.add_document(2, "alpha gamma delta");
    idx.add_document(3, "beta delta");
    idx.add_document(4, "epsilon zeta");
    idx
}

#[test]
fn query_term_returns_matching_docs() {
    let idx = sample_index();
    let mut got = Query::Term("alpha".into()).execute(&idx);
    got.sort_unstable();
    assert_eq!(got, vec![1, 2]);
}

#[test]
fn query_phrase_collapses_to_and() {
    // Without positional postings cave-search documents Phrase as conjunction.
    let idx = sample_index();
    let mut got = Query::Phrase(vec!["alpha".into(), "gamma".into()]).execute(&idx);
    got.sort_unstable();
    assert_eq!(got, vec![1, 2]);
    // word not co-occurring
    let none = Query::Phrase(vec!["alpha".into(), "zeta".into()]).execute(&idx);
    assert!(none.is_empty());
}

#[test]
fn query_bool_must_intersects() {
    let idx = sample_index();
    let q = Query::Bool(BoolNode {
        must: vec![Query::Term("alpha".into()), Query::Term("gamma".into())],
        should: vec![],
        must_not: vec![],
    });
    let mut got = q.execute(&idx);
    got.sort_unstable();
    assert_eq!(got, vec![1, 2]);
}

#[test]
fn query_bool_should_unions() {
    let idx = sample_index();
    let q = Query::Bool(BoolNode {
        must: vec![],
        should: vec![Query::Term("alpha".into()), Query::Term("zeta".into())],
        must_not: vec![],
    });
    let mut got = q.execute(&idx);
    got.sort_unstable();
    assert_eq!(got, vec![1, 2, 4]);
}

#[test]
fn query_bool_must_not_subtracts() {
    let idx = sample_index();
    let q = Query::Bool(BoolNode {
        must: vec![Query::Term("alpha".into())],
        should: vec![],
        must_not: vec![Query::Term("delta".into())],
    });
    let got = q.execute(&idx);
    assert_eq!(got, vec![1]); // doc 2 has delta → excluded
}

#[test]
fn boolean_query_and_or_not_constructors() {
    let q_and = BooleanQuery::and(vec![Query::Term("a".into()), Query::Term("b".into())]);
    match q_and {
        Query::Bool(n) => {
            assert_eq!(n.must.len(), 2);
            assert!(n.should.is_empty());
            assert!(n.must_not.is_empty());
        }
        _ => panic!("expected Bool"),
    }
    let q_or = BooleanQuery::or(vec![Query::Term("a".into()), Query::Term("b".into())]);
    match q_or {
        Query::Bool(n) => {
            assert!(n.must.is_empty());
            assert_eq!(n.should.len(), 2);
            assert!(n.must_not.is_empty());
        }
        _ => panic!("expected Bool"),
    }
    let q_not = BooleanQuery::not(Query::Term("x".into()));
    match q_not {
        Query::Bool(n) => {
            assert!(n.must.is_empty());
            assert!(n.should.is_empty());
            assert_eq!(n.must_not.len(), 1);
        }
        _ => panic!("expected Bool"),
    }
}
