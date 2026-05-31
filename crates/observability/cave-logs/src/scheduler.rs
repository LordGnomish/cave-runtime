// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query-scheduler fair-share request queue.
//!
//! In-process port of grafana/loki `pkg/queue` (queue.go + tenant_queues.go +
//! mapping.go, pinned v3.4.0). Loki's scheduler fans queries across queriers
//! using a per-tenant fair-share queue: requests are bucketed per tenant, a
//! round-robin index (`lastUserIndex`) advances one tenant per dequeue so no
//! single tenant can starve the others, and shuffle-sharding optionally pins a
//! deterministic subset of consumers (queriers) to each tenant for load
//! spreading + isolation.
//!
//! The cross-process gRPC transport between query-frontend → scheduler →
//! querier stays out of scope (single-process cave-logs); this module ports the
//! scheduling *algorithm* itself, which is purely in-process.
//!
//! Upstream seed derivation uses md5 purely as a predictable, non-cryptographic
//! hash (`#nosec G401 -- intentionally predictable value`). We seed from FNV-1a
//! 64-bit instead to avoid a crypto dependency; shuffle-sharding only requires
//! deterministic, well-spread selection, not collision resistance.

use std::collections::{HashMap, HashSet, VecDeque};

/// Start index for round-robin iteration over tenant sub-queues.
/// Mirrors `StartIndex = -1` in queue.go.
pub const START_INDEX: i64 = -1;
/// Start index that also visits a local queue first. At the RequestQueue level
/// there is no local queue, so this collapses to [`START_INDEX`].
/// Mirrors `StartIndexWithLocalQueue = -2`.
pub const START_INDEX_WITH_LOCAL_QUEUE: i64 = -2;

/// Errors returned by [`RequestQueue::enqueue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    /// The queue has been stopped; no further enqueues are accepted.
    Stopped,
    /// The tenant's queue is at `max_user_queue_size`.
    TooManyRequests,
    /// An empty tenant id was supplied ("" is reserved as the tombstone slot).
    EmptyTenant,
}

/// FNV-1a 64-bit seed derived from a tenant id. Deterministic and consistent
/// for a given tenant, mirroring `util.ShuffleShardSeed(tenantID, "")`.
pub fn shuffle_shard_seed(tenant: &str) -> u64 {
    let _ = tenant;
    0 // RED stub
}

/// Deterministically select `select` consumers out of `sorted` for a tenant.
///
/// Returns `None` (meaning "all consumers are eligible") when `select == 0` or
/// there are not strictly more consumers than the selection size — mirrors
/// `shuffleConsumersForTenants`.
pub fn shuffle_consumers_for_tenants(
    seed: u64,
    select: usize,
    sorted: &[String],
) -> Option<HashSet<String>> {
    let _ = (seed, select, sorted);
    None // RED stub
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("querier-{i}")).collect()
    }

    #[test]
    fn seed_is_deterministic_and_tenant_specific() {
        assert_eq!(shuffle_shard_seed("tenant-a"), shuffle_shard_seed("tenant-a"));
        assert_ne!(shuffle_shard_seed("tenant-a"), shuffle_shard_seed("tenant-b"));
    }

    #[test]
    fn shuffle_returns_none_when_select_zero() {
        assert!(shuffle_consumers_for_tenants(123, 0, &names(5)).is_none());
    }

    #[test]
    fn shuffle_returns_none_when_not_enough_consumers() {
        // len <= select -> nil (all eligible)
        assert!(shuffle_consumers_for_tenants(123, 5, &names(5)).is_none());
        assert!(shuffle_consumers_for_tenants(123, 6, &names(5)).is_none());
    }

    #[test]
    fn shuffle_selects_exact_subset() {
        let all = names(10);
        let sel = shuffle_consumers_for_tenants(shuffle_shard_seed("t"), 3, &all)
            .expect("subset");
        assert_eq!(sel.len(), 3);
        for c in &sel {
            assert!(all.contains(c), "selected {c} must come from the input set");
        }
    }

    #[test]
    fn shuffle_is_deterministic_for_same_seed() {
        let all = names(10);
        let seed = shuffle_shard_seed("tenant-x");
        let a = shuffle_consumers_for_tenants(seed, 4, &all).unwrap();
        let b = shuffle_consumers_for_tenants(seed, 4, &all).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn shuffle_differs_across_seeds() {
        let all = names(20);
        let a = shuffle_consumers_for_tenants(shuffle_shard_seed("tenant-1"), 4, &all).unwrap();
        let b = shuffle_consumers_for_tenants(shuffle_shard_seed("tenant-2"), 4, &all).unwrap();
        assert_ne!(a, b, "different tenants should shard to different querier subsets");
    }
}
