# ADR-040: Self-Hosted CI Runners — Actions Runner Controller (ARC)

**Status:** Accepted

**Scope:** Azure

**Category:** CI/CD

**Related ADRs:** 010, 115

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's 27-stage CI pipeline runs on GitHub Actions. GitHub-hosted runners are shared, ephemeral, and rate-limited. For a platform with sovereign requirements, CI workloads must run on self-hosted infrastructure within the cluster.

## Candidates

## | Criteria | ARC (K8s) | GitHub-hosted | GitLab Runner | Jenkins Agent |
|---|---|---|---|---|
| K8s native | ✅ CRD-based (RunnerDeployment, RunnerSet) | ❌ GitHub-managed VMs | ✅ K8s executor | ✅ K8s plugin |
| Ephemeral | ✅ Pod-per-job, destroyed after | ⚠️ Reused within timeout | ✅ | ⚠️ |
| OIDC token exchange | ✅ GitHub OIDC → OpenBao/Key Vault (ADR-115) | ✅ | ✅ | ⚠️ Plugin |
| Autoscaling | ✅ Scale-to-zero, scale on demand | N/A | ✅ | ⚠️ |
| Network isolation | ✅ CiliumNetworkPolicy per runner namespace | ❌ Shared GitHub infra | ✅ | ✅ |
| License | Apache 2.0 | GitHub terms | MIT | MIT |

## Decision

## **Actions Runner Controller (ARC)** on K8s for all CI workloads. Pod-per-job (ephemeral — destroyed after each pipeline run). Scale-to-zero when idle. OIDC token exchange for secret-free CI (ADR-115). CiliumNetworkPolicy restricts runner network access to: Harbor (push), OpenBao/Key Vault (OIDC), ArgoCD (deploy), cluster API (tests). No internet access during build (hermetic — ADR-005).

## Rejected

## - **GitHub-hosted runners:** Shared infrastructure. Code and artifacts transit GitHub servers — sovereign profile incompatible. Rate limits on concurrent jobs. No network isolation.
- **GitLab Runner:** Would require GitLab installation. CAVE uses GitHub Actions (ADR-010).
- **Jenkins Agent:** Legacy. Jenkins not chosen as CI engine (ADR-010).

## Consequences

## **Positive:**
- CI runs on CAVE's own infrastructure — full sovereignty.
- Ephemeral pods — no credential persistence, no stale state between jobs.
- Scale-to-zero saves resources when no CI running.
- Network isolation prevents supply chain attacks via CI.

**Negative:**
- ARC management overhead (runner image updates, scaling config, node affinity).
- Runner pods compete with tenant workloads for node resources (mitigated: dedicated runner node pool or preemption priority).
- GitHub Actions workflow files still hosted on GitHub — Gitea self-hosted mirror is Phase 4 fallback.

## Compliance Mapping

## SOC2 CC6.1 (CI infrastructure access controls). SOC2 CC8.1 (build process integrity). ISO A.8.25 (secure development — controlled build environment). NIS2 Art.21 (supply chain — CI infrastructure sovereignty).
