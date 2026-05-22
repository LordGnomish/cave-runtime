# ADR-026: ArgoCD for GitOps

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD / Deployment

**Related ADRs:** 063, 120, 036, 101

## Context

CAVE needs a GitOps engine to continuously reconcile desired state (Git) with actual state (cluster) for all 73 platform components and unlimited tenant workloads across 7 profiles (3 sovereign + 4 hyperscaler). GitOps provides:

- **Source of truth:** Git repository is the single source of truth for infrastructure and application state
- **Continuous reconciliation:** Continuous loop detects drift (manual changes, failed updates) and auto-corrects
- **Multi-profile support:** Same ArgoCD instance manages dev, staging, prod across sovereign cloud and hyperscaler with tenant isolation
- **Progressive delivery:** Integration with Argo Rollouts (ADR-036) for canary deployments
- **Audit trail:** Every state change traceable to Git commit + author

## Candidates

| Criteria | ArgoCD | Flux | Jenkins X | Rancher Fleet |
|---|---|---|---|---|
| Web UI | ✅ Rich UI (sync status, diff, logs) | ❌ No native UI (Weave GitOps separate) | Limited | Limited |
| Multi-tenancy | ✅ AppProjects + RBAC + ApplicationSets | ⚠️ Kustomization-level, less granular | ❌ | ⚠️ Cluster-level |
| OCI source | ✅ Native (Harbor registry) | ✅ Native | ❌ | ❌ |
| Server-side apply | ✅ Default (v2.5+) | ⚠️ Supported but not default | N/A | N/A |
| ApplicationSets (multi-profile template) | ✅ Full support (per profile/tenant) | ❌ No equivalent | N/A | N/A |
| Backstage integration | ✅ Mature ArgoCD plugin | ⚠️ Flux plugin less mature | ❌ | ❌ |
| Argo Rollouts (canary) | ✅ Native, same ecosystem | ⚠️ Flagger bridge | ❌ | ❌ |
| Helm support | ✅ Native Helm charts | ✅ HelmRelease CRD | ✅ | ✅ |
| License | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |
| Community | Very large (CNCF Graduated, 18K+ stars) | Large (CNCF Graduated, 6K+ stars) | Declining | Rancher-coupled |
| Per-cluster admin overhead | Medium (AppProject RBAC) | Medium (multi-tenancy less native) | Low but declining support | Low but vendor lock-in |

## Decision

**ArgoCD** (self-hosted on all 7 profiles via Helm chart). Configuration:
- **Repositories:** Git repo (cave-gitops-config crate) + Harbor OCI registry (ADR-120) for Helm charts
- **Multi-tenancy:** AppProjects per tenant + RBAC roles (viewer, editor, admin per AppProject)
- **ApplicationSets:** Template applications per profile (Hetzner dev/staging/prod, Azure dev/staging/prod, edge)
- **Server-side apply:** Default (v2.5+). Drift detection on platform components continuous.
- **Argo Rollouts:** ArgoCD + Rollouts for progressive delivery (canary, analysis gates, ADR-036)
- **Backstage integration:** ArgoCD UI embedded in Backstage portal (ADR-025) via plugin

## Implementation Reference

**Implementation Status:** Production

- **cave-gitops-config** crate: ArgoCD Application/ApplicationSet manifests. CAVE components + tenant workloads organized by profile.
- **Repository:** cave-gitops-config Git repo is source of truth. ArgoCD webhook auto-syncs on commit.
- **Profiles:** 7 environments (sovereign-dev, sovereign-staging, sovereign-prod, azure-dev, azure-staging, azure-prod, edge). ApplicationSets template per profile.
- **Helm integration:** Helm charts from Harbor (ADR-028). Chart versions pinned by digest (ADR-108).

**Version State (April 2026):** ArgoCD v3.2.9 (current stable) is the first major v3 release (upgraded from v2.14). v3 delivers improved ApplicationSet webhook support, UI performance improvements for large application sets, and refined server-side apply conflict resolution.

## Rejected Options

### Flux — Not Primary

**Reasons:**
1. **No native UI:** Debugging sync failures requires kubectl + CLI. In a 73-component platform with multi-tenant workloads, visual sync status is critical for operations. ArgoCD's rich UI (diff viewer, event logs, sync status per component) reduces MTTR.
2. **Multi-tenancy:** Flux's Kustomization model is per-resource. Managing 7 profiles × N tenants requires N×7 individual Kustomization resources. ArgoCD's ApplicationSets use templating to generate 7 Applications from single template.
3. **Backstage integration:** ArgoCD has mature Backstage plugin (100K+ downloads). Flux's Backstage plugin less mature.
4. **Argo Rollouts ecosystem:** ArgoCD and Rollouts share Argo ecosystem. Flux requires Flagger bridge for canary deployments — additional operational burden.

### Jenkins X — Not Recommended

**Reasons:**
1. **Community decline:** Jenkins X project declining. Less activity, fewer contributors.
2. **Opinionated pipeline:** Jenkins X bundles CI + CD + GitOps. CAVE uses GitHub Actions for CI (ADR-040), ArgoCD for CD — separation of concerns. Jenkins X forces tight coupling.
3. **Limited multi-cloud:** Jenkins X less battle-tested across diverse cloud providers (Hetzner, Azure, edge).

### Rancher Fleet — Not Recommended

**Reasons:**
1. **Vendor lock-in:** Tightly coupled to Rancher ecosystem. CAVE doesn't run Rancher as management layer. Fleet adds unnecessary dependency.
2. **Multi-cloud friction:** Fleet designed for Rancher-managed clusters. CAVE's heterogeneous clusters (Talos on the sovereign profile, AKS on Azure) don't naturally fit Rancher model.

## Consequences

### Positive

- **Rich UI for operations:** Sync status visualization, diff viewer, event logs. Reduces debugging time for drift issues.
- **Scalable multi-tenancy:** ApplicationSets template-based generation supports 1 template → 7 profiles × N tenants without N×7 manual manifests.
- **Native OCI source:** Harbor support (ADR-120) enables storing Helm charts as OCI artifacts. Fewer package formats to manage.
- **Same ecosystem:** Argo Rollouts (canary), Argo Workflows (orchestration) extend ArgoCD. Unified operational model.
- **Backstage integration:** ArgoCD plugin embedded in developer portal (ADR-025). Developers see deployment status in portal.
- **CNCF Graduated:** Large community, continuous improvements, security audits.

### Negative

- **Resource intensive:** ArgoCD server + repo-server + controller: ~500MB-1GB RAM total. On the sovereign profile dev (16GB), this is ~6% of node RAM.
- **ApplicationSet complexity:** As tenant count grows (1 → 10 → 100 tenants), ApplicationSet templates become complex. Risk of accidental cross-tenant deployments (mitigated by AppProject RBAC).
- **Server-side apply behavior changes:** Migrating from client-side to server-side apply changes conflict resolution semantics. Requires validation with Crossplane (ADR-067).

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| ApplicationSet misconfiguration deploys to wrong tenant | Low | High | AppProject RBAC enforces boundaries. Code review on ApplicationSet changes. Staging validates. |
| ArgoCD resource exhaustion causes missed syncs | Low | High | Monitor ArgoCD server/repo-server resource usage. HPA scaling for repo-server (multi-repo). |
| Server-side apply conflict with Crossplane drift detection | Low | High | Validation: deploy to staging first. Crossplane ManagedResourceActivationPolicy (ADR-124). |
| Git webhook failure causes stale state | Low | Medium | ArgoCD syncs periodically (default 3min). Manual sync button in UI. Monitoring alert on stale status. |

## License

**ArgoCD:** Apache 2.0 License (https://github.com/argoproj/argo-cd/blob/master/LICENSE)

## Compliance Mapping

**SOC2 CC8.1:** Change management — all deployment changes via Git. ArgoCD reconciliation provides audit trail.
**SOC2 CC8.2:** System monitoring — ArgoCD sync status monitoring. Drift detection alerts.
**ISO/IEC 27001 A.5.30:** Access control — AppProject RBAC enforces tenant isolation on deployment permissions.
**NIS2 Directive Article 21:** Configuration management — Infrastructure-as-code via Git enables version control and rollback.
