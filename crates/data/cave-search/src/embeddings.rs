// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vector embeddings: stub.

use crate::tenant::TenantId;

pub fn dot_product(_v1: &[f64], _v2: &[f64]) -> f64 { 0.0 }
pub fn euclidean_distance(_v1: &[f64], _v2: &[f64]) -> f64 { 0.0 }
pub fn cosine_similarity(_v1: &[f64], _v2: &[f64]) -> f64 { 0.0 }
pub fn tfidf_vector(_text: &str, _corpus: &[String], _tenant: &TenantId) -> Vec<f64> { Vec::new() }
pub fn compute_embedding(_text: &str, _tenant_id: &TenantId) -> Vec<f64> { Vec::new() }
