# ADR-093: Sovereign Ledger (WORM + Sigstore)

**Status:** Accepted

**Category:** Governance

**Related ADRs:** 106

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs non-repudiable audit evidence that cannot be tampered with — even by platform administrators. Evidence must survive platform destruction.

## Candidates

## | Approach | WORM + Sigstore | Database audit log | Cloud-provider audit | Blockchain |
|---|---|---|---|---|
| Immutability | ✅ Object Lock / immutable blob | ❌ Mutable (admin can modify) | ⚠️ Provider-dependent | ✅ ||
| Cryptographic proof | ✅ Sigstore signing + Merkle tree | ❌ | ⚠️ | ✅ |
| Self-hosted | ✅ MinIO/ADLS | ✅ | ❌ Provider-owned | ❌ Public chain |
| Portability | ✅ WORM escrow cross-region | ❌ | ❌ | ⚠️ |
| Cost | ✅ Storage-only (MinIO) | ✅ Low | ❌ Per-event pricing | ❌ Gas fees |

## Decision

## Append-only WORM log (MinIO Object Lock / ADLS immutable blob) with Sigstore signing for non-repudiation. Merkle tree aggregation for CI attestations. Cross-region escrow replica independent of primary cluster. Quarterly integrity verification. 15 attestation types covering CI, resilience, AI, security, governance. Retention: constitutional indefinite, operational 3y, CI 2y. Expired: hash-of-hash consolidation.

## Rejected

## - **Database audit log:** Mutable. Platform admin with DB access can modify entries. Non-repudiation impossible.
- **Cloud-provider audit (Azure Activity Log, etc.):** Provider-owned. Not portable. Provider lock-in for audit data.
- **Blockchain:** Overkill for internal platform. Public chain = data exposure. Private chain = additional infrastructure. WORM + Sigstore provides same guarantees at lower complexity.

## Consequences

## **Positive:**
- Tamper-proof audit evidence. Even platform administrators cannot modify signed WORM entries.
- Cross-region escrow survives primary cluster destruction.
- Merkle tree aggregation reduces write frequency (one write per pipeline, not per stage).
- Sigstore provides cryptographic non-repudiation linked to CI OIDC identity.

**Negative:**
- WORM storage costs scale with attestation volume (mitigated: Merkle aggregation, retention policies).
- Quarterly integrity verification is compute-intensive for large Ledger.
- Ledger ingestion failure creates evidence gap (mitigated: buffered ingestion with chain continuity — max 1h).

## Compliance Mapping

## SOC2 CC7.2 (monitoring evidence preservation). SOC2 CC8.1 (change evidence). ISO A.8.15 (logging). ISO A.5.33 (protection of records). NIS2 Art.21 (incident evidence). GDPR Art.30 (processing records).
