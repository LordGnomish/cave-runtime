// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BM25 scoring: stub.

use crate::index::Index;

#[derive(Debug, Clone)]
pub struct BM25Params { pub k1: f64, pub b: f64 }
impl Default for BM25Params { fn default() -> Self { BM25Params { k1: 1.2, b: 0.75 } } }

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredDoc { pub doc_id: u32, pub score: f64 }

pub fn bm25_score(_tf: u32, _doc_len: u32, _avg_doc_len: f64, _df: u32, _n: u32) -> f64 { 0.0 }
pub fn bm25_score_with_params(_tf: u32, _dl: u32, _avgdl: f64, _df: u32, _n: u32, _p: &BM25Params) -> f64 { 0.0 }
pub fn search_bm25(_index: &Index, _term: &str, _params: &BM25Params) -> Vec<ScoredDoc> { Vec::new() }
pub fn rank_results(docs: Vec<ScoredDoc>, _k: usize) -> Vec<ScoredDoc> { docs }
pub fn search_multi_term(_index: &Index, _query: &str, _params: &BM25Params, _top_k: usize) -> Vec<ScoredDoc> { Vec::new() }
