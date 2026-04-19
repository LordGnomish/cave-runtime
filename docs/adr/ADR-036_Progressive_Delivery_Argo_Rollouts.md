# ADR-036: Progressive Delivery — Argo Rollouts

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 026, 037

## Context

CAVE needs zero-downtime production deployments with automated canary analysis. If a new version degrades SLOs, it must be automatically rolled back without human intervention.

## Candidates

| Criteria | Argo Rollouts | Flagger | Istio VirtualService (manual) | K8s Rolling Update |
|---|---|---|---|---|
| Canary analysis | ✅ Automated (Prometheus-driven SLO analysis) | ✅ Automated | ❌ Manual | ❌ |
| Traffic shifting | ✅ Istio, Nginx, ALB, SMI | ✅ Istio, Linkerd, NGINX, Gloo | ✅ Istio | ❌ (replica-based) |
| Blue-green | ✅ | ✅ | ❌ | ❌ |
| ArgoCD integration | ✅ Native (same Argo ecosystem) | ⚠️ Separate ecosystem | N/A | ✅ Native |
| Feature flag integration | ✅ Experiment + AnalysisRun | ⚠️ | ❌ | ❌ |
| Backstage integration | ✅ Argo Rollouts plugin | ⚠️ | ❌ | N/A |
| License | Apache 2.0 | Apache 2.0 | Apache 2.0 | K8s native |

## Decision

**Argo Rollouts** for canary deployments on production. Integrated with Istio ambient for traffic shifting. Automated canary analysis via Prometheus metrics (error rate, latency P99, SLO compliance). Auto-rollback if analysis fails. Used with **Unleash** feature flags (ADR-037) for feature-gated rollouts.

## Rejected

- **Flagger:** Capable but different ecosystem from ArgoCD. Running Flagger alongside ArgoCD + Argo Rollouts + Argo Workflows adds ecosystem fragmentation.
- **Manual Istio VirtualService:** No automated analysis. Human must watch metrics and decide. Doesn't scale for multi-tenant platform.
- **K8s Rolling Update:** No canary analysis. No traffic shifting. All-or-nothing rollout. Risky for production.

## Consequences

**Positive:**
- Zero-downtime deployments with automated canary analysis.
- Auto-rollback on SLO regression — no human in the loop for common cases.
- Same Argo ecosystem (Rollouts, CD, Workflows) — consistent tooling.
- Istio ambient traffic shifting — no sidecar needed.

**Negative:**
- Argo Rollouts CRD replaces K8s Deployment — learning curve for developers.
- Canary analysis requires well-defined SLO metrics per service.
- Istio traffic shifting adds latency measurement complexity.

## Compliance Mapping

SOC2 CC8.1 (controlled change deployment). ISO A.14.2.9 (system acceptance testing — canary as production testing). NIS2 Art.21 (change management).
