# ADR-053: External Secrets Operator (ESO)

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** Identity & Secrets

**Related ADRs:** 020, 079, 083

## Context

Kubernetes workloads need secrets from OpenBao (Hetzner) and Azure Key Vault (Azure) delivered as native K8s Secrets. A standard operator must work across both providers without requiring application changes.

## Candidates

| Criteria | ESO | Sealed Secrets | Vault Agent Injector | Secrets Store CSI Driver |
|---|---|---|---|---|
| Multi-backend | ✅ OpenBao + Key Vault + AWS + GCP | ❌ Kubeseal only | ❌ Vault only | ✅ Multiple |
| Delivery method | K8s Secret (native) | K8s Secret | File injection via sidecar | Volume mount |
| Dynamic secrets | ✅ Lease-based sync | ❌ Static encrypted blobs | ✅ | ⚠️ Rotation complex |
| Sidecar required | ❌ Operator-only | ❌ | ✅ Per-pod sidecar | ❌ DaemonSet |
| Rotation support | ✅ ESO sync interval | ❌ Manual re-encrypt | ✅ Lease renewal | ⚠️ |
| GitOps compatible | ✅ SecretStore + ExternalSecret CRDs in Git (no secrets in Git) | ⚠️ Encrypted secrets in Git | ⚠️ Annotations on pods | ⚠️ SecretProviderClass |
| License | Apache 2.0 | Apache 2.0 | MPL 2.0 (Vault) | Apache 2.0 |

## Decision

**External Secrets Operator (ESO)** for all profiles. Single operator syncs secrets from OpenBao (Hetzner) and Key Vault (Azure) to native K8s Secrets. SecretStore and ExternalSecret CRDs managed in Git via ArgoCD — no secrets in Git, only references.

## Rejected

- **Sealed Secrets (Bitnami):** Requires encrypting secrets before committing to Git. No dynamic secret support — all secrets are static encrypted blobs. No integration with OpenBao/Key Vault. Secret rotation requires manual re-encryption and commit. Fundamentally wrong model for a platform with dynamic credential rotation (ADR-083).
- **Vault Agent Injector:** Adds a sidecar container to every pod consuming secrets. At scale (100+ pods across tenants), sidecar overhead is significant (~50MB RAM per sidecar × 100 pods = 5GB). Only works with Vault/OpenBao — no Key Vault support. Application must read files from sidecar volume, not standard K8s Secret env vars.
- **Secrets Store CSI Driver:** Mounts secrets as files on a volume, not as K8s Secrets. Some applications require env vars from K8s Secrets (standard K8s pattern). DaemonSet requirement adds node-level resource consumption. Rotation handling less mature than ESO.

## Consequences

**Positive:**
- Single operator for both providers (OpenBao + Key Vault) — no per-provider secret delivery mechanism.
- Secrets sync automatically on provider-side rotation (ESO detects changes within sync interval).
- Dynamic credentials supported (ESO refreshes K8s Secret when OpenBao lease renews).
- No sidecar overhead — operator runs cluster-wide, not per-pod.
- GitOps-native: SecretStore and ExternalSecret CRDs in Git, actual secret values never in Git.

**Negative:**
- Additional operator to manage (ESO deployment, CRDs, RBAC).
- Sync latency: secrets update within ESO refresh interval (default 1 minute, configurable). During rotation, brief window where old credential is in K8s Secret but new credential is in OpenBao/Key Vault.
- Cache TTL creates brief stale window if ESO pod restarts during rotation.
- ESO failure mode: if ESO pod is down, existing K8s Secrets remain but new secrets or rotations are not synced. P2 alert triggers.

## Compliance Mapping

SOC2 CC6.7 (credential lifecycle management). ISO A.8.24 (use of cryptography — secrets encrypted at rest in vault, decrypted only in ESO memory). GDPR Art.32 (security of processing — no credentials in Git or logs).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-073**

Crossplane Credentials via External Secret Store

**Decision:** Crossplane provider credentials stored in OpenBao (Hetzner) / Key Vault (Azure), retrieved via ESO. Never plain K8s Secrets for provider auth — prevents credential sprawl.
