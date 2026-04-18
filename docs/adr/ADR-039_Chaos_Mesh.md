# ADR-039: Chaos Mesh

**Status:** Accepted

**Category:** Resilience

**Related ADRs:** 100

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs continuous resilience testing to validate survivability design intent (≥99% tenant pod continuity during single-component failure).

## Candidates

## | Criteria | Chaos Mesh | LitmusChaos | Gremlin | Chaos Monkey |
|---|---|---|---|---|
| K8s native | ✅ CRD-based experiments | ✅ CRD-based | ❌ SaaS + agent | ❌ Legacy |
| Fault types | Pod, network, IO, time, kernel, DNS, HTTP, stress | Pod, network, IO, time, stress | Full (SaaS) | Pod kill only |
| Scheduling | ✅ Cron-based continuous experiments | ✅ CronExperiment | ✅ Scheduled attacks | ❌ |
| Dashboard | ✅ Chaos Dashboard (web UI) | ✅ ChaosCenter | ✅ Gremlin UI | ❌ |
| RBAC scoping | ✅ Namespace-scoped experiments (tenant-safe) | ⚠️ Less granular | ✅ | ❌ |
| License | Apache 2.0 | Apache 2.0 | Proprietary (SaaS) | Apache 2.0 |
| Community | Large (CNCF Incubating, PingCAP-originated) | Large (CNCF Incubating) | N/A (commercial) | Small/legacy |

## Decision

## **Chaos Mesh** for continuous resilience testing. Hourly experiments on prod, 4-hourly on staging. Results logged as `Resilience Proof` attestations in Sovereign Ledger.

## Rejected

## - **LitmusChaos:** Similar capability but Chaos Mesh's CRD model is more composable (workflow-based experiments with sequential/parallel steps). Chaos Mesh Dashboard UI is more mature.
- **Gremlin:** SaaS-only for full features. Proprietary. Agent-based. Contradicts self-hosted principle.
- **Chaos Monkey:** Netflix-era tool. Pod kill only. No network/IO/DNS fault injection. Too limited.

## Consequences

## (+) CRD-based, namespace-scoped (safe for multi-tenant). Continuous validation produces signed resilience proofs. Rich fault types cover all blast-radius catalog entries.
(-) Chaos experiments can cause real disruption if misconfigured. Namespace scoping must be enforced by OPA. Experiments must avoid tenant-dedicated resources without tenant consent.

## Compliance Mapping

## SOC2 CC7.1 (resilience testing), NIS2 Art.21 (business continuity testing).
