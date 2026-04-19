# ADR-077: Sigstore Policy Controller for Image Admission

**Status:** Accepted

**Scope:** Universal

**Category:** Security / Admission Control

**Related ADRs:** 032, 101, 107, 108

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

## Implementation Reference

**Implementation Status:** Production

- **cave-sign** crate: cosign integration, keyless signing via Sigstore/Fulcio
- **Policy Controller:** Deployed as K8s webhook. ClusterImagePolicy CRDs define trust rules per profile (Hetzner GitHub OIDC issuer, Azure Gitea OIDC issuer).
- **Verification:** SLSA Provenance v1 predicate checked on all images. Build repo, build trigger, build timestamp verified.

## Consequences

### Positive

- **Supply chain integrity:** Only cryptographically-signed CI images run in production. Tampered images rejected at admission.
- **Keyless workflow:** No static signing keys to rotate/secure. OIDC token from CI issuer (GitHub/Gitea) proves identity.
- **SLSA compliance:** Provenance verification proves build hermiticity (ADR-101) at deployment time.
- **Fail-closed:** Policy Controller down → all pod creation blocked (safer than allowing unsigned images).

### Negative

- **Deployment blocks if Policy Controller down:** Mitigation required: Policy Controller must be highly available (deployed on multiple nodes, PDB).
- **ClusterImagePolicy maintenance:** Trust policies must evolve as new CI issuers added or compromised. Runbook for policy updates required.
- **Upstream image verification:** Pull-through cached images from Docker Hub/quay.io lack CI signatures. Trust policy must allow upstream publishers or use digest pinning (ADR-108).

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Policy Controller pod crash blocks deployments | Low | High | Deploy on 3+ nodes with Pod Disruption Budget. Monitoring alert if <2 replicas healthy. |
| Signature verification false positive | Low | Medium | Staging environment tests policy before prod rollout. Human review of policy changes. |
| Upstream image (Alpine 3.18) lacks signature | Medium | Low | Digest pinning (ADR-108) for upstream images. If signature unavailable, use digest. |

## License

**Sigstore Policy Controller:** Apache 2.0 (https://github.com/sigstore/policy-controller/blob/main/LICENSE)

## Compliance Mapping

**SOC2 CC8.1:** Integrity of deployed artifacts — image signatures + provenance verification.
**ISO/IEC 27001 A.8.24:** Cryptographic controls — digital signatures on container images.
**SLSA Level 3:** Provenance verification matches hermetic build requirement.
**NIS2 Directive Article 21:** Supply chain security — artifact integrity verified before deployment.

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-032**

cosign Keyless Signing

**Decision:** cosign keyless signing via Sigstore/Fulcio. Rejection: Notary (DCT legacy, v2 stagnation, no keyless workflow).

**ADR-107**

Provenance Verification Per CI Issuer

**Decision:** Provenance verification rules per CI issuer. GitHub profiles: keyless cosign via GitHub OIDC + Fulcio. Gitea profiles: Gitea OIDC issuer + Fulcio. Sigstore Policy Controller validates issuer identity + SLSA Provenance v1 predicate per profile.
