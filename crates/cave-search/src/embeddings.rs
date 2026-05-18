// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Vector embeddings + cosine similarity for the semantic-search side of
//! cave-search.
//!
//! `compute_embedding` produces a deterministic, pure-Rust 64-dim sparse
//! hash-of-tokens embedding.  It is *not* a learned model — the real path
//! is plug-in (cf. OpenSearch k-NN's MODEL_INDEX) — but it lets the rest
//! of the engine (vector router, similarity index) be exercised end-to-end
//! without an external service.  Hashing uses FNV-1a (already a crate dep)
//! so the output is stable across runs and platforms.
//!
//! `cosine_similarity` is the textbook formula; guards against empty inputs,
//! mismatched dimension, and zero-norm vectors by returning `0.0`.

use crate::analyzer::tokenize;
use crate::tenant::TenantId;
use fnv::FnvHasher;
use std::hash::Hasher;

const DIM: usize = 64;

pub fn compute_embedding(text: &str, tenant_id: &TenantId) -> Vec<f64> {
    let mut v = vec![0.0_f64; DIM];
    for tok in tokenize(text, tenant_id) {
        let mut h = FnvHasher::default();
        h.write(tok.as_bytes());
        let digest = h.finish();
        let bucket = (digest as usize) % DIM;
        // Sign: top bit of digest gives ±1 — keeps near-orthogonality
        // across distinct tokens (count-sketch style).
        let sign = if digest & (1 << 63) != 0 { -1.0 } else { 1.0 };
        v[bucket] += sign;
    }
    v
}

pub fn cosine_similarity(v1: &[f64], v2: &[f64]) -> f64 {
    if v1.is_empty() || v2.is_empty() || v1.len() != v2.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut n1 = 0.0;
    let mut n2 = 0.0;
    for (a, b) in v1.iter().zip(v2.iter()) {
        dot += a * b;
        n1 += a * a;
        n2 += b * b;
    }
    if n1 == 0.0 || n2 == 0.0 {
        return 0.0;
    }
    dot / (n1.sqrt() * n2.sqrt())
}
