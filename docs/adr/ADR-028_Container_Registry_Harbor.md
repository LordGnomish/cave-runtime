# ADR-028: Container Registry — Harbor

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD / Infrastructure

**Related ADRs:** 005, 026, 077, 108, 120

## Context

CAVE needs a container registry for storing:
- Built container images (from Buildah, stage 13, ADR-005)
- Helm charts as OCI artifacts (ArgoCD OCI source, ADR-120, ADR-026)
- Crossplane provider packages (Day 1+ provisioning, ADR-067)
- SBOM artifacts (CycloneDX, from stage 9, ADR-010)
- Signed attestations (cosign + Sigstore, ADR-077)

Requirements:
- **Self-hosted:** Sovereign profiles can't use external SaaS registries
- **Multi-tenancy:** Project-per-tenant isolation + RBAC (viewer/editor/admin per tenant)
- **Signing:** cosign signature storage + verification (Sigstore Policy Controller, ADR-077)
- **Pull-through cache:** Reduce upstream registry dependency + network latency
- **Vulnerability scanning:** Built-in Trivy integration (defense-in-depth with CI scanning)

## Candidates

| Criteria | Harbor | Docker Hub | GHCR | Zot | Distribution |
|---|---|---|---|---|---|
| Self-hosted | ✅ Helm/K8s | ❌ SaaS | ❌ SaaS | ✅ Go binary | ✅ Go binary |
| OCI artifacts | ✅ Full support | ⚠️ Images only | ✅ | ✅ | ✅ |
| Image signing | ✅ cosign + Notation | ❌ | ⚠️ | ✅ cosign | ❌ |
| Pull-through cache | ✅ Multi-upstream | N/A | N/A | ✅ | ❌ |
| Multi-tenancy/RBAC | ✅ Project + robot accounts | ❌ | ❌ | ❌ | ❌ |
| Vuln scanning | ✅ Trivy built-in | ❌ | ❌ | ❌ | ❌ |
| Garbage collection | ✅ Scheduled | N/A | N/A | ✅ | ✅ |
| Replication | ✅ Cross-registry | N/A | N/A | ✅ | ❌ |
| Helm integration | ✅ Native OCI | ⚠️ Legacy chart repo | ✅ | ✅ | ✅ |
| License | Apache 2.0 | Proprietary | Proprietary | Apache 2.0 | Apache 2.0 |
| Community | CNCF Graduated, 13K+ stars | N/A | N/A | Growing, 1K+ stars | Foundation, 6K+ stars |

## Decision

**Harbor** (self-hosted via Helm on all profiles). Configuration:
- **Projects:** One Harbor project per tenant (isolation boundary)
- **Robot accounts:** Per-tenant CI push/pull credentials. Signed with tenant identity (SPIFFE/OIDC)
- **Pull-through cache:** Upstream registries (Docker Hub, ghcr.io, quay.io, registry.k8s.io). Automatic cache warming.
- **Trivy scanning:** Built-in on push. Quarantine policy on critical CVEs.
- **OCI artifacts:** Helm charts (from stage 13, ADR-010), Crossplane providers, manifests stored as OCI
- **Signing:** cosign signatures stored alongside images. Policy Controller (ADR-077) enforces signature verification on pull

## Implementation Reference

**Implementation Status:** Production

- **cave-registry** crate: Harbor Helm deployment, robot account management, pull-through cache configuration
- **Storage backend:** PVC on shared storage (Hetzner: Ceph via managed volumes. Azure: Managed Disk)
- **Database:** PostgreSQL via CNPG Cluster (ADR-105: encrypted at rest via OpenBao Transit)
- **Cache warming:** Scheduled job pulls popular base images (ubuntu:22.04, alpine:3.18, golang:1.21, python:3.11, node:18)

## Rejected Options

### Docker Hub / GHCR — Not Acceptable

**Reasons:**
1. **SaaS-only:** External image storage contradicts CAVE's sovereign deployment profile requirement. Regulated customers cannot store container images on external infrastructure.
2. **No multi-tenancy:** Docker Hub has user-level accounts, no project-per-tenant isolation. All tenants see all images.
3. **No pull-through cache:** Adds latency + external dependency on Docker Hub/GitHub availability. Pull-through cache is critical for reliable deployments.

### Zot — Not Recommended as Primary

**Reasons:**
1. **Lightweight but feature-limited:** Zot is excellent for simple registries but lacks:
   - Multi-tenant RBAC (project-per-tenant + robot accounts)
   - Pull-through cache from multiple upstreams (would need separate proxy)
   - Built-in vulnerability scanning (would need Trivy sidecar)
2. **Operational complexity:** Features CAVE needs would require building additional components. Harbor provides them integrated.

### Distribution (CNCF) — Not Recommended

**Reasons:**
1. **Bare-bones registry:** Official CNCF Docker registry spec. No RBAC, no scanning, no pull-through, no replication.
2. **Custom development required:** To match Harbor's feature set, CAVE would need to build:
   - Multi-tenant project isolation
   - Robot account management
   - Pull-through cache orchestration
   - Vulnerability scanning integration
3. **Maintenance burden:** Custom components require ongoing maintenance vs. Harbor's active community.

## Consequences

### Positive

- **Full-featured:** Multi-tenant projects, RBAC, pull-through cache, Trivy scanning, OCI artifacts all integrated
- **Pulled image caching:** Popular base images (ubuntu, alpine, golang, python) cached locally. Reduces Docker Hub dependency + network latency.
- **Scalable multi-tenancy:** Project-per-tenant + robot accounts enable fine-grained CICD secrets (tenant A can only push/pull tenant A images).
- **OCI artifacts:** Helm charts, Crossplane providers, SBOMs stored as OCI artifacts. Unified artifact format (no legacy chart repos).
- **Signing integration:** cosign signatures stored in Harbor alongside images. Sigstore Policy Controller (ADR-077) enforces signature verification.
- **Enterprise-proven:** CNCF Graduated. Widely deployed (VMware, Cloud Native Computing Foundation, large enterprises).

### Negative

- **Resource overhead:** ~2-4GB RAM for core + database + Redis. On Hetzner CX62 (16GB), this is 12-25% of node RAM.
- **PostgreSQL dependency:** Harbor requires PostgreSQL backend. Another database to backup/restore. Mitigated: CNPG provides HA (ADR-105).
- **Garbage collection complexity:** Scheduled GC must not interfere with pulls. Risk of stale images if GC too aggressive.
- **Upgrade complexity:** Harbor major version upgrades can require DB migrations. Testing required on staging before prod rollout.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Pull-through cache fills storage | Low | High | Quota policy on cache. Manual cleanup of stale cached layers. Monitoring cache storage %. |
| PostgreSQL backend failure | Low | High | CNPG Cluster with HA (3 replicas). Automated failover. Backups in S3. |
| Harbor upgrade breaks project RBAC | Low | High | Staging validates. RBAC tested before/after upgrade. Runbook for rollback. |
| Trivy DB stale (cached from old update) | Low | Medium | Harbor updates Trivy DB daily. Scheduled scan of cached layers weekly. |

## License

**Harbor:** Apache 2.0 License (https://github.com/goharbor/harbor/blob/main/LICENSE)

## Compliance Mapping

**SOC2 CC6.1:** Registry access controls — project-level RBAC enforces tenant isolation.
**SOC2 CC8.1:** Artifact integrity — cosign signatures required for all images. Policy Controller enforces.
**ISO/IEC 27001 A.8.24:** Cryptographic controls — image signatures + artifact encryption at rest.
**ISO/IEC 27001 A.8.9:** Configuration management — Harbor as single source of truth for deployable artifacts.
**NIS2 Directive Article 21:** Supply chain security — artifact integrity + signature verification prevents tampering.
