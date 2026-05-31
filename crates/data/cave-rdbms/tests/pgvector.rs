// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of the pgvector extension's `vector` type and distance
// operators (pgvector/src/vector.c — l2/inner-product/cosine/l1, accumulated
// in double, dimensions matched, VECTOR_MAX_DIM enforced) plus a brute-force
// nearest-neighbour scan standing in for an ivfflat/hnsw probe.

use cave_rdbms::storage::pgvector::{nearest_neighbor, Vector, VectorError, VECTOR_MAX_DIM};

fn v(xs: &[f32]) -> Vector {
    Vector::new(xs.to_vec())
}

#[test]
fn dim_and_limits() {
    assert_eq!(v(&[1.0, 2.0, 3.0]).dim(), 3);
    assert_eq!(VECTOR_MAX_DIM, 16000);
    assert!(Vector::checked(vec![0.0; 16000]).is_ok());
    assert_eq!(
        Vector::checked(vec![0.0; 16001]),
        Err(VectorError::TooManyDimensions(16001))
    );
}

#[test]
fn l2_distance_matches_pgvector() {
    // sqrt((1-4)^2 + (2-6)^2 + (3-3)^2) = sqrt(9+16+0) = 5
    let d = v(&[1.0, 2.0, 3.0]).l2_distance(&v(&[4.0, 6.0, 3.0])).unwrap();
    assert!((d - 5.0).abs() < 1e-9);
}

#[test]
fn inner_product_and_negative() {
    // 1*4 + 2*5 + 3*6 = 32
    let a = v(&[1.0, 2.0, 3.0]);
    let b = v(&[4.0, 5.0, 6.0]);
    assert!((a.inner_product(&b).unwrap() - 32.0).abs() < 1e-9);
    // the <#> operator returns the *negative* inner product
    assert!((a.negative_inner_product(&b).unwrap() + 32.0).abs() < 1e-9);
}

#[test]
fn cosine_distance_endpoints() {
    // orthogonal → cosine similarity 0 → distance 1
    let d = v(&[1.0, 0.0]).cosine_distance(&v(&[0.0, 1.0])).unwrap();
    assert!((d - 1.0).abs() < 1e-9);
    // identical direction → similarity 1 → distance 0
    let d2 = v(&[1.0, 1.0]).cosine_distance(&v(&[2.0, 2.0])).unwrap();
    assert!(d2.abs() < 1e-9);
    // a zero-norm vector yields NaN, as pgvector does
    let dz = v(&[0.0, 0.0]).cosine_distance(&v(&[1.0, 1.0])).unwrap();
    assert!(dz.is_nan());
}

#[test]
fn l1_distance_is_taxicab() {
    // |1-4| + |2-6| + |3-3| = 3+4+0 = 7
    let d = v(&[1.0, 2.0, 3.0]).l1_distance(&v(&[4.0, 6.0, 3.0])).unwrap();
    assert!((d - 7.0).abs() < 1e-9);
}

#[test]
fn dimension_mismatch_errors() {
    assert_eq!(
        v(&[1.0, 2.0]).l2_distance(&v(&[1.0, 2.0, 3.0])),
        Err(VectorError::DimMismatch(2, 3))
    );
}

#[test]
fn nearest_neighbor_brute_force() {
    let query = v(&[0.0, 0.0]);
    let corpus = vec![v(&[10.0, 10.0]), v(&[1.0, 1.0]), v(&[5.0, 5.0])];
    // closest by L2 is index 1 ([1,1])
    assert_eq!(nearest_neighbor(&query, &corpus), Some(1));
    assert_eq!(nearest_neighbor(&query, &[]), None);
}
