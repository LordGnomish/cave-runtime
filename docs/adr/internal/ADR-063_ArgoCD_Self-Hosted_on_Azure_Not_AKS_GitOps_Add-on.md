# ADR-063: ArgoCD Self-Hosted on Azure (Not AKS GitOps Add-on)

**Status:** Accepted

**Scope:** Azure

**Category:** CI/CD

**Related ADRs:** 026

## Context

AKS offers a built-in GitOps add-on based on Flux. CAVE must decide whether to use this managed offering or self-host ArgoCD on AKS, same as Hetzner.


## Candidates

| Criteria | Self-hosted ArgoCD | AKS GitOps Add-on (Flux) |
|---|---|---|
| UI | ✅ Full ArgoCD UI (sync status, diff, logs) | ❌ No UI (CLI only) |
| ApplicationSets | ✅ Template-based multi-tenant/multi-profile generation | ❌ Not available |
| OCI source | ✅ Harbor OCI (ADR-120) | ❌ Git-only |
| Argo Rollouts integration | ✅ Same ecosystem | ❌ Flagger (different) |
| Backstage plugin | ✅ Mature ArgoCD plugin | ❌ No Flux Backstage plugin |
| Provider parity | ✅ Same ArgoCD config as Hetzner | ❌ Different GitOps engine per provider |
| Upgrade management | ❌ Self-managed (Renovate + soak) | ✅ Azure-managed |
| Azure Monitor integration | ❌ Grafana instead | ✅ Native |


## Decision

**Self-hosted ArgoCD via Helm on AKS.** Identical ArgoCD installation, configuration, ApplicationSets, and Backstage integration as Hetzner profiles.


## Rejected Options

- **AKS GitOps add-on (Flux-based):** Would create two fundamentally different GitOps engines across providers. ArgoCD on Hetzner, Flux on Azure = doubled operational knowledge, doubled debugging procedures, different sync semantics, different multi-tenancy models. Breaks the "same UX across providers" principle (ADR-025). No ArgoCD UI — debugging sync issues requires CLI + kubectl. No ApplicationSets — managing N tenants × 7 profiles requires manual Kustomization files. No OCI source — cannot use Harbor OCI registry for immutable artifact deployment (ADR-120). No Argo Rollouts integration — would need Flagger for canary on Azure, Argo Rollouts on Hetzner.


## Consequences

**Positive:**
- Identical ArgoCD across both providers. Same config, same debugging tools, same Backstage plugin, same ApplicationSets.
- Zero operational knowledge delta between providers — one GitOps engine to master.
- OCI source, Rollouts, and all ArgoCD features available on both providers.

**Negative:**
- Must manage ArgoCD lifecycle on AKS (upgrade, HA, monitoring) instead of leveraging managed Flux.
- AKS GitOps add-on benefits (auto-upgrade, Azure Monitor integration, Azure Arc integration) not available.
- Compensated by: Renovate automates ArgoCD upgrades, Grafana provides equivalent monitoring, soak windows (ADR-132) ensure safe upgrades.

Compliance Mapping

SOC2 CC8.1 (consistent change management — same GitOps across all environments). ISO A.14.2 (secure development — same deployment pipeline).

