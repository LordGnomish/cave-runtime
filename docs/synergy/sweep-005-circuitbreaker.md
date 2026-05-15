# Sweep-005 — `cave_kernel::circuitbreaker` adoption (cave-gateway)

**Date:** 2026-05-12
**Status:** **Landed** (partial — see "Not adopted" below).
**Author:** runtime
**Predecessor:** `docs/synergy/sweep-005-006-deferral-2026-05-12.md` (recon
note that flagged the kernel-API gap blocking this sweep).

## What landed

### Kernel API extension (additive, non-breaking)

`cave_kernel::circuitbreaker` previously expressed only the resilience4j
**windowed failure-rate** model:

```rust
BreakerConfig::new(WindowKind::Count(10), 0.5, 4, Duration::from_secs(30));
```

The cave-gateway breaker used **Envoy/Istio outlier-detection** semantics:
trip after N consecutive failures, close from HalfOpen after K consecutive
successes. That semantic gap blocked the prior deferral.

This sweep adds a `TripCondition` enum so both models live in the same
primitive without behavioural collision:

```rust
pub enum TripCondition {
    WindowedRate { failure_rate_threshold: f64, minimum_calls: usize },
    Consecutive  { failure_count: u32, success_count: u32 },
}

// New constructor for the Envoy model:
BreakerConfig::consecutive(failure_count, success_count, reset_timeout);
```

Existing call sites (`BreakerConfig::new(...)`) keep their semantics —
the older constructor now sets `trip = WindowedRate { .. }`. Legacy
field-style accessors (`cfg.failure_rate_threshold()`,
`cfg.minimum_calls()`) were preserved as `pub fn` so older callers stay
green.

Also added: `PerKeyBreakers::remove(&str) -> bool`, used by adopters
that need an explicit operator-reset path (e.g. the gateway's
`CircuitBreakerRegistry::reset`).

### Adoption — cave-gateway

`crates/cave-gateway/src/circuit_breaker.rs` is now a thin wrapper around
`cave_kernel::circuitbreaker::PerKeyBreakers` configured for
`TripCondition::Consecutive`. The gateway-facing public API
(`CircuitBreakerRegistry`, `CbState`, `allow`/`on_success`/`on_failure`/
`get_state`/`reset`) is preserved verbatim — callers in `proxy.rs` and
the test suite did not change.

Behaviour preserved (regression-checked via 4 unit tests):
- Closed → Open after `failure_threshold` *consecutive* failures.
- Open → HalfOpen once `timeout` elapses, on the next `allow()`.
- HalfOpen → Closed after `success_threshold` consecutive successes.
- A single success inside the failure streak resets the trip counter.

The deletion is honest: the pre-adoption file owned a `Breaker` struct
with `failure_count`/`success_count`/`last_failure`/`open_at` fields and
the matching state machine. After the adoption only the thin
`Arc<PerKeyBreakers>` wrapper remains. Net LOC: 170 → 174 (parity tests
make up the small wash).

## Not adopted (deferred again, with reason)

### cave-mesh — Envoy outlier-detection with ejection backoff

`crates/cave-mesh/src/circuit.rs` (246 LOC) uses an **exponential
ejection backoff** on top of consecutive-failure counting:
`base_ejection_time` × `2 ^ ejection_count`, capped at
`max_ejection_time`, plus a `max_ejection_percent` cluster-wide cap.

This is faithful Istio outlier-detection. The kernel breaker (even in
the new Consecutive mode) uses a fixed `reset_timeout`. Adopting
cave-mesh would either:

1. **Lose the exponential backoff** — silently regress Istio parity. The
   parity audit explicitly tracks this behaviour, so a silent regression
   would show up as a downgrade on the dashboard.
2. **Force the kernel to grow another mode** — a `Consecutive` variant
   with `base_ejection: Duration` + `max_ejection: Duration` +
   `multiplier: f64`. That's a real kernel extension that wants its own
   recon + test plan + reviewers. Out of scope for this sweep.

Honest call: cave-mesh stays on its local breaker, gated by its own
parity tests, until a follow-up sweep is willing to negotiate the kernel
API for outlier-with-backoff.

## Tests

- Kernel: 19 / 19 pass (15 existing + 4 new):
  `breaker_consecutive_opens_after_n_failures`,
  `breaker_consecutive_success_resets_streak`,
  `breaker_consecutive_halfopen_closes_after_n_successes`,
  `breaker_legacy_accessors_match_trip_mode`.
- cave-gateway: 4 / 4 pass (3 preserved + 1 new):
  `success_resets_streak` (Envoy regression).
- `cargo check --workspace`: clean.

## Files changed

```
crates/cave-kernel/src/circuitbreaker.rs   +110 / -19   (TripCondition + remove)
crates/cave-gateway/Cargo.toml             +1          (cave-kernel dep)
crates/cave-gateway/src/circuit_breaker.rs +90 / -148  (rewritten as wrapper)
```
