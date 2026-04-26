# ADR-085: Platform Upgrade Strategy

**Status:** Accepted

**Scope:** Universal

**Category:** Operations

**Related ADRs:** 099, 127, 132, 133, 134 | Absorbs: ADR-094

## Context

73 components must be upgraded regularly (security patches, new features, upstream EOL). Upgrades must be safe, reversible, and evidence-producing.


## Candidates

| Approach | Soak windows + compatibility matrix (chosen) | Direct staging → prod | Canary-only | Blue-green immediate |
|---|---|---|---|---|
| Regression detection window | ✅ 14d control plane, 10d data | ❌ Minutes of staging testing | ⚠️ Canary catches only customer-facing | ⚠️ Depends on traffic split |
| Upgrade evidence | ✅ Upgrade Safe attestation after soak | ❌ Minimal evidence | ⚠️ Canary metrics only | ⚠️ |
| Emergency patches | ✅ Documented skip-soak path | ✅ Always direct | ⚠️ Canary may delay | ✅ |
| Rollback | ✅ Full reversal possible | ⚠️ Limited | ✅ Traffic shift | ✅ Swap environments |
| Slow-burn issue detection | ✅ Memory leaks, connection pool | ❌ Missed | ❌ Missed | ❌ Missed |


## Decision

**Upgrade flow:** Renovate PR (ADR-041) → cave-ctl upgrade check (compatibility matrix validation — ADR-133) → CI pipeline (27 stages) → staging soak (ADR-132: 14d control plane, 10d data plane, 7d tooling) → compatibility re-verification (max 90d tuple age) → prod promotion → Sovereign Ledger `Upgrade Safe` attestation.

**Dependency ordering:** cave-ctl upgrade check reads dependency-graph.yaml for topological sort. Critical chains: CNI before mesh, Crossplane before data, ArgoCD before everything.

**Emergency patches:** Guardian + Security Approver can approve skip-soak with Ledger `Emergency Upgrade` attestation. Compensating control: enhanced monitoring for 24h post-upgrade.


## Rejected Options

- **No soak windows (direct staging→prod):** Regressions caught only in staging (minutes of testing). Soak windows (days of observation) catch slow-burn issues (memory leaks, connection pool exhaustion, metric drift).
- **Manual upgrade coordination:** Unsustainable for 73 components. Renovate + cave-ctl automates detection and validation.
- **Rolling upgrades without dependency ordering:** Upgrading Istio before Cilium could break networking. Dependency graph enforces safe ordering.
- **No emergency path:** Critical CVEs can't wait 14-day soak. Emergency path exists with guardian + security approver dual-approval.


## Consequences

**Positive:**
- Every upgrade validated against compatibility matrix before promotion.
- Soak windows catch regressions before prod impact.
- Dependency ordering prevents incompatible version combinations.
- Emergency path exists for critical security patches.

**Negative:**
- Soak windows delay feature availability (14d for control plane).
- Compatibility matrix maintenance overhead (version tuple management).
- Emergency skip-soak increases risk (mitigated: enhanced monitoring).

Compliance Mapping

SOC2 CC8.1 (change management — controlled upgrades). ISO A.8.8 (vulnerability management — patching). ISO A.14.2 (secure development — upgrade validation). NIS2 Art.21 (change management).

Absorbed Decisions:

The following tool-level decisions are absorbed into this ADR for traceability

Dual SDLC Hierarchy

Decision:

Dual SDLC hierarchy: Platform SDLC (dev→staging→prod for infra changes) is independent from Tenant SDLC (each tenant has own dev→staging→prod for workloads). Neither lifecycle blocks the other.

