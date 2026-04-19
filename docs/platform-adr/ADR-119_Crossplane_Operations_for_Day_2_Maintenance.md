# ADR-119: Crossplane Operations for Day-2 Maintenance

**Status:** Accepted

**Scope:** Azure, Universal

**Category:** Platform

**Related ADRs:** 067, 095

## Context

Day-2 infrastructure maintenance (DB vacuum, cert monitoring, index optimization) should be declarative and managed by Crossplane, not external cron scripts.

## Candidates

| Approach | Crossplane Operations | External CronJobs | Argo Workflows | Manual |
|---|---|---|---|---|
| K8s native | ✅ Crossplane CRDs | ⚠️ K8s CronJob (no Crossplane context) | ✅ CRDs | ❌ |
| Crossplane XR context | ✅ Can reference XR state | ❌ | ❌ | ❌ |
| Scheduling | ✅ CronOperation | ✅ CronJob | ✅ CronWorkflow | ❌ |
| Event-driven | ✅ WatchOperation | ❌ | ⚠️ Argo Events | ❌ |

## Decision

CronOperation (scheduled: weekly DB vacuum, monthly index optimization). WatchOperation (event-driven: cert approaching expiry, resource drift). Simple single-resource remediation. Complex multi-step → Reflex Engine (ADR-095). **Alpha stability fallback:** Every Operations CRD has mirror Reflex playbook. Stability exit: 2 consecutive Crossplane minors with no Operations breaking changes → promote to prod-mandatory.

## Rejected

- **External CronJobs:** No Crossplane context. Cannot reference XR state. Manual RBAC. Not part of GitOps reconciliation.
- **Argo Workflows for everything:** Overkill for simple tasks (DB vacuum is a single command). Argo Workflows overhead justified for complex multi-step remediation only.
- **Manual maintenance:** Doesn't scale. Human forgets, schedule drifts.

## Consequences

**Positive:**
- Day-2 maintenance is declarative and GitOps-managed.
- CronOperation and WatchOperation cover scheduled and event-driven patterns.
- Crossplane XR context enables operations that know about the resource they're maintaining.
- Alpha fallback ensures no remediation gap if Operations API breaks.

**Negative:**
- Crossplane Operations is alpha/beta — API may change between releases.
- Mirror Reflex playbooks double the maintenance for each operation (mitigated: auto-generated from Operations CRDs).
- WatchOperation trigger conditions must be carefully tuned to avoid remediation storms.

## Compliance Mapping

SOC2 CC7.1 (automated maintenance). ISO A.8.8 (management of technical vulnerabilities — automated patching). NIS2 Art.21 (proactive security maintenance).
