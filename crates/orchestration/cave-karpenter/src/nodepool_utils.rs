// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! NodePool utility helpers — port of the pure functions in
//! `pkg/utils/nodepool/nodepool.go` from kubernetes-sigs/karpenter v1.12.1
//! (sha `ed490e8`).
//!
//! The provisioner evaluates NodePools highest-weight-first so that an
//! operator can express preference (e.g. prefer a cheaper Spot pool, fall
//! back to On-Demand). [`order_by_weight`] is the comparator the scheduler
//! applies before its first-match walk.

use crate::models::NodePool;

/// `lo.FromPtr(nodePool.Spec.Weight)` — an unset weight counts as `0`.
pub fn effective_weight(pool: &NodePool) -> i32 {
    pool.weight.unwrap_or(0)
}

/// `OrderByWeight` — sort `pools` in place, highest weight first, breaking
/// ties by name ascending for a stable, deterministic ordering.
pub fn order_by_weight(pools: &mut [NodePool]) {
    pools.sort_by(|a, b| {
        let wa = effective_weight(a);
        let wb = effective_weight(b);
        // weight descending, then name ascending
        wb.cmp(&wa).then_with(|| a.name.cmp(&b.name))
    });
}

/// Non-mutating twin of [`order_by_weight`]: returns a weight-ordered clone,
/// leaving the input untouched (convenient for read-only slices).
pub fn ordered_by_weight(pools: &[NodePool]) -> Vec<NodePool> {
    let mut out = pools.to_vec();
    order_by_weight(&mut out);
    out
}
