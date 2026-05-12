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

## Adoption — recon, then deferred

The original sweep-006 deferral note flagged `cave-mesh::rate_limit`
as "semaphore-shaped". The 2026-05-12 reread of that file shows it
is actually a **token-bucket** (`TokenBucket { capacity, tokens,
refill_rate }`), not a max-concurrency semaphore. The recon was
wrong; cave-mesh does not currently have a semaphore use site.

Other candidates exist but each is a real refactor:

| Crate | Site | Shape | Why not adopted |
|-------|------|-------|-----------------|
| `cave-gateway` | per-route `max_in_flight` cap | true semaphore | Local impl pre-dates kernel; migration touches 4 routes + 12 tests. |
| `cave-mesh` | `circuit.rs` `max_pending_requests` | true semaphore | Coupled to circuit-breaker state; needs careful sequencing. |
| `cave-rdbms-operator` | per-cluster max-concurrent failover | true semaphore | Single use site; cleanest candidate, but blocked on the Cluster CRD refactor in Paket C. |

The primitive is the prerequisite; each adoption ticket can land
in its own PR with its own test re-baseline.

## Tests

`cargo test -p cave-kernel --lib semaphore::` — 8 passed.
