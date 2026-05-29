// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for vector math (cosine similarity, dot product, euclidean) and TF-IDF dense vectors.

use cave_search::embeddings::{cosine_similarity, tfidf_vector, dot_product, euclidean_distance};
use cave_search::tenant::TenantId;
use std::str::FromStr;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

#[test]
fn cosine_similarity_identical_vectors_is_one() {
    let v = vec![1.0, 2.0, 3.0];
    let sim = cosine_similarity(&v, &v);
    assert!((sim - 1.0).abs() < 1e-9, "identical vectors should have cosine=1.0, got {}", sim);
}

#[test]
fn cosine_similarity_orthogonal_vectors_is_zero() {
    let v1 = vec![1.0, 0.0, 0.0];
    let v2 = vec![0.0, 1.0, 0.0];
    let sim = cosine_similarity(&v1, &v2);
    assert!(sim.abs() < 1e-9, "orthogonal vectors should have cosine=0.0, got {}", sim);
}

#[test]
fn cosine_similarity_opposite_vectors_is_minus_one() {
    let v1 = vec![1.0, 0.0];
    let v2 = vec![-1.0, 0.0];
    let sim = cosine_similarity(&v1, &v2);
    assert!((sim + 1.0).abs() < 1e-9, "opposite vectors should have cosine=-1.0, got {}", sim);
}

#[test]
fn cosine_similarity_zero_vector_returns_zero() {
    let v1 = vec![0.0, 0.0, 0.0];
    let v2 = vec![1.0, 2.0, 3.0];
    let sim = cosine_similarity(&v1, &v2);
    assert_eq!(sim, 0.0, "zero vector should return 0.0");
}

#[test]
fn cosine_similarity_range_between_minus_one_and_one() {
    let v1 = vec![3.0, 1.0, 4.0, 1.0, 5.0];
    let v2 = vec![2.0, 7.0, 1.0, 8.0, 2.0];
    let sim = cosine_similarity(&v1, &v2);
    assert!(sim >= -1.0 && sim <= 1.0, "cosine must be in [-1,1], got {}", sim);
}

#[test]
fn dot_product_correct() {
    let v1 = vec![1.0, 2.0, 3.0];
    let v2 = vec![4.0, 5.0, 6.0];
    let dp = dot_product(&v1, &v2);
    // 1*4 + 2*5 + 3*6 = 4+10+18 = 32
    assert!((dp - 32.0).abs() < 1e-9, "dot product should be 32.0, got {}", dp);
}

#[test]
fn euclidean_distance_same_vector_is_zero() {
    let v = vec![1.0, 2.0, 3.0];
    let d = euclidean_distance(&v, &v);
    assert!(d.abs() < 1e-9, "distance to self should be 0.0, got {}", d);
}

#[test]
fn euclidean_distance_unit_vectors() {
    let v1 = vec![1.0, 0.0];
    let v2 = vec![0.0, 1.0];
    let d = euclidean_distance(&v1, &v2);
    // sqrt((1-0)^2 + (0-1)^2) = sqrt(2)
    assert!((d - 2.0_f64.sqrt()).abs() < 1e-9, "expected sqrt(2), got {}", d);
}

#[test]
fn tfidf_vector_produces_correct_dimensions() {
    let corpus = vec![
        "rust programming language".to_string(),
        "python programming language".to_string(),
        "java programming".to_string(),
    ];
    let text = "rust language";
    let v = tfidf_vector(text, &corpus, &tenant());
    // Vocabulary size == number of unique terms across all docs
    // (after tokenization and stop-word removal)
    assert!(!v.is_empty(), "TF-IDF vector must be non-empty");
    // The vector should have the same dimensionality as the vocabulary
    let vocab_size = v.len();
    let v2 = tfidf_vector("python java", &corpus, &tenant());
    assert_eq!(v2.len(), vocab_size, "all vectors should share the same vocabulary dimension");
}

#[test]
fn tfidf_vector_rust_closer_to_rust_doc_than_python_doc() {
    let corpus = vec![
        "rust systems programming memory safety".to_string(),
        "python scripting dynamic typing".to_string(),
    ];
    let v_rust = tfidf_vector("rust memory", &corpus, &tenant());
    let v_doc0 = tfidf_vector("rust systems programming memory safety", &corpus, &tenant());
    let v_doc1 = tfidf_vector("python scripting dynamic typing", &corpus, &tenant());
    let sim_with_rust = cosine_similarity(&v_rust, &v_doc0);
    let sim_with_python = cosine_similarity(&v_rust, &v_doc1);
    assert!(sim_with_rust > sim_with_python,
        "rust query should be closer to rust doc: {:.4} vs {:.4}", sim_with_rust, sim_with_python);
}
