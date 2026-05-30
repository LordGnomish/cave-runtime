// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! NodePool hash controller — port of
//! `pkg/controllers/nodepool/hash/controller.go` from kubernetes-sigs/karpenter
//! v1.12.1 (sha `ed490e8`).
//!
//! Upstream the controller, on each NodePool reconcile, writes
//! `NodePool.Hash()` into the `karpenter.sh/nodepool-hash` annotation at the
//! current `karpenter.sh/nodepool-hash-version` (`"v3"`), then walks the owned
//! NodeClaims: any claim whose recorded hash-version is stale (or absent) is
//! re-stamped with the pool's current hash (a *migration*, not drift), while
//! claims already at the current version keep their hash so the disruption
//! controller can detect genuine drift.
//!
//! cave carries the per-object annotation as [`NodePool::template_hash`] /
//! [`NodeClaim::template_hash`], so the port stamps those fields. The hash
//! itself comes from [`crate::hash::nodepool_hash`] (cycle 12).

use crate::hash::nodepool_hash;
use crate::models::{NodeClaim, NodePool};

/// Annotation key holding the stamped `NodePool.Hash()` value.
pub const NODEPOOL_HASH_ANNOTATION: &str = "karpenter.sh/nodepool-hash";

/// Annotation key holding the hash *version* — a bump invalidates all stored
/// hashes so a hashing-algorithm change does not mass-trigger drift.
pub const NODEPOOL_HASH_VERSION_ANNOTATION: &str = "karpenter.sh/nodepool-hash-version";

/// Current hash version for the Karpenter v1 API surface.
pub const NODEPOOL_HASH_VERSION: &str = "v3";

/// Compute `NodePool.Hash()` and stamp it onto the pool. Idempotent: a pool
/// whose spec is unchanged stamps the same value.
pub fn stamp_nodepool_hash(pool: &mut NodePool) {
    pool.template_hash = Some(nodepool_hash(pool));
}

/// True when `claim` carries a recorded hash that differs from `pool`'s
/// freshly recomputed hash. A claim with no recorded hash is *not* drifted —
/// it needs an initial stamp ([`reconcile_hashes`]), not disruption.
pub fn nodepool_hash_drifted(claim: &NodeClaim, pool: &NodePool) -> bool {
    match claim.template_hash.as_ref() {
        Some(have) => have != &nodepool_hash(pool),
        None => false,
    }
}

/// Reconcile loop: stamp every pool's current hash, then sync the hash onto
/// any owned claim that does not yet carry one (initial stamp / version
/// migration). Claims that already carry a hash are left untouched so a later
/// spec change surfaces as drift rather than being silently re-stamped.
pub fn reconcile_hashes(pools: &mut [NodePool], claims: &mut [NodeClaim]) {
    for pool in pools.iter_mut() {
        stamp_nodepool_hash(pool);
    }
    for claim in claims.iter_mut() {
        if claim.template_hash.is_some() {
            continue;
        }
        let Some(pool_name) = claim.pool_name.as_ref() else {
            continue;
        };
        if let Some(pool) = pools.iter().find(|p| &p.name == pool_name) {
            claim.template_hash = pool.template_hash.clone();
        }
    }
}
