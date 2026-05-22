# Sweep-010 — `cave_kernel::semaphore`

**Author:** 2026-05-12 close-out
**Branch:** `claude/gracious-banach-9be8eb`
**Status:** **Landed (primitive + tests).** Live adoption in
cave-mesh deferred — the existing local code is token-bucket-shaped,
not semaphore-shaped (see §3).

## What landed

`crates/cave-kernel/src/semaphore.rs` — 160 LOC + 8 unit tests.

Public surface, matching the subset of `tokio::sync::Semaphore`
every cave module actually consumes:

* `Semaphore::new(permits)` — bounded concurrency, internally
  backed by `tokio::sync::Semaphore` so cancellation + fairness
  inherit tokio's guarantees.
* `acquire().await` returns an RAII `Permit` that releases the
  slot on drop. `Permit` is `Send` so callers can hand it across
  an `await` boundary.
* `try_acquire()` for the non-blocking path; returns
  `AcquireError::NoPermits { available, capacity }` when full so
  the caller can render a useful 429.
* `capacity()` / `available_permits()` / `in_use()` for
  introspection.

`Semaphore` is `Clone` so the same slot pool can be shared across
spawned tasks without `Arc::clone` ceremony.

## What is NOT in scope

* No fair priority queue. Tokio's default semaphore is FIFO; a
  weighted/priority variant is a follow-up.
* No "credit-weighted" acquire (cost-multiplied permit consumption).
  Cave-llm-gateway's per-request token-budget would need this; it
  stays on its local impl until weighted permits land in the kernel.

## Adoption — landed for cave-upstream daemon

`crates/cave-upstream/src/daemon.rs` previously imported
`tokio::sync::Semaphore` directly and wrapped it in
`Arc::new(Semaphore::new(...))` plus an `.expect("semaphore not
closed")` at the acquire call. With the kernel primitive:

```rust
// before
use tokio::sync::Semaphore;
let sem = Arc::new(Semaphore::new(self.cfg.concurrency.max(1)));
let _permit = sem.acquire().await.expect("semaphore not closed");

// after
use cave_kernel::semaphore::Semaphore;
let sem = Semaphore::new(self.cfg.concurrency.max(1));     // Clone+Arc internal
let _permit = sem.acquire().await;                          // infallible
```

The kernel primitive's `Clone` impl drops the redundant `Arc::new`
wrap, and its infallible `acquire()` drops the `.expect(...)`.
Behaviour identical (cancellation + fairness inherit from the inner
tokio semaphore).

`cargo test -p cave-upstream --lib` — 47/49 pass (the two failures
pre-date this change and are unrelated, confirmed by a stash + test
run on `main`).

## Adoption — deferred

The original sweep-006 deferral note flagged `cave-mesh::rate_limit`
as "semaphore-shaped". The 2026-05-12 reread of that file shows it
is actually a **token-bucket** (`TokenBucket { capacity, tokens,
refill_rate }`), not a max-concurrency semaphore. The recon was
wrong; cave-mesh does not currently have a semaphore use site.

Other candidates exist but each is a real refactor:

| Crate | Site | Shape | Why not adopted |
|-------|------|-------|-----------------|
| `cave-gateway` | per-route `max_in_flight` cap | true semaphore | Local impl pre-dates kernel; migration touches 4 routes + 12 tests. |
| `cave-mesh` | `circuit.rs` `max_pending_requests` | config-only today | Field exists in `BreakerConfig` but no in-code enforcement; adoption requires implementing the gate first, not just swapping the type. |
| `cave-rdbms-operator` | per-cluster max-concurrent failover | true semaphore | Single use site; cleanest candidate, but blocked on the Cluster CRD refactor in Paket C. |

## Tests

`cargo test -p cave-kernel --lib semaphore::` — 8 passed.
`cargo test -p cave-upstream --lib` — 47 passing (regression-free
against the 2 pre-existing failures).
