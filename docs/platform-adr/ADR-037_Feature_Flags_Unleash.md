# ADR-037: Feature Flags — Unleash

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 036

## Context

CAVE needs feature flag management for progressive rollouts, A/B testing, and emergency kill switches. Feature flags decouple deployment from release — code is deployed but features activated gradually.

## Candidates

| Criteria | Unleash | LaunchDarkly | Flagsmith | OpenFeature (spec only) |
|---|---|---|---|---|
| Self-hosted | ✅ K8s Helm, PostgreSQL backend | ❌ SaaS only | ✅ | N/A (spec) |
| SDK support | ✅ Java, Python, Go, Node.js, .NET | ✅ Broadest | ✅ | ✅ (provider-agnostic) |
| Gradual rollout | ✅ Percentage-based, user-segment | ✅ | ✅ | Depends on provider |
| A/B experiments | ✅ Variants | ✅ | ✅ | Depends |
| Admin UI | ✅ Full-featured | ✅ Best-in-class | ✅ | N/A |
| Argo Rollouts integration | ✅ AnalysisRun provider | ✅ | ⚠️ | ⚠️ |
| License | Apache 2.0 (OSS) | Proprietary | BSD 3-Clause | Apache 2.0 |

## Decision

**Unleash** (self-hosted, Apache 2.0) for feature flag management. PostgreSQL backend via CNPG. Integrated with Argo Rollouts for feature-gated canary promotions. OpenFeature SDK recommended for tenant applications (provider-agnostic abstraction).

## Rejected

- **LaunchDarkly:** SaaS-only. Feature flag state stored externally — contradicts sovereign profile. Cost scales with MAU (monthly active users).
- **Flagsmith:** BSD 3-Clause (acceptable). Smaller community than Unleash. Less mature Argo Rollouts integration.

## Consequences

**Positive:**
- Self-hosted, Apache 2.0 — full control, no external dependency for feature flag state.
- Argo Rollouts integration enables feature-gated canary: deploy feature behind flag → enable for 5% → analyze → rollout.
- OpenFeature SDK abstraction prevents Unleash lock-in at application level.

**Negative:**
- PostgreSQL backend (additional DB, managed via CNPG).
- Unleash admin UI requires RBAC configuration for multi-tenant access.
- Feature flag technical debt risk — flags must be cleaned up after full rollout.

## Compliance Mapping

SOC2 CC8.1 (controlled feature release). ISO A.14.2.9 (system acceptance — progressive release).
