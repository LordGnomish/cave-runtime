# Sweep-007 — `cave_kernel::retrypolicy` adoption

**Author:** Sweep-007 close-out (2026-05-12)
**Branch:** `claude/gracious-banach-9be8eb`
**Owner:** runtime
**Honest budget consumed:** ~25 min recon + 10 min implementation. Tiny
landed footprint — the recon found that the workspace doesn't currently
host the kind of retry-executor call sites a sweep can adopt.
**Status:** Landed.

## 1. Premise

The Charter "adoption ratio" KPI counts crates that import a
`cave_kernel::*` primitive instead of rolling their own. Before this
sweep, only the parity / codec / identity / ns primitives had real
adopters; `retrypolicy` was kernel-only.

The sweep brief named three candidate sites:

- `cave-mesh/src/proxy.rs` — `retry_with_backoff` helper
- `cave-mesh/src/xds.rs` — `XdsRetryPolicy` struct
- `cave-pipelines/src/models.rs` — `RetryPolicy` struct

## 2. Recon

| Site | Shape | Verdict |
|------|-------|---------|
| `cave-mesh/proxy::retry_with_backoff` | free function returning `Vec<Duration>`; **no callers** in src or tests | **Adopt** — replace body with `cave_kernel::retrypolicy::RetryPolicy::schedule` |
| `cave-mesh/xds::XdsRetryPolicy` | xDS wire struct (Envoy retry config) | **Keep** — wire-format mirror, not a runtime executor |
| `cave-pipelines::RetryPolicy` | Tekton CRD spec field (`limit`, `retry_after`) | **Keep** — CRD mirror, not a runtime executor |

So only one of the three was an honest adoption target, and that one
had no callers at all. The dead-code option (delete) and the adopt
option (replace internals) both passed Charter rule #1 (no stubs); we
chose adoption so the kernel primitive gains a documented user that
exercises `BackoffStrategy::Exponential`.

## 3. Change

`crates/cave-mesh/src/proxy.rs::retry_with_backoff` now constructs a
`cave_kernel::retrypolicy::RetryPolicy` with
`BackoffStrategy::Exponential { base, cap }` and returns
`policy.schedule(&mut deterministic_rng)`. The public signature
(`attempts: u32, base_ms: u64, max_ms: u64`) is preserved.

The `RetryPolicy::schedule` API yields `max_attempts - 1` delays, so the
helper requests `attempts + 1` to keep its caller-facing count exact.
A `StdRng::seed_from_u64(0)` keeps the schedule reproducible (the
no-jitter `Exponential` strategy doesn't consume the rng, but the API
still requires one).

`cave-mesh/Cargo.toml` picks up `rand = { workspace = true }`.

## 4. What we did not touch and why

- `XdsRetryPolicy` / `cave-pipelines::RetryPolicy` — both are wire /
  CRD mirrors, not runtime policies. Renaming or unifying them with
  the kernel would conflate the upstream-protocol contract with the
  internal execution contract. Out of scope.
- A real retry executor wired into the proxy fast-path — that would be
  a feature add, not a sweep. Tracked as a follow-up if mesh ever
  flips its retry behaviour from "declarative-only" to "active".

## 5. Adoption delta

| Primitive | Crates importing before | Crates importing after |
|-----------|------------------------:|-----------------------:|
| `cave_kernel::retrypolicy` | 0 | 1 (`cave-mesh`) |

The sweep moves the workspace adoption ratio for `retrypolicy` from
0/93 to 1/93 — a single honest tick rather than a fabricated bulk
update.

## 6. Test surface

`cargo test -p cave-mesh --lib` — 164 passed, 0 failed.
