# ADR-077: Sigstore Policy Controller for Image Admission

**Status:** Accepted

**Category:** Security

**Related ADRs:** 032, 101, 107

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE requires that only images built by the platform's CI pipeline run in any environment. Images must have cryptographic proof of provenance.

## Candidates

## | Criteria | Sigstore Policy Controller | Notary v2 | Connaisseur | Manual verification |
|---|---|---|---|---|
| cosign integration | ✅ Native (same project) | ❌ Different signing model | ⚠️ Supports cosign | N/A |
| Keyless (OIDC) | ✅ CI OIDC issuer verification | ❌ Key-based only | ⚠️ | N/A |
| SLSA provenance | ✅ Predicate verification | ❌ | ❌ | ❌ |
| K8s admission webhook | ✅ ClusterImagePolicy CRD | ⚠️ Separate admission | ✅ | ❌ |
| Community | CNCF (sigstore project) | CNCF (notary) | Small | N/A |

## Decision

## **Sigstore Policy Controller** for image signature + SLSA provenance verification at K8s admission. cosign keyless signing via CI OIDC (GitHub/Gitea). ClusterImagePolicy CRDs define trust policies.

## Rejected

## - **Notary v2:** Different signing model (not cosign-compatible). No keyless OIDC flow. No SLSA provenance predicate verification. Would require separate tooling for CI signing + admission verification.
- **Connaisseur:** Smaller community. Less native integration with cosign keyless flow. No SLSA predicate support.
- **No admission verification:** Images could be tampered between Harbor push and K8s deployment. Unacceptable supply chain risk.

## Consequences

## **Positive:**
- Only CI-built images with cryptographic provenance run in any environment.
- Keyless OIDC eliminates static signing key management.
- Supply chain attack surface eliminated at admission layer.

**Negative:**
- Fail-closed: if Policy Controller is down, no new pods can start (safe default but blocks deployments).
- ClusterImagePolicy must be maintained as trust policies evolve.
- Pull-through cached images from upstream (Harbor) must have their upstream signatures verified.

## Compliance Mapping

## SOC2 CC8.1 (integrity of deployed artifacts). ISO A.8.24 (cryptographic controls). SLSA Level 3 (hermetic build + signed provenance). NIS2 Art.21 (supply chain security).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-032**

cosign Keyless Signing

**Decision:** cosign keyless signing via Sigstore/Fulcio. Rejection: Notary (DCT legacy, v2 stagnation, no keyless workflow).

**ADR-107**

Provenance Verification Per CI Issuer

**Decision:** Provenance verification rules per CI issuer. GitHub profiles: keyless cosign via GitHub OIDC + Fulcio. Gitea profiles: Gitea OIDC issuer + Fulcio. Sigstore Policy Controller validates issuer identity + SLSA Provenance v1 predicate per profile.
