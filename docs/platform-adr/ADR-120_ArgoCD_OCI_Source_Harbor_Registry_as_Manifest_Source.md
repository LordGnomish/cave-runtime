# ADR-120: ArgoCD OCI Source — Harbor Registry as Manifest Source

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 026, 028

## Context

Git repositories allow force-push (mutable refs). For deployment artifacts, immutability is required to prevent tampering between build and deploy.

## Candidates

| Source | Immutable | Signed | Versioned | Provenance |
|---|---|---|---|---|
| Git (tag) | ❌ Tags can be moved | ⚠️ GPG commit signing | ✅ | ⚠️ |
| Git (commit SHA) | ✅ | ⚠️ | ⚠️ Not human-readable | ⚠️ |
| OCI (Harbor) | ✅ Content-addressed digest | ✅ cosign-signed | ✅ OCI tags + digests | ✅ SLSA |

## Decision

Harbor OCI registry as manifest source alongside Git. Deployment artifacts packaged as OCI images: hermetic, versioned, signed (cosign), immutable. ArgoCD ApplicationSet sources include both Git (for config) and OCI (for artifacts). Source Hydrator produces reproducible hydrated manifests.

## Rejected

- **Git-only source:** Git refs (tags, branches) are mutable — force-push can change what a tag points to. OCI digests are content-addressed (SHA256) — immutable by definition.
- **Flux OCI:** ArgoCD chosen over Flux (ADR-026). OCI source is an ArgoCD capability.

## Consequences

**Positive:**
- Immutable deployment artifacts. Content-addressed digests prevent tampering.
- cosign-signed artifacts verify CI provenance at deployment.
- Source Hydrator provides reproducible manifests with commit association.

**Negative:**
- Two sources to manage (Git + OCI). ArgoCD must reconcile both.
- OCI artifact packaging adds build step (~10s per artifact).
- Harbor storage requirements increase with artifact retention.

## Compliance Mapping

SOC2 CC8.1 (deployment artifact integrity). SLSA Level 3 (immutable deployment artifacts). NIS2 Art.21 (supply chain — deployment integrity).
