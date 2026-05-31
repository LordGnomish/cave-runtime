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
    // FNV-1a 64-bit.
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for b in tenant.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// splitmix64 — a deterministic, well-spread PRNG used to drive the
/// shuffle-shard selection from a fixed seed. Replaces Go's `math/rand` source
/// (load spreading does not require a specific PRNG, only determinism).
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
    if select == 0 || sorted.len() <= select {
        return None;
    }

    let mut result = HashSet::with_capacity(select);
    let mut state = seed;
    // Fisher-Yates partial selection: pick `select` items, swapping each chosen
    // item to the tail so it cannot be picked again (mirrors upstream).
    let mut scratch: Vec<String> = sorted.to_vec();
    let mut last = scratch.len() - 1;
    for _ in 0..select {
        let r = (splitmix64(&mut state) % (last as u64 + 1)) as usize;
        result.insert(scratch[r].clone());
        scratch.swap(r, last);
        last -= 1;
    }
    Some(result)
}

/// One tenant's sub-queue plus its shuffle-shard state.
struct TenantQueue<T> {
    name: String,
    /// Deterministic seed for this tenant's consumer shuffle.
    seed: u64,
    /// Position in the mapping `keys` slice.
    pos: i64,
    /// `Some` => only these consumers may dequeue this tenant's requests.
    /// `None` => all consumers are eligible.
    consumers: Option<HashSet<String>>,
    /// FIFO of pending requests for this tenant.
    queue: VecDeque<T>,
}

/// Per-consumer (querier) connection bookkeeping.
#[derive(Default)]
struct ConsumerInfo {
    connections: u32,
    shutting_down: bool,
}

/// In-process fair-share request queue, the algorithmic core of Loki's
/// query-scheduler. Generic over the request payload `T`.
pub struct RequestQueue<T> {
    /// Tombstoned key slice — `""` marks a removed slot, reusable on insert.
    keys: Vec<String>,
    tenants: HashMap<String, TenantQueue<T>>,
    consumers: HashMap<String, ConsumerInfo>,
    sorted_consumers: Vec<String>,
    max_user_queue_size: usize,
    /// Max consumers pinned per tenant via shuffle-sharding (0 => all eligible).
    max_consumers: usize,
    stopped: bool,
}

impl<T> RequestQueue<T> {
    /// Create a queue capped at `max_user_queue_size` pending requests per
    /// tenant. `max_consumers == 0` disables shuffle-sharding (all consumers
    /// eligible for every tenant).
    pub fn new(max_user_queue_size: usize, max_consumers: usize) -> Self {
        Self {
            keys: Vec::new(),
            tenants: HashMap::new(),
            consumers: HashMap::new(),
            sorted_consumers: Vec::new(),
            max_user_queue_size,
            max_consumers,
            stopped: false,
        }
    }

    /// Number of live (non-tombstoned) tenant queues.
    pub fn tenant_count(&self) -> usize {
        self.tenants.len()
    }

    /// Total pending requests across all tenants.
    pub fn len(&self) -> usize {
        self.tenants.values().map(|t| t.queue.len()).sum()
    }

    /// Whether the queue holds no pending requests.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Stop the queue; subsequent enqueues return [`QueueError::Stopped`].
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// Register a consumer connection (mirrors `addConsumerToConnection`),
    /// keeping `sorted_consumers` sorted and refreshing every tenant's
    /// shuffle-shard assignment.
    pub fn register_consumer(&mut self, consumer_id: &str) {
        let _ = consumer_id; // RED stub
    }

    /// Mark a consumer as shutting down so it is skipped by the scheduler
    /// (mirrors `notifyQuerierShutdown`).
    pub fn notify_shutdown(&mut self, consumer_id: &str) {
        let _ = consumer_id; // RED stub
    }

    /// Fully remove a consumer and recompute tenant assignments
    /// (mirrors `removeConsumer`).
    pub fn remove_consumer(&mut self, consumer_id: &str) {
        let _ = consumer_id; // RED stub
    }

    /// Recompute the shuffle-shard consumer subset for `tenant`.
    fn recompute_tenant_consumers(&mut self, tenant: &str) {
        let select = self.max_consumers.min(self.sorted_consumers.len());
        let select = if self.max_consumers == 0 { 0 } else { select };
        if let Some(tq) = self.tenants.get(tenant) {
            let seed = tq.seed;
            let consumers =
                shuffle_consumers_for_tenants(seed, select, &self.sorted_consumers);
            if let Some(tq) = self.tenants.get_mut(tenant) {
                tq.consumers = consumers;
            }
        }
    }

    /// Get or create the tenant's queue, refreshing its consumer assignment.
    fn get_or_add_queue(&mut self, tenant: &str) -> Result<(), QueueError> {
        if tenant.is_empty() {
            return Err(QueueError::EmptyTenant);
        }
        if !self.tenants.contains_key(tenant) {
            // Reuse a tombstoned slot if available, else append.
            let pos = match self.keys.iter().position(|k| k.is_empty()) {
                Some(i) => {
                    self.keys[i] = tenant.to_string();
                    i as i64
                }
                None => {
                    self.keys.push(tenant.to_string());
                    (self.keys.len() - 1) as i64
                }
            };
            self.tenants.insert(
                tenant.to_string(),
                TenantQueue {
                    name: tenant.to_string(),
                    seed: shuffle_shard_seed(tenant),
                    pos,
                    consumers: None,
                    queue: VecDeque::new(),
                },
            );
        }
        self.recompute_tenant_consumers(tenant);
        Ok(())
    }

    /// Remove a tenant queue, leaving a reusable tombstone in `keys`.
    fn delete_queue(&mut self, tenant: &str) {
        if let Some(tq) = self.tenants.remove(tenant) {
            let pos = tq.pos as usize;
            if pos < self.keys.len() {
                self.keys[pos] = String::new();
            }
        }
    }

    /// Advance from `idx` to the next live tenant in `keys`, mirroring
    /// `Mapping.GetNext`: returns the tenant name and its position, or `None`
    /// (out of bounds) when no live slot follows `idx`.
    fn get_next(&self, idx: i64) -> Option<(String, i64)> {
        if self.tenants.is_empty() {
            return None;
        }
        let mut i = idx + 1;
        while (i as usize) < self.keys.len() {
            let k = &self.keys[i as usize];
            if !k.is_empty() {
                return Some((k.clone(), i));
            }
            i += 1;
        }
        None
    }

    /// Find the next tenant queue this consumer may serve, starting after
    /// `last_user_index`. Returns the chosen tenant name and the updated index
    /// to pass on the next call. Mirrors `getNextQueueForConsumer`.
    pub fn next_tenant_for_consumer(
        &self,
        last_user_index: i64,
        consumer_id: &str,
    ) -> (Option<String>, i64) {
        let _ = (last_user_index, consumer_id);
        (None, START_INDEX) // RED stub
    }

    /// Enqueue a request for `tenant`. Errors if stopped or the tenant queue is
    /// full.
    pub fn enqueue(&mut self, tenant: &str, req: T) -> Result<(), QueueError> {
        let _ = (tenant, req);
        Err(QueueError::Stopped) // RED stub
    }

    /// Dequeue the next request fairly for `consumer_id`, resuming round-robin
    /// from `last_user_index`. Returns the `(tenant, request)` and the updated
    /// index, or `None` when no eligible request is available.
    pub fn dequeue(
        &mut self,
        last_user_index: i64,
        consumer_id: &str,
    ) -> (Option<(String, T)>, i64) {
        let _ = (last_user_index, consumer_id);
        (None, START_INDEX) // RED stub
    }
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

    // ── Cycle 2: enqueue / dequeue / fair round-robin / limits ──────────────

    #[test]
    fn enqueue_rejected_when_stopped() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.stop();
        assert_eq!(q.enqueue("t1", 1), Err(QueueError::Stopped));
    }

    #[test]
    fn enqueue_rejects_empty_tenant() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        assert_eq!(q.enqueue("", 1), Err(QueueError::EmptyTenant));
    }

    #[test]
    fn enqueue_enforces_per_tenant_limit() {
        let mut q: RequestQueue<u32> = RequestQueue::new(2, 0);
        assert!(q.enqueue("t1", 1).is_ok());
        assert!(q.enqueue("t1", 2).is_ok());
        assert_eq!(q.enqueue("t1", 3), Err(QueueError::TooManyRequests));
        // A different tenant has its own independent budget.
        assert!(q.enqueue("t2", 9).is_ok());
    }

    #[test]
    fn dequeue_is_fifo_within_a_tenant() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("t1", 10).unwrap();
        q.enqueue("t1", 11).unwrap();
        let (a, idx) = q.dequeue(START_INDEX, "c1");
        assert_eq!(a, Some(("t1".to_string(), 10)));
        let (b, _) = q.dequeue(idx, "c1");
        assert_eq!(b, Some(("t1".to_string(), 11)));
    }

    #[test]
    fn dequeue_round_robins_across_tenants() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("a", 1).unwrap();
        q.enqueue("a", 2).unwrap();
        q.enqueue("b", 3).unwrap();
        q.enqueue("b", 4).unwrap();
        // Threading the returned index round-robins one tenant per dequeue.
        let mut idx = START_INDEX;
        let mut order = Vec::new();
        for _ in 0..4 {
            let (got, ni) = q.dequeue(idx, "c1");
            idx = ni;
            order.push(got.unwrap().0);
        }
        // a, b, a, b — strict alternation, no starvation.
        assert_eq!(order, vec!["a", "b", "a", "b"]);
    }

    #[test]
    fn empty_tenant_queue_is_deleted_and_slot_reused() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("a", 1).unwrap();
        assert_eq!(q.tenant_count(), 1);
        let (_, idx) = q.dequeue(START_INDEX, "c1");
        // Draining the last entry removes the tenant queue.
        assert_eq!(q.tenant_count(), 0);
        assert!(q.dequeue(idx, "c1").0.is_none());
        // Slot is reusable for a new tenant.
        q.enqueue("b", 2).unwrap();
        assert_eq!(q.tenant_count(), 1);
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        assert!(q.dequeue(START_INDEX, "c1").0.is_none());
    }
}
