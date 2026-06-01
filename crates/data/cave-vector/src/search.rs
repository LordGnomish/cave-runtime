// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Search composition: filtered search, MMR re-ranking, hybrid fusion.
//!
//! * `filtered_search` — Qdrant filtered ANN (payload predicate + top-k).
//! * `mmr_rerank` — Maximal Marginal Relevance (Carbonell & Goldstein 1998),
//!   the diversity re-ranker LangChain/Milvus expose as `max_marginal_relevance`.
//! * `rrf_fuse` — Reciprocal Rank Fusion (Cormack et al. 2009), the default
//!   hybrid dense+sparse combiner in Qdrant `Query::Fusion(Rrf)` / Milvus
//!   `RRFRanker`.

use crate::collection::{topk_scored, Collection};
use crate::distance::Metric;
use crate::filter::Filter;
use crate::models::{PointId, ScoredPoint};

/// Top-`k` search restricted to points whose payload passes `filter`.
pub fn filtered_search(
    c: &Collection,
    query: &[f32],
    k: usize,
    filter: &Filter,
) -> Vec<ScoredPoint> {
    let matching = c.points.iter().filter(|(_, p)| filter.matches(&p.payload));
    topk_scored(Metric(c.params.distance), query, matching, k)
}

/// Maximal Marginal Relevance re-rank.
///
/// `candidates` is the ANN candidate pool `(id, vector)`. Returns up to `k`
/// ids ordered by greedy MMR:
/// `argmax  λ·rel(q,d) − (1−λ)·max_{s∈S} sim(d,s)`.
/// `lambda = 1.0` is pure relevance; lower favours diversity.
pub fn mmr_rerank(
    query: &[f32],
    candidates: &[(PointId, Vec<f32>)],
    metric: Metric,
    lambda: f32,
    k: usize,
) -> Vec<PointId> {
    let rel: Vec<f32> = candidates.iter().map(|(_, v)| metric.score(query, v)).collect();
    let mut selected: Vec<usize> = Vec::new();
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();
    let want = k.min(candidates.len());

    while selected.len() < want && !remaining.is_empty() {
        let mut best_idx = 0usize; // position within `remaining`
        let mut best_score = f32::NEG_INFINITY;
        for (pos, &i) in remaining.iter().enumerate() {
            let max_sim_to_selected = selected
                .iter()
                .map(|&s| metric.score(&candidates[i].1, &candidates[s].1))
                .fold(f32::NEG_INFINITY, f32::max);
            let diversity = if selected.is_empty() { 0.0 } else { max_sim_to_selected };
            let mmr = lambda * rel[i] - (1.0 - lambda) * diversity;
            // strict `>` keeps the lowest original index on ties (determinism).
            if mmr > best_score {
                best_score = mmr;
                best_idx = pos;
            }
        }
        selected.push(remaining.remove(best_idx));
    }
    selected.into_iter().map(|i| candidates[i].0.clone()).collect()
}

/// Reciprocal Rank Fusion over several ranked id lists (best-first).
///
/// `score(d) = Σ_lists 1 / (k_const + rank_1based(d))`. Returns `(id, score)`
/// sorted by score descending (ties broken by id for determinism).
pub fn rrf_fuse(rankings: &[Vec<PointId>], k_const: f32) -> Vec<(PointId, f32)> {
    use std::collections::HashMap;
    let mut scores: HashMap<PointId, f32> = HashMap::new();
    for list in rankings {
        for (rank0, id) in list.iter().enumerate() {
            let rank = (rank0 + 1) as f32; // 1-based
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k_const + rank);
        }
    }
    let mut out: Vec<(PointId, f32)> = scores.into_iter().collect();
    // score desc; ties broken by id ascending for determinism.
    out.sort_by(|a, b| {
        b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Distance, Payload, Point, VectorParams};
    use serde_json::json;

    fn coll() -> Collection {
        let mut c = Collection::new(VectorParams {
            size: 2,
            distance: Distance::Euclid,
            hnsw_config: None,
            quantization: None,
        });
        let mut put = |id: u64, v: [f32; 2], color: &str| {
            let mut payload = Payload::new();
            payload.insert("color".into(), json!(color));
            c.upsert(Point { id: PointId::Num(id), vector: v.to_vec(), payload }).unwrap();
        };
        put(1, [0.0, 0.0], "red");
        put(2, [1.0, 1.0], "blue");
        put(3, [2.0, 2.0], "red");
        put(4, [0.5, 0.5], "blue");
        c
    }

    #[test]
    fn filtered_search_only_returns_matches() {
        let c = coll();
        let f = Filter {
            must: vec![crate::filter::Condition::Match { key: "color".into(), value: json!("red") }],
            ..Default::default()
        };
        let hits = filtered_search(&c, &[0.0, 0.0], 10, &f);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.id == PointId::Num(1) || h.id == PointId::Num(3)));
        // nearest red to origin is point 1.
        assert_eq!(hits[0].id, PointId::Num(1));
    }

    #[test]
    fn mmr_favours_diversity_below_half() {
        // query aligned with point A; A2 is a duplicate of A; C is orthogonal.
        let q = [1.0, 0.0, 0.0];
        let cands = vec![
            (PointId::Num(1), vec![1.0, 0.0, 0.0]), // A
            (PointId::Num(2), vec![1.0, 0.0, 0.0]), // A2 (dup)
            (PointId::Num(3), vec![0.0, 1.0, 0.0]), // C (orthogonal)
        ];
        let out = mmr_rerank(&q, &cands, Metric(Distance::Cosine), 0.3, 2);
        // pick A first (most relevant), then C (diverse) — A2 excluded.
        assert_eq!(out, vec![PointId::Num(1), PointId::Num(3)]);
    }

    #[test]
    fn mmr_pure_relevance_at_lambda_one() {
        let q = [1.0, 0.0, 0.0];
        let cands = vec![
            (PointId::Num(1), vec![1.0, 0.0, 0.0]),
            (PointId::Num(2), vec![1.0, 0.0, 0.0]),
            (PointId::Num(3), vec![0.0, 1.0, 0.0]),
        ];
        let out = mmr_rerank(&q, &cands, Metric(Distance::Cosine), 1.0, 2);
        // both A and A2 are top-relevance; C excluded.
        assert!(out.contains(&PointId::Num(1)) && out.contains(&PointId::Num(2)));
        assert!(!out.contains(&PointId::Num(3)));
    }

    #[test]
    fn rrf_fuses_ranked_lists() {
        let l1 = vec![PointId::Num(2), PointId::Num(1), PointId::Num(3)];
        let l2 = vec![PointId::Num(2), PointId::Num(1), PointId::Num(4)];
        let fused = rrf_fuse(&[l1, l2], 60.0);
        assert_eq!(fused[0].0, PointId::Num(2)); // rank-1 in both
        assert_eq!(fused[1].0, PointId::Num(1)); // rank-2 in both
        assert!(fused[0].1 > fused[1].1);
        // 3 and 4 each appear once → lower than 1 and 2.
        assert!(fused.len() == 4);
    }
}
