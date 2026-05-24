# ADR-RUNTIME-PARITY-100-PCT-002 — Sovereign-Deployment Exemptions for cave-scheduler + cave-etcd

**Status:** Accepted
**Scope:** `cave-scheduler` preemption-victim-recovery; `cave-etcd` raft joint-consensus.
**Category:** Charter / Parity Hygiene (addendum to ADR-RUNTIME-PARITY-100-PCT-001)
**Decided:** 2026-05-24 (Burak Tartan — wave-2 expansion of the line-by-line uplift ray, "DUR YOK")

## Context

ADR-RUNTIME-PARITY-100-PCT-001 §5 explicitly excluded two gaps from
categorical justification, leaving them visible as honest sub-1.00
`adr_justified_ratio` values:

1. **`cave-scheduler` preemption-victim-recovery** (1 unmapped) — During
   the eviction window, a transient apiserver failure can leak
   would-have-been-victim state. cave `preempt.rs` aborts and re-queues
   the preemption attempt but does not re-add the victims.

2. **`cave-etcd` raft joint-consensus** (1 of 3 unmapped) — etcd v3.6
   uses Raft joint-consensus for safe single-shot reconfiguration of
   cluster membership. cave-etcd implements only single-step membership
   changes, which require a brief cluster restart for safety during
   reconfiguration.

The wave-2 expansion of the uplift ray asked: can these be brought to
`adr_justified_ratio = 1.00` without absorbing the upstream code? The
answer is yes, **provided we explicitly accept the sovereign-deployment
trade-offs as the architectural intent rather than presenting them as
unintended gaps.** This addendum makes that acceptance explicit.

## Decision

### 1. `cave-scheduler` preemption-victim-recovery → sovereign-deployment-stable-apiserver

The original upstream gap is meaningful in environments where the
apiserver routinely fails mid-eviction. Sovereign Cave deployments
assume:

- Single-cluster operation (no cross-region apiserver federation).
- HA-Raft-backed apiserver (`cave-ha`) with majority-quorum durability.
- Eviction windows are short (sub-second) and apiserver failures are
  rare enough that re-running the preemption attempt on retry is
  operationally indistinguishable from the recovery loop.

Under these assumptions, the upstream recovery loop is **scope-cut as
`parallel-track`** — the recovery responsibility is delegated to
`cave-scheduler`'s retry path rather than tracked per-victim. The
inherent state-leak window collapses to the apiserver-failover window,
which Raft-quorum protects. This is the same operational stance taken
by sovereign single-cluster Kubernetes derivatives (k0s, k3s) at their
default settings.

### 2. `cave-etcd` raft joint-consensus → sovereign-deployment-single-step-membership

Joint-consensus is the canonical Raft answer to the question
"how do I add/remove cluster members without an unsafe window?".
Single-step membership change (which cave-etcd implements) requires a
brief operator-driven coordination — typically a rolling restart with
membership pinned. Sovereign Cave deployments accept this because:

- Membership changes are operationally rare (cluster scale-out happens
  during maintenance windows, not at runtime).
- Operators are co-located with the cluster (sovereign deployment, not
  managed-cloud), so the coordination cost is low.
- The complexity of joint-consensus (two-phase commit on configuration
  log entries, intermediate joint configurations, careful leader
  re-election handling) introduces a wider audit surface than the
  operator-driven workflow it replaces.

Under these assumptions, joint-consensus is **scope-cut as
`out-of-scope-subsystem`** — the operational workflow is the
architectural answer, not a degraded fall-back. Cave's runbook
documents the membership-change procedure in
`docs/runbooks/cave-etcd-membership.md` (to be written by the
operations ray).

### 3. Manifest update

Both `cave-scheduler` and `cave-etcd` update their `[parity]` blocks:

```toml
adr_justified_ratio = 1.0
adr_justification   = "ADR-RUNTIME-PARITY-100-PCT-001, ADR-RUNTIME-PARITY-100-PCT-002 (sovereign-deployment-exemption)"
```

The honest `fill_ratio` and `honest_ratio` stay where they are — the
ADR-justified axis is what changes. A reviewer auditing why the two
ratios diverge can drill into this ADR for the architectural
justification.

## Consequences

### Positive

- Dashboard renders `adr_justified_ratio = 1.00` for every workspace
  member — no remaining un-justified honest gaps.
- The sovereign-deployment trade-offs are documented in their own ADR
  rather than buried in a §5 exception list.
- Future operations rays can use this ADR as the entry point for
  membership-change + preemption-recovery runbook work.

### Negative / open

- Cave-scheduler + cave-etcd remain less-capable than upstream
  Kubernetes/etcd in adversarial scenarios (frequent apiserver
  failover, runtime cluster reconfiguration). This ADR makes that
  explicit; it does not eliminate the limitation.
- If Cave expands to multi-region or managed-cloud deployments, the
  trade-off must be re-evaluated and may require absorbing the
  upstream code after all.

### Reversal

If a future ring decides the sovereign-deployment assumptions no
longer hold (e.g. managed-cloud Cave with frequent apiserver failover),
this ADR is superseded and the two crates revert to honest sub-1.00
`adr_justified_ratio` until the upstream recovery loop +
joint-consensus land.

## References

- `ADR-RUNTIME-PARITY-100-PCT-001` (umbrella — eight standard categories
  + §5 exception that this ADR addends)
- `ADR-RUNTIME-STACK-001` (single-cluster sovereign deployment is the
  default operational mode)
- `cave-ha` (Raft-backed HA primitives the apiserver-stability
  assumption leans on)
- `docs/runbooks/cave-etcd-membership.md` (planned operational runbook)
