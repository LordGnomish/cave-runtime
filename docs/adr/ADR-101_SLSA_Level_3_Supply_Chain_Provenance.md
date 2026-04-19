# ADR-101: SLSA Level 3 Supply Chain Provenance

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 005, 077

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Every container image deployed to CAVE must have verifiable proof of how it was built, from what source code, by which CI system.

## Candidates

## | SLSA Level | Requirements | CAVE Implementation |
|---|---|---|
| Level 1 | Build process documented | ✅ 27-stage pipeline |
| Level 2 | Hosted build service, signed provenance | ✅ ARC runners + cosign |
| Level 3 | Hermetic build, hardened build platform, non-falsifiable provenance | ✅ Buildah --no-network + OIDC keyless |

## Decision

## Hermetic builds via Buildah `--no-network` (ADR-005). All dependencies from Pulp proxy. Signed provenance via cosign keyless OIDC + SLSA Provenance v1 predicate. SBOM (CycloneDX) generated at build. Provenance uploaded to Harbor OCI + Sovereign Ledger. Sigstore Policy Controller (ADR-077) validates at admission.

## Rejected

## - **SLSA Level 1-2 only:** Insufficient for enterprise compliance and regulated tenants. Level 2 allows non-hermetic builds where network access during build could inject malicious dependencies.
- **No SLSA:** Supply chain completely unverified. No proof that deployed image matches source code.
- **Manual attestation:** Not scalable across 27-stage pipeline. Human error in attestation creation.

## Consequences

## **Positive:**
- Complete supply chain provenance from source code to running container.
- Hermetic builds eliminate network-based dependency injection attacks.
- Keyless OIDC signing eliminates static key management.
- SBOM provides vulnerability tracking at package level.

**Negative:**
- Hermetic builds require all dependencies pre-cached in Pulp proxy.
- Build isolation (--no-network) can break builds that expect internet access — requires developer education.
- SLSA Level 3 adds ~30s to build time (signing + provenance generation).

## Compliance Mapping

## SOC2 CC8.1 (change integrity). ISO A.8.24 (cryptographic controls). SLSA Framework Level 3. NIS2 Art.21 (supply chain security).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-033**

CycloneDX as SBOM Format

**Decision:** CycloneDX as SBOM format. Rejection: SPDX (less tooling for vulnerability correlation, slower native Dependency-Track integration).
