<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- needs Burak verify: premise is Hetzner/Azure two-provider split. OSS Cave Runtime is provider-agnostic; reassess whether the onboarding-time choice still applies. -->
# ADR-066: Tenant Provider Choice at Onboarding

**Status:** Accepted

**Scope:** Universal

**Category:** Infrastructure

**Related ADRs:** 001, 002, 135

## Context

CAVE supports two providers. Should tenants be able to choose? Can they switch later? Can workloads span providers?


## Candidates

| Approach | Permanent choice at onboarding (chosen) | Live cross-cloud (active-active) | Cold migration with hybrid | No choice (single provider) |
|---|---|---|---|---|
| Data residency clarity | ✅ Explicit per tenant | ⚠️ Complex (where does data live?) | ⚠️ | ✅ |
| Network complexity | ✅ None (no cross-cloud traffic) | ❌ VPN/peering, latency | ⚠️ Periodic migration | ✅ |
| Split-brain risk | ✅ None | ❌ Real risk | ⚠️ During migration only | ✅ None |
| Tenant cost model | ✅ Predictable per provider | ❌ Unclear cost attribution | ⚠️ Migration cost visible | ✅ Predictable |
| Provider exit strategy | ✅ Migration path documented | ⚠️ Entangled state | ✅ | ❌ None |


## Decision

Tenant chooses Hetzner or Azure at onboarding. Choice is **permanent by default** — no live cross-cloud traffic. Cross-cloud cold migration supported as planned operation with downtime (see Recovery Contracts). Architecture is provider-extensible (new cloud = new Crossplane Compositions + tfvars).


## Rejected Options

- **Live cross-cloud migration:** Network latency between Hetzner and Azure makes real-time data sync impractical. Split-brain risk. Kafka cross-cluster replication adds massive complexity. Data residency violations possible.
- **Hybrid split-workload (compute on one, data on other):** Cross-provider network costs. Latency kills performance. Data residency unclear. Phase 4 future evaluation only.
- **No choice (Azure only / Hetzner only):** Eliminates cost advantage of Hetzner for internal workloads. Eliminates enterprise SLA of Azure for paying tenants.


## Consequences

(+) Clear boundary. Provider parity tested (ADR-135). Annual portability drill proves migration works. Exit strategy real, not theoretical.
(-) Tenant locked to provider after onboarding (mitigated: cold migration available). Cross-provider data sharing not possible without explicit migration. Kafka offset migration not supported (consumers must replay).

Compliance Mapping

GDPR Art.44-49 (data residency choice at onboarding), SOC2 CC6.1 (provider access controls).

