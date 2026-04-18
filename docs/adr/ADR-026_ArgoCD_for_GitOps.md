# ADR-026: ArgoCD for GitOps

**Status:** Accepted

**Category:** CI/CD

**Related ADRs:** 063, 120

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs a GitOps engine to continuously reconcile desired state (Git) with actual state (cluster) for all platform components and tenant workloads across 7 profiles.

## Candidates

## | Criteria | ArgoCD | Flux | Jenkins X | Rancher Fleet |
|---|---|---|---|---|
| Web UI | ✅ Rich UI (sync status, diff, logs, events) | ❌ No native UI (Weave GitOps is separate product) | Limited | Limited |
| Multi-tenancy | ✅ AppProjects with RBAC, ApplicationSets | ⚠️ Kustomization-level, less granular | ❌ | ⚠️ |
| OCI source | ✅ Native (Harbor OCI registry — ADR-120) | ✅ Native | ❌ | ❌ |
| Server-side apply | ✅ Default since v2.5+ | ⚠️ Supported but not default | N/A | N/A |
| ApplicationSets | ✅ Generate apps from templates (per profile, per tenant) | ❌ No equivalent (Kustomization per app) | N/A | N/A |
| Backstage integration | ✅ Backstage ArgoCD plugin (mature) | ⚠️ Backstage Flux plugin (less mature) | ❌ | ❌ |
| Argo Rollouts integration | ✅ Native (same ecosystem) | ⚠️ Flagger integration (different ecosystem) | ❌ | ❌ |
| Helm support | ✅ Native | ✅ Native (HelmRelease) | ✅ | ✅ |
| License | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |
| Community | Very large (CNCF Graduated, 18K+ stars) | Large (CNCF Graduated, 6K+ stars) | Declining | Rancher-coupled |

## Decision

## **ArgoCD** (self-hosted via Helm) for GitOps on all profiles. Server-side apply default. OCI source support via Harbor (ADR-120). ApplicationSets for per-profile + per-tenant management.

## Rejected

## - **Flux:** No native UI — debugging sync issues requires CLI + kubectl. In a 73-component platform with multi-tenant workloads, visual sync status is critical for operations and Backstage integration. ApplicationSets pattern is more powerful than Flux's per-resource Kustomization model for managing 7 profiles × N tenants.
- **Jenkins X:** Declining community. Opinionated CI/CD, not pure GitOps.
- **Rancher Fleet:** Tightly coupled to Rancher ecosystem. CAVE doesn't use Rancher.

## Consequences

## (+) Rich UI for debugging. ApplicationSets for scalable multi-profile + multi-tenant management. Native OCI source (Harbor). Same ecosystem as Argo Rollouts + Argo Workflows. Backstage plugin mature.
(-) ArgoCD is resource-intensive (~500MB-1GB RAM). ApplicationSet complexity grows with tenant count. Server-side apply behavior changes require Crossplane compatibility validation.

## Compliance Mapping

## SOC2 CC8.1 (change management — all changes via Git, ArgoCD reconciles).
