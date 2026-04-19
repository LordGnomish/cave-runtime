# ADR-070: vcluster CI — Mandatory Prod, Opt-in Dev/Staging

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** CI/CD

**Related ADRs:** 012

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## PR validation needs isolated environments. Prod PRs must validate in cluster-isolated vclusters to catch multi-tenant issues. Dev/staging PRs can use cheaper namespace-only deployment.

## Candidates

## | Profile | vcluster mode | Rationale |
|---|---|---|
| prod | Mandatory (every PR) | Must validate cluster-level isolation before canary |
| staging | Opt-in (CI label) | Useful for integration testing, not always needed |
| dev | Opt-in (CI label) | Speed > isolation for development iteration |

## Decision

## PR vclusters mandatory on prod profile. Opt-in on dev/staging via `cave-ci: vcluster` label. Resource cap per vcluster: 2CPU/4Gi. TTL: 4h. Max 5 concurrent per tenant. Auto-destroyed after CI stage 27 (cleanup).

## Rejected

## - **Namespace-only CI everywhere:** Faster and cheaper but misses cluster-level issues (CRD conflicts, RBAC boundary violations, cross-namespace network policy). Unacceptable risk for prod-bound changes.
- **Full cluster per PR:** 10-20 min provision time, ~€10/cluster. At 20 PRs/day = 200 min wait + €200/day. vcluster provides cluster semantics in 30 seconds at namespace cost.
- **Mandatory vcluster everywhere:** Slows dev iteration unnecessarily. Dev profile doesn't need cluster isolation for every PR.

## Consequences

## **Positive:**
- Prod PRs validated in cluster-isolated environment before canary promotion.
- 30-second creation time vs 10-20 min for real clusters.
- Resource caps prevent CI from consuming tenant production resources.

**Negative:**
- Additional resource consumption (2CPU/4Gi per active PR vcluster).
- vcluster syncer adds small overhead.
- Max 5 concurrent = queue possible during high PR volume (mitigated by 4h TTL auto-cleanup).

## Compliance Mapping

## SOC2 CC8.1 (change validation before production). ISO A.14.2.9 (system acceptance testing).
