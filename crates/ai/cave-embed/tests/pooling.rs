// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 1 — pooling strategies (sentence-transformers parity).
//
// Token embeddings arrive as a [seq_len][hidden] row-major matrix with an
// attention mask marking real (1) vs padding (0) tokens. infinity/
// sentence-transformers support mean (mask-aware), CLS, max, and last-token
// pooling, optionally followed by L2 normalization.

use cave_embed::pooling::{l2_normalize, pool, Pooling};

// 3 tokens, hidden=2. Last token is padding (mask=0).
fn fixture() -> (Vec<Vec<f32>>, Vec<u32>) {
    let tokens = vec![
        vec![1.0, 2.0],
        vec![3.0, 4.0],
        vec![100.0, 100.0], // padding — must be ignored by mask-aware ops
    ];
    let mask = vec![1u32, 1, 0];
    (tokens, mask)
}

#[test]
fn mean_pool_is_mask_aware() {
    let (t, m) = fixture();
    let out = pool(Pooling::Mean, &t, &m).unwrap();
    // mean of rows 0,1 only: ((1+3)/2, (2+4)/2) = (2.0, 3.0)
    assert_eq!(out, vec![2.0, 3.0]);
}

#[test]
fn cls_pool_takes_first_token() {
    let (t, m) = fixture();
    let out = pool(Pooling::Cls, &t, &m).unwrap();
    assert_eq!(out, vec![1.0, 2.0]);
}

#[test]
fn max_pool_is_mask_aware() {
    let (t, m) = fixture();
    let out = pool(Pooling::Max, &t, &m).unwrap();
    // elementwise max over masked rows 0,1 — padding's 100s excluded.
    assert_eq!(out, vec![3.0, 4.0]);
}

#[test]
fn last_token_pool_uses_last_unmasked() {
    let (t, m) = fixture();
    let out = pool(Pooling::LastToken, &t, &m).unwrap();
    // last token with mask==1 is row index 1.
    assert_eq!(out, vec![3.0, 4.0]);
}

#[test]
fn l2_normalize_unit_length() {
    let v = vec![3.0f32, 4.0];
    let n = l2_normalize(&v);
    assert!((n[0] - 0.6).abs() < 1e-6);
    assert!((n[1] - 0.8).abs() < 1e-6);
    let mag: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((mag - 1.0).abs() < 1e-6);
}

#[test]
fn l2_normalize_zero_vector_is_safe() {
    let v = vec![0.0f32, 0.0, 0.0];
    let n = l2_normalize(&v);
    assert_eq!(n, vec![0.0, 0.0, 0.0]);
}

#[test]
fn empty_mask_errors() {
    let t: Vec<Vec<f32>> = vec![vec![1.0, 2.0]];
    let m = vec![0u32]; // all padding → no tokens to pool
    assert!(pool(Pooling::Mean, &t, &m).is_err());
}

#[test]
fn pooling_parses_from_str() {
    assert_eq!("mean".parse::<Pooling>().unwrap(), Pooling::Mean);
    assert_eq!("cls".parse::<Pooling>().unwrap(), Pooling::Cls);
    assert_eq!("max".parse::<Pooling>().unwrap(), Pooling::Max);
    assert_eq!("lasttoken".parse::<Pooling>().unwrap(), Pooling::LastToken);
    assert!("bogus".parse::<Pooling>().is_err());
}
