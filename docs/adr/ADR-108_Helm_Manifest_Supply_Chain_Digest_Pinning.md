# ADR-108: Helm/Manifest Supply Chain — Digest Pinning

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** ## Context

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Helm charts and container images referenced by mutable tags (`:latest`, `:stable`) create TOCTOU attacks. An attacker who compromises an upstream registry can replace a tagged image.

## Candidates

## | Approach | Digest pinning + cosign verification | Tag-based | Manual verification |
|---|---|---|---|
| Immutability | ✅ Digest is content-addressed (SHA256) | ❌ Tag can be overwritten | ❌ Manual |
| Automated | ✅ Renovate enforces digest-only | ❌ | ❌ |
| OPA enforcement | ✅ Rejects mutable tags | ❌ | ❌ |

## Decision

## OCI registries with digest pinning — no floating tags. Upstream Helm charts cosign-verified before pull. Renovate enforces digest-only updates (opens PRs with new digests). OPA rejects any resource referencing a mutable tag. Applies equally to Crossplane provider images and platform Helm charts.

## Rejected

## - **Tag-based references:** Mutable. Registry compromise → deployed image changes without Git commit. TOCTOU attack.
- **No upstream verification:** Upstream chart compromise undetected until deployment fails or security incident occurs.
- **Manual version tracking:** Unsustainable with 73 components. Renovate automates digest tracking.

## Consequences

## **Positive:**
- Every deployed artifact is content-addressed. Image replacement attacks impossible.
- Automated by Renovate — no manual version tracking.
- OPA admission rejects mutable references at deployment time.

**Negative:**
- Digest pinning makes Helm chart references verbose (SHA256 hashes instead of readable tags).
- Renovate PR volume increases (every upstream update generates a digest-change PR).
- If upstream registry changes digest without version bump, Renovate detects and surfaces the change.

## Compliance Mapping

## SOC2 CC8.1 (artifact integrity). SLSA Level 3 (immutable references). NIS2 Art.21 (supply chain security).
