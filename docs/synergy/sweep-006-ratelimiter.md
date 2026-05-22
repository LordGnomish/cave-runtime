# Sweep-006 — `cave_kernel::ratelimiter` adoption (cave-llm-gateway)

**Date:** 2026-05-12
**Status:** **Landed** (partial — see "Not adopted" below).
**Author:** runtime
**Predecessor:** `docs/synergy/sweep-005-006-deferral-2026-05-12.md` (recon
note that flagged the kernel-API gap blocking this sweep).

## What landed

### Kernel API extension (additive, non-breaking)

`cave_kernel::ratelimiter::TokenBucket` previously exposed
`try_consume(n: f64) -> bool`. Cost-weighting was already supported
(any positive `n` works), but callers that wanted to surface a `Retry-After`
HTTP header had to re-derive the wait time locally from the deficit.

This sweep adds the wait-aware variant:

```rust
impl TokenBucket {
    pub fn try_consume_or_retry(&self, n: f64) -> Result<(), Duration>;
    pub fn try_consume_or_retry_at(&self, n: f64, now: Instant) -> Result<(), Duration>;
}
```

On exhaustion the error carries the duration the caller must wait
before `n` tokens will be available given the current refill rate. The
1ms floor prevents callers from receiving a zero retry hint on
sub-millisecond deficits.

The kernel's pre-existing cost-weighted consumption (`try_consume(cost)`)
plus this Retry-After-aware wrapper covers the cave-llm-gateway use case
the deferral note flagged ("kernel has no cost-weighted variant") — the
gap was the retry-after surface, not cost weighting itself.

### Adoption — cave-llm-gateway

`crates/cave-llm-gateway/src/rate_limit.rs` now uses the kernel
`TokenBucket` as its bucket primitive. The two buckets per consumer
(request bucket + cost-weighted token bucket) are kernel buckets sized
for that consumer's effective `RateLimit`. Per-consumer custom limits
remain on the gateway side because the kernel's `PerTenant<B>` only
supports a single factory — adopting that surface would require
extending `PerTenant` with a "replace one tenant's config" hook, which
is more invasive than the gateway's local override map.

Public API preserved (regression-checked via 5 unit tests):
- `RateLimiter::new(default_limit)`, `Default`.
- `set_limit(consumer, RateLimit)` / `get_limit(consumer)`.
- `check(consumer, token_cost) -> GatewayResult<()>` — returns
  `GatewayError::RateLimitExceeded { consumer, retry_after_ms }` on
  exhaustion of either bucket.
- `reset(consumer)`, `list_consumers()`.

Behaviour preserved (regression-checked):
- Burst capacity = `requests_per_minute` / `tokens_per_minute` (the
  bucket starts full).
- Refill = `cap / 60` tokens-per-second (per-minute → per-second).
- Setting a custom limit drops the consumer's buckets so they re-sized
  on the next call.
- `retry_after_ms` is non-zero on rejection (Retry-After regression
  test added).

Net LOC: 166 → 173 (the local `TokenBucket` struct + `try_consume` are
gone, replaced by the kernel-facing wrapper and the `duration_to_ms_ceil`
helper).

## Not adopted (deferred again, with reason)

### cave-gateway/src/plugins/rate_limiting.rs — per-route token bucket with header key

Per-route token bucket with a `header_extractor` to compute the bucket
key from the incoming request. Closest match to the kernel primitive,
but the deferral chain says "migrate cave-gateway and cave-mesh first";
this sweep deliberately scoped to the most novel case (cave-llm-gateway,
cost-weighted) so kernel-API growth could be motivated by the hardest
adopter first. The gateway adoption is a follow-up.

### cave-mesh/src/rate_limit.rs — per-destination semaphore

Not a token bucket. A concurrency limit ("max N in-flight per dst")
which the kernel currently doesn't ship. Adoption would require a
`Semaphore` primitive in `cave_kernel::ratelimiter`. Out of scope.

### cave-streams/src/quota.rs — per-topic with replenishment cadence

Different shape (cadence-based, not continuous refill). Out of scope.

### cave-trace/src/sampling.rs — probabilistic, not rate limiting

The deferral note called this out: the module is mis-named upstream; it
implements probabilistic sampling, not rate limiting. Adopting it via
the kernel rate limiter would be semantically wrong.

## Tests

- Kernel: 19 / 19 pass (15 existing + 4 new):
  `ratelimiter_token_or_retry_ok_when_available`,
  `ratelimiter_token_or_retry_returns_wait_on_exhaustion`,
  `ratelimiter_token_or_retry_supports_cost_weighted_consume`,
  `ratelimiter_token_or_retry_zero_cost_is_free`.
- cave-llm-gateway: 5 / 5 pass (4 preserved + 1 new):
  `rejection_carries_nonzero_retry_after`.
- `cargo check --workspace`: clean.

## Files changed

```
crates/cave-kernel/src/ratelimiter.rs       +84 / -2   (try_consume_or_retry + 4 tests)
crates/cave-llm-gateway/Cargo.toml          +1         (cave-kernel dep)
crates/cave-llm-gateway/src/rate_limit.rs   +98 / -84  (rewritten on kernel primitive)
```
