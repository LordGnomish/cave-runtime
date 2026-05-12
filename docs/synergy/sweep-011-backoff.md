# Sweep-011 — `cave_kernel::backoff`

**Author:** 2026-05-12 close-out
**Branch:** `claude/gracious-banach-9be8eb`
**Status:** **Landed (primitive + tests).** Live adoption in
cave-mesh outlier ejection deferred — recon below.

## What landed

`crates/cave-kernel/src/backoff.rs` — 110 LOC + 11 unit tests.

Pure-function delay schedules for the **outlier-ejection** path
(distinct from `retrypolicy::BackoffStrategy`, which is owned by
the retry executor and bundles jitter for retried operations).
This module ships strategies the Envoy-style "wait longer before
re-introducing a host" cadence needs.

`Backoff` variants:

* **`Constant(d)`** — fixed delay.
* **`Linear { base, cap }`** — `base * (n+1)`.
* **`Exponential { base, cap }`** — `base * 2^n`, capped.
* **`Fibonacci { base, cap }`** — `base * F(n+1)`, capped. The
  Fibonacci variant matches the AWS SDK recommendation for "kinder"
  growth between linear and exponential.

API:
* `delay_for(n: u32) -> Duration` — 0-indexed: `delay_for(0)` is the
  first retry's wait, not the initial attempt.
* `schedule(n: u32) -> Vec<Duration>` — convenience for the first
  `n` delays.

All variants saturate at the `cap` rather than overflowing. Calling
`delay_for(u32::MAX)` on an `Exponential` is safe and returns
`cap`. The `Fibonacci` helper internally uses `u64::saturating_add`
so the multiplier doesn't overflow until F(93).

## What is NOT in scope

* No jitter. Outlier ejection should be deterministic; the jittered
  shapes live in `retrypolicy` where they belong.
* No "decorrelated jitter" — that variant is owned by `retrypolicy`.

## Adoption — recon

The original sweep-005 deferral note identified Envoy/Istio
outlier-ejection as the consumer. cave-mesh's `circuit.rs` has the
`base_ejection_time` / `max_ejection_time` shape:

```rust
pub struct BreakerConfig {
    pub consecutive_errors: u32,
    pub base_ejection_time: Duration,
    pub max_ejection_time: Duration,
    pub max_ejection_percent: u8,
    ...
}
```

The straight-line replacement would be:

```rust
let backoff = Backoff::Exponential {
    base: cfg.base_ejection_time,
    cap: cfg.max_ejection_time,
};
let next_wait = backoff.delay_for(ejection_count);
```

Why this didn't land in the same PR: cave-mesh's circuit-breaker
state machine tracks an internal `ejection_duration: Duration`
that's mutated on each ejection cycle; swapping the calculation to
`Backoff::Exponential` is a semantic change (linear-with-doubling
vs the current pure exponential) that needs parity-test coverage.
The parity tests in `cave-mesh/tests/cilium_parity_e2e.rs` are
1759 deep; re-baselining them is real work and warrants its own
PR.

## Tests

`cargo test -p cave-kernel --lib backoff::` — 11 passed.
