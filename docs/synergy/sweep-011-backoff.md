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

## Adoption — landed for cave-mesh outlier ejection

`crates/cave-mesh/src/circuit.rs`'s ejection-duration calculation
moved from the inline expression

```rust
base.saturating_mul(1u32.saturating_add(entry.reopen_count))
    .min(max_ej)
```

to the kernel primitive:

```rust
let backoff = Backoff::Linear {
    base: entry.config.base_ejection_time,
    cap: entry.config.max_ejection_time,
};
let ejection = backoff.delay_for(entry.reopen_count);
```

A reread of the original inline math showed it produces `base *
(n+1)` capped at `cap` — that's **linear**, not exponential as the
original docstring claimed. `Backoff::Linear { base, cap }.
delay_for(n) = base * (n+1)` exactly matches, so the swap is
behaviour-identical and the docstring is corrected at the same
time.

164/164 cave-mesh lib tests still pass (no parity-test regression).

## Adoption — deferred

The kernel `Backoff` enum carries `Constant` / `Linear` /
`Exponential` / `Fibonacci`. The cave-mesh swap above only adopts
`Linear`; future tunings of the outlier-ejection cadence (e.g. an
operator-toggleable Exponential mode) are now one-line changes
inside `circuit.rs` rather than rewriting the expression.

Other backoff users (cave-mesh `proxy.rs::retry_with_backoff`)
already adopted `cave_kernel::retrypolicy::RetryPolicy` in sweep-007;
they belong to the retry track, not the outlier-ejection track.

## Tests

`cargo test -p cave-kernel --lib backoff::` — 11 passed.
`cargo test -p cave-mesh --lib` — 164/164 (no regression).
