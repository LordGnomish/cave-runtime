// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vector embeddings + cosine similarity for semantic search.
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/knn/

use crate::tenant::TenantId;

pub fn compute_embedding(_text: &str, _tenant_id: &TenantId) -> Vec<f64> {
    panic!("cave-search::embeddings::compute_embedding")
}

pub fn cosine_similarity(_v1: &[f64], _v2: &[f64]) -> f64 {
    panic!("cave-search::embeddings::cosine_similarity")
}
