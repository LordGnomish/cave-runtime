# ADR-089: Signed OPA Policy Bundles

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Security

**Related ADRs:** 030, 131

## Context

OPA policy bundles are deployed to clusters and govern admission decisions. If bundles are tampered with, policy enforcement is compromised. Bundles must have cryptographic provenance.

## Candidates

| Approach | cosign-signed bundles | Self-signed | Unsigned | Per-policy signing |
|---|---|---|---|---|
| Trust chain | ✅ CI OIDC issuer chain | ⚠️ Internal CA only | ❌ None | ❌ Key management nightmare |
| Tamper detection | ✅ At deployment | ⚠️ If verified | ❌ No | ✅ But overhead |
| Git pinning | ✅ Bundle hash pinned to Git commit | ⚠️ Manual | ❌ | ⚠️ |

## Decision

OPA bundles cosign-signed and pinned to Git commits. ArgoCD detects drift. Core policies are Tier A/B constitutional artifacts (ADR-137). Lifecycle: Rego authoring → Git commit → CI validation (Conftest) → cosign signing → ArgoCD deployment → OPAL data distribution → drift detection.

## Rejected

- **Unsigned bundles:** Tamper risk. Modified policy could silently bypass admission controls.
- **Self-signed:** No external trust chain. Internal compromise undetectable.
- **Per-policy signing:** Operational overhead of signing individual .rego files. Bundle-level signing is sufficient.

## Consequences

**Positive:**
- Policy bundles cryptographically verified at deployment. Tampered bundles rejected.
- Git commit pinning provides complete audit trail for policy changes.
- Same cosign toolchain as image signing — unified signing infrastructure.

**Negative:**
- CI pipeline must include bundle signing step (additional build time, ~5s).
- cosign key/keyless configuration must be maintained.
- Bundle signature verification failure = no policy updates (safe default but blocks policy fixes during incident).

## Compliance Mapping

SOC2 CC6.1 (policy integrity). ISO A.5.1 (policy controls). NIS2 Art.21 (security policy management).
