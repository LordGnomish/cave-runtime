# ADR-115: CI Secret Injection via OIDC Token Exchange

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** CI/CD

**Related ADRs:** 079

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CI runners need credentials to push images, access registries, and deploy to clusters. Static secrets in runners create persistent attack surface.

## Candidates

## | Approach | OIDC token exchange | Static secrets | Vault Agent sidecar | Mounted secret files |
|---|---|---|---|---|
| Credential lifetime | ✅ Job duration only (~minutes) | ❌ Persistent | ⚠️ Lease-based but sidecar persists | ❌ Persistent |
| Runner compromise impact | ✅ Minimal (token expires with job) | ❌ All secrets exposed | ⚠️ | ❌ |
| RBAC | ✅ Runner ServiceAccount minimal | ❌ Broad | ⚠️ | ❌ |

## Decision

## ARC runners use OIDC token exchange to obtain short-lived credentials from OpenBao (Hetzner) / Key Vault (Azure) via ESO. Runner ServiceAccount has minimal RBAC. Credentials exist only for CI job duration — destroyed with runner pod.

## Rejected

## - **Static secrets in CI:** If runner is compromised, all static secrets are exposed. Secrets persist in runner environment between jobs.
- **Vault Agent sidecar:** Per-runner sidecar overhead. Adds ~50MB RAM per runner. Lease-based but sidecar process persists.
- **Mounted secret files:** Persists on runner filesystem between jobs. If runner is shared (not ephemeral), previous job's secrets visible.

## Consequences

## **Positive:**
- No persistent credentials in CI environment.
- Runner compromise has minimal blast radius (token expires with job).
- Minimal RBAC — runner can only do what the job needs.

**Negative:**
- OIDC token exchange adds ~2s to job startup (token acquisition).
- OpenBao/Key Vault must be available during CI jobs (dependency).
- OIDC configuration per CI system (GitHub Actions, Gitea) must be maintained.

## Compliance Mapping

## SOC2 CC6.7 (credential lifecycle in CI). ISO A.8.24 (cryptographic controls in CI). NIS2 Art.21 (supply chain security — CI infrastructure).
