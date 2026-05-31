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
        let info = self.consumers.entry(consumer_id.to_string()).or_default();
        if info.connections == 0 {
            // Newly active consumer — insert into the sorted list.
            let pos = self
                .sorted_consumers
                .binary_search(&consumer_id.to_string())
                .unwrap_or_else(|p| p);
            self.sorted_consumers.insert(pos, consumer_id.to_string());
        }
        info.connections += 1;
        info.shutting_down = false;
        self.recompute_all_consumers();
    }

    /// Mark a consumer as shutting down so it is skipped by the scheduler
    /// (mirrors `notifyQuerierShutdown`).
    pub fn notify_shutdown(&mut self, consumer_id: &str) {
        if let Some(info) = self.consumers.get_mut(consumer_id) {
            info.shutting_down = true;
        }
    }

    /// Release one connection for a consumer (mirrors
    /// `removeConsumerConnection`). The consumer stays eligible while it has
    /// remaining connections; on the last release it is fully removed and
    /// tenant shard assignments are recomputed. Returns `true` if the consumer
    /// was fully removed.
    pub fn unregister_consumer(&mut self, consumer_id: &str) -> bool {
        let remaining = match self.consumers.get_mut(consumer_id) {
            Some(info) if info.connections > 0 => {
                info.connections -= 1;
                info.connections
            }
            _ => return false,
        };
        if remaining == 0 {
            self.remove_consumer(consumer_id);
            return true;
        }
        false
    }

    /// Number of active connections held by a consumer (0 if unknown).
    pub fn consumer_connections(&self, consumer_id: &str) -> u32 {
        self.consumers
            .get(consumer_id)
            .map(|i| i.connections)
            .unwrap_or(0)
    }

    /// Fully remove a consumer and recompute tenant assignments
    /// (mirrors `removeConsumer`).
    pub fn remove_consumer(&mut self, consumer_id: &str) {
        if self.consumers.remove(consumer_id).is_some() {
            self.sorted_consumers.retain(|c| c != consumer_id);
            self.recompute_all_consumers();
        }
    }

    /// Refresh every tenant's shuffle-shard assignment after the consumer set
    /// changes (mirrors `recomputeUserConsumers`).
    fn recompute_all_consumers(&mut self) {
        let tenants: Vec<String> = self.tenants.keys().cloned().collect();
        for t in tenants {
            self.recompute_tenant_consumers(&t);
        }
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
        let mut uid = last_user_index;
        // No local queue at the RequestQueue level: -2 collapses to -1.
        if uid == START_INDEX_WITH_LOCAL_QUEUE {
            uid = START_INDEX;
        }
        // A shutting-down or unknown consumer receives no work.
        match self.consumers.get(consumer_id) {
            Some(info) if !info.shutting_down => {}
            _ => return (None, uid),
        }

        let max_iters = self.keys.len() + 1;
        for _ in 0..max_iters {
            match self.get_next(uid) {
                None => {
                    // Out of bounds — wrap to the start and retry.
                    if uid == START_INDEX {
                        break;
                    }
                    uid = START_INDEX;
                    continue;
                }
                Some((name, pos)) => {
                    uid = pos;
                    if let Some(tq) = self.tenants.get(&name) {
                        if let Some(allowed) = &tq.consumers {
                            if !allowed.contains(consumer_id) {
                                // This consumer is not sharded onto this tenant.
                                continue;
                            }
                        }
                    }
                    return (Some(name), uid);
                }
            }
        }
        (None, uid)
    }

    /// Enqueue a request for `tenant`. Errors if stopped or the tenant queue is
    /// full.
    pub fn enqueue(&mut self, tenant: &str, req: T) -> Result<(), QueueError> {
        if self.stopped {
            return Err(QueueError::Stopped);
        }
        self.get_or_add_queue(tenant)?;
        let tq = self.tenants.get_mut(tenant).expect("just created");
        if tq.queue.len() >= self.max_user_queue_size {
            return Err(QueueError::TooManyRequests);
        }
        tq.queue.push_back(req);
        Ok(())
    }

    /// Dequeue the next request fairly for `consumer_id`, resuming round-robin
    /// from `last_user_index`. Returns the `(tenant, request)` and the updated
    /// index, or `None` when no eligible request is available.
    pub fn dequeue(
        &mut self,
        last_user_index: i64,
        consumer_id: &str,
    ) -> (Option<(String, T)>, i64) {
        let (tenant, uid) = match self.next_tenant_for_consumer(last_user_index, consumer_id) {
            (Some(t), uid) => (t, uid),
            (None, uid) => return (None, uid),
        };
        let req = self
            .tenants
            .get_mut(&tenant)
            .and_then(|tq| tq.queue.pop_front());
        // Drop the tenant queue once drained so it stops consuming a round-robin
        // slot (mirrors upstream deleteQueue on empty).
        if self
            .tenants
            .get(&tenant)
            .map(|tq| tq.queue.is_empty())
            .unwrap_or(false)
        {
            self.delete_queue(&tenant);
        }
        match req {
            Some(r) => (Some((tenant, r)), uid),
            None => (None, uid),
        }
    }

    /// Batch-dequeue up to `max_items` requests from a single tenant, mirroring
    /// `DequeueMany`. It selects one tenant fairly (round-robin from
    /// `last_user_index`) and drains that tenant only — never crossing a tenant
    /// boundary within one call — returning the tenant name, the drained
    /// requests, and the updated index. The tenant queue is deleted if drained.
    pub fn dequeue_many(
        &mut self,
        last_user_index: i64,
        consumer_id: &str,
        max_items: usize,
    ) -> (Option<String>, Vec<T>, i64) {
        let (tenant, uid) = match self.next_tenant_for_consumer(last_user_index, consumer_id) {
            (Some(t), uid) => (t, uid),
            (None, uid) => return (None, Vec::new(), uid),
        };
        let mut batch = Vec::new();
        if let Some(tq) = self.tenants.get_mut(&tenant) {
            while batch.len() < max_items {
                match tq.queue.pop_front() {
                    Some(r) => batch.push(r),
                    None => break,
                }
            }
        }
        // Delete the tenant queue once it is fully drained.
        if self
            .tenants
            .get(&tenant)
            .map(|tq| tq.queue.is_empty())
            .unwrap_or(false)
        {
            self.delete_queue(&tenant);
        }
        (Some(tenant), batch, uid)
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

    // ── Cycle 3: consumer connection lifecycle + shard eligibility ──────────

    #[test]
    fn connection_count_tracks_register_and_unregister() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.register_consumer("c1"); // two connections
        assert_eq!(q.consumer_connections("c1"), 2);

        // First release keeps the consumer eligible.
        assert!(!q.unregister_consumer("c1"));
        assert_eq!(q.consumer_connections("c1"), 1);
        q.enqueue("t", 1).unwrap();
        assert!(q.dequeue(START_INDEX, "c1").0.is_some());

        // Final release fully removes it.
        q.enqueue("t", 2).unwrap();
        assert!(q.unregister_consumer("c1"));
        assert_eq!(q.consumer_connections("c1"), 0);
        // An unknown consumer is served nothing.
        assert!(q.dequeue(START_INDEX, "c1").0.is_none());
    }

    #[test]
    fn shutting_down_consumer_is_skipped() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("t", 1).unwrap();
        q.notify_shutdown("c1");
        assert!(q.dequeue(START_INDEX, "c1").0.is_none());
    }

    #[test]
    fn unknown_consumer_is_skipped() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("t", 1).unwrap();
        assert!(q.dequeue(START_INDEX, "ghost").0.is_none());
    }

    #[test]
    fn shuffle_sharding_restricts_tenant_to_assigned_consumers() {
        // 4 consumers, each tenant pinned to exactly 1 (max_consumers = 1).
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 1);
        let all: Vec<String> = ["c0", "c1", "c2", "c3"].iter().map(|s| s.to_string()).collect();
        for c in &all {
            q.register_consumer(c);
        }
        q.enqueue("tenant-a", 42).unwrap();

        // Determine the single eligible consumer deterministically.
        let eligible = shuffle_consumers_for_tenants(shuffle_shard_seed("tenant-a"), 1, &all)
            .expect("one consumer pinned");
        let eligible_id = eligible.iter().next().unwrap().clone();

        // Every other consumer is skipped for this tenant.
        for c in &all {
            if c != &eligible_id {
                assert!(
                    q.dequeue(START_INDEX, c).0.is_none(),
                    "{c} is not sharded onto tenant-a and must be skipped"
                );
            }
        }
        // The pinned consumer gets the request.
        assert_eq!(
            q.dequeue(START_INDEX, &eligible_id).0,
            Some(("tenant-a".to_string(), 42))
        );
    }

    // ── Cycle 4: batch dequeue (DequeueMany) ────────────────────────────────

    #[test]
    fn dequeue_many_batches_up_to_max_items_from_one_tenant() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        for i in 0..5 {
            q.enqueue("a", i).unwrap();
        }
        let (tenant, batch, _) = q.dequeue_many(START_INDEX, "c1", 3);
        assert_eq!(tenant, Some("a".to_string()));
        assert_eq!(batch, vec![0, 1, 2]);
        // The remaining two are still queued.
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn dequeue_many_does_not_cross_tenant_boundary() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("a", 1).unwrap();
        q.enqueue("b", 2).unwrap();
        // max_items=5 but tenant "a" only has one — must not pull from "b".
        let (tenant, batch, _) = q.dequeue_many(START_INDEX, "c1", 5);
        assert_eq!(tenant, Some("a".to_string()));
        assert_eq!(batch, vec![1]);
        assert_eq!(q.tenant_count(), 1); // "a" drained+deleted, "b" remains
    }

    #[test]
    fn dequeue_many_drains_and_deletes_tenant() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        q.enqueue("a", 1).unwrap();
        q.enqueue("a", 2).unwrap();
        let (tenant, batch, _) = q.dequeue_many(START_INDEX, "c1", 10);
        assert_eq!(tenant, Some("a".to_string()));
        assert_eq!(batch, vec![1, 2]);
        assert_eq!(q.tenant_count(), 0);
    }

    #[test]
    fn dequeue_many_empty_returns_empty() {
        let mut q: RequestQueue<u32> = RequestQueue::new(10, 0);
        q.register_consumer("c1");
        let (tenant, batch, _) = q.dequeue_many(START_INDEX, "c1", 4);
        assert_eq!(tenant, None);
        assert!(batch.is_empty());
    }
}
