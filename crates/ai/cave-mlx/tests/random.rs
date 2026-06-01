// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for the `mx.random` distribution suite (cave-mlx `random` module).
//!
//! The PRNG primitive is a faithful Threefry2x32-20 counter-based generator
//! (the Random123 algorithm MLX builds its `mx.random` on). It is verified
//! against the canonical Random123 known-answer-test (KAT) vectors, then the
//! distribution functions (uniform/normal/bernoulli/randint/truncated_normal/
//! categorical) are exercised for shape, support, statistical sanity, and
//! key-determinism.

use cave_mlx::random::{self, Key};

// ── Cycle 1: Threefry2x32 PRNG core + Key/split + uniform ──────────────────

#[test]
fn threefry2x32_kat_all_zero() {
    // Random123 kat_vectors.txt: threefry2x32 20 rounds, ctr=0 key=0.
    let out = random::threefry2x32([0, 0], [0, 0]);
    assert_eq!(out, [0x6b20_0159, 0x99ba_4efe]);
}

#[test]
fn threefry2x32_kat_all_ones() {
    let out = random::threefry2x32([0xffff_ffff, 0xffff_ffff], [0xffff_ffff, 0xffff_ffff]);
    assert_eq!(out, [0x1cb9_96fc, 0xbb00_2be7]);
}

#[test]
fn threefry2x32_kat_pi_digits() {
    // ctr = (0x243f6a88, 0x85a308d3), key = (0x13198a2e, 0x03707344).
    let out = random::threefry2x32([0x1319_8a2e, 0x0370_7344], [0x243f_6a88, 0x85a3_08d3]);
    assert_eq!(out, [0xc492_3a9c, 0x483d_f7a0]);
}

#[test]
fn uniform_shape_and_support() {
    let key = Key::new(42);
    let a = random::uniform(&key, -1.0, 3.0, &[2, 5]);
    assert_eq!(a.shape(), &[2, 5]);
    assert_eq!(a.size(), 10);
    for &v in a.data() {
        assert!((-1.0..3.0).contains(&v), "uniform value {v} out of [-1,3)");
    }
}

#[test]
fn uniform_is_deterministic_for_a_key() {
    let a = random::uniform(&Key::new(7), 0.0, 1.0, &[64]);
    let b = random::uniform(&Key::new(7), 0.0, 1.0, &[64]);
    assert_eq!(a.data(), b.data(), "same seed must reproduce the stream");
}

#[test]
fn distinct_keys_and_splits_decorrelate() {
    let a = random::uniform(&Key::new(1), 0.0, 1.0, &[128]);
    let b = random::uniform(&Key::new(2), 0.0, 1.0, &[128]);
    assert_ne!(a.data(), b.data(), "different seeds must differ");

    let subs = Key::new(1).split(2);
    assert_eq!(subs.len(), 2);
    let s0 = random::uniform(&subs[0], 0.0, 1.0, &[128]);
    let s1 = random::uniform(&subs[1], 0.0, 1.0, &[128]);
    assert_ne!(s0.data(), s1.data(), "split subkeys must produce independent streams");
    assert_ne!(s0.data(), a.data(), "a subkey stream must differ from the parent stream");
}

#[test]
fn uniform_mean_is_roughly_centered() {
    let a = random::uniform(&Key::new(99), 0.0, 1.0, &[20_000]);
    let mean: f32 = a.data().iter().sum::<f32>() / a.size() as f32;
    assert!((mean - 0.5).abs() < 0.02, "uniform mean {mean} not near 0.5");
}

// ── Cycle 2: normal + bernoulli + randint ──────────────────────────────────

fn moments(xs: &[f32]) -> (f32, f32) {
    let n = xs.len() as f32;
    let mean = xs.iter().sum::<f32>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    (mean, var.sqrt())
}

#[test]
fn normal_shape_mean_and_std() {
    let a = random::normal(&Key::new(5), 2.0, 0.5, &[40_000]);
    assert_eq!(a.shape(), &[40_000]);
    let (mean, std) = moments(a.data());
    assert!((mean - 2.0).abs() < 0.02, "normal mean {mean} not near loc=2.0");
    assert!((std - 0.5).abs() < 0.02, "normal std {std} not near scale=0.5");
}

#[test]
fn normal_is_deterministic_for_a_key() {
    let a = random::normal(&Key::new(11), 0.0, 1.0, &[256]);
    let b = random::normal(&Key::new(11), 0.0, 1.0, &[256]);
    assert_eq!(a.data(), b.data());
}

#[test]
fn bernoulli_is_binary_with_expected_rate() {
    let p = 0.3;
    let a = random::bernoulli(&Key::new(8), p, &[30_000]);
    assert_eq!(a.shape(), &[30_000]);
    for &v in a.data() {
        assert!(v == 0.0 || v == 1.0, "bernoulli value {v} is not 0/1");
    }
    let rate = a.data().iter().sum::<f32>() / a.size() as f32;
    assert!((rate - p).abs() < 0.02, "bernoulli rate {rate} not near p={p}");
}

#[test]
fn randint_range_integrality_and_coverage() {
    let a = random::randint(&Key::new(3), 5, 10, &[5_000]);
    assert_eq!(a.shape(), &[5_000]);
    let mut seen = [false; 5];
    for &v in a.data() {
        assert_eq!(v, v.trunc(), "randint value {v} is not integral");
        assert!((5.0..10.0).contains(&v), "randint value {v} out of [5,10)");
        seen[(v as usize) - 5] = true;
    }
    assert!(seen.iter().all(|&s| s), "randint should cover every value in [5,10)");
}
