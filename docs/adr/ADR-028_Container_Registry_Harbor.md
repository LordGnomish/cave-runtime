# ADR-028: Container Registry — Harbor

**Status:** Accepted

**Category:** CI/CD

**Related ADRs:** 005, 026, 077, 108, 120

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs a container registry for storing built images, Helm charts (OCI), Crossplane providers, and deployment artifacts. The registry must support image signing verification, pull-through caching, multi-tenant project isolation, and garbage collection.

## Candidates

## | Criteria | Harbor | Docker Hub | GitHub Container Registry | Zot | Distribution (CNCF) |
|---|---|---|---|---|---|
| Self-hosted | ✅ K8s Helm | ❌ SaaS | ❌ SaaS | ✅ | ✅ |
| OCI artifacts | ✅ Full OCI support (Helm, Crossplane, manifests) | ⚠️ Images mainly | ✅ | ✅ | ✅ |
| Image signing | ✅ cosign + Notation support | ⚠️ | ⚠️ | ✅ cosign | ❌ |
| Pull-through cache | ✅ Multiple upstream registries | N/A | N/A | ✅ | ❌ |
| Multi-tenant | ✅ Project-per-tenant, robot accounts | ❌ | ❌ | ❌ | ❌ |
| Vulnerability scanning | ✅ Trivy integration (built-in) | ❌ | ❌ | ❌ | ❌ |
| Garbage collection | ✅ Scheduled GC | N/A | N/A | ✅ | ✅ |
| Replication | ✅ Cross-registry replication | N/A | N/A | ✅ Sync | ❌ |
| RBAC | ✅ Project-level RBAC | ❌ | ⚠️ Org-level | ❌ | ❌ |
| License | Apache 2.0 | Proprietary | Proprietary | Apache 2.0 | Apache 2.0 |
| Community | Very large (CNCF Graduated, VMware-originated) | N/A | N/A | Growing | Foundation project |

## Decision

## **Harbor** (self-hosted via Helm) for all profiles. Project-per-tenant isolation. Pull-through cache for upstream registries (Docker Hub, ghcr.io, quay.io). Built-in Trivy scanning. OCI artifact support for Helm charts, Crossplane providers, and ArgoCD OCI sources (ADR-120). Robot accounts per tenant for CI push/pull.

## Rejected

## - **Docker Hub / GitHub Container Registry:** SaaS-only. No self-hosting. Image storage on external provider contradicts sovereign profile. No project-level multi-tenancy.
- **Zot:** Excellent lightweight OCI registry but no multi-tenant RBAC (project-per-tenant), no pull-through cache from multiple upstreams, no built-in vulnerability scanning. Would require additional components to match Harbor's feature set.
- **Distribution (CNCF):** Bare registry. No RBAC, no scanning, no pull-through cache, no replication. Would need significant custom development.

## Consequences

## **Positive:**
- Full-featured registry with multi-tenant isolation (project-per-tenant).
- Pull-through cache reduces upstream registry dependency and speeds up image pulls.
- Built-in Trivy scanning provides additional vulnerability check beyond CI pipeline.
- OCI support enables Harbor as artifact store for Helm, Crossplane, and ArgoCD OCI sources.
- cosign signature storage alongside images — Sigstore Policy Controller (ADR-077) verifies signatures from Harbor.
- CNCF Graduated — enterprise-proven, active community.

**Negative:**
- Harbor is resource-intensive (~2-4GB RAM for core + DB + Redis + storage).
- PostgreSQL backend required (deployed via CNPG — additional DB to manage).
- Garbage collection must be scheduled carefully to avoid disrupting pulls.
- Harbor upgrade path can be complex (DB migrations between major versions).

## Compliance Mapping

## SOC2 CC6.1 (registry access controls — project RBAC). SOC2 CC8.1 (artifact integrity — signed images). ISO A.8.24 (cryptographic controls — image signatures). ISO A.8.9 (configuration management — registry as single source of truth). NIS2 Art.21 (supply chain — controlled artifact distribution).
