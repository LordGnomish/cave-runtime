# ADR-050: Object Storage — MinIO (Hetzner) / ADLS Gen2 (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** Data

**Related ADRs:** 067, 093, 106, 135 | Absorbs: ADR-048

Status:

Category:

Data

Related ADRs:

067, 093, 106, 135

Back to Index:

## Context

CAVE needs object storage for: Loki log chunks, Tempo traces, Velero backups, MLflow artifacts, Sovereign Ledger (WORM), Thanos long-term metrics, tenant application data. Must support S3 API compatibility (industry standard).


## Candidates

| Criteria | MinIO (Hz) + ADLS (Az) | MinIO both | ADLS both | Ceph/Rook |
|---|---|---|---|---|
| S3 API compatible | ✅ MinIO (native S3) | ✅ | ⚠️ ADLS uses Azure Blob API (S3 compatibility via gateway) | ✅ |
| K8s operator | ✅ MinIO Operator | ✅ | N/A (managed) | ✅ Rook operator |
| WORM/Object Lock | ✅ MinIO Object Lock | ✅ | ✅ ADLS immutable blob | ⚠️ |
| Managed option | ✅ ADLS (Azure) | ❌ | ✅ | ❌ |
| Erasure coding | ✅ MinIO EC | ✅ | N/A | ✅ |
| License | AGPL-3.0 (MinIO) | AGPL-3.0 | Azure terms | Apache 2.0 |


## Decision

**MinIO** (self-hosted, MinIO Operator) on Hetzner. **ADLS Gen2** (managed, Private Endpoint) on Azure. Unified Bucket XRD via Crossplane. MinIO Object Lock for WORM (Sovereign Ledger, forensic bucket). ADLS immutable blob for Azure WORM.


## Rejected Options

- **MinIO on both:** Would self-manage object storage on AKS when ADLS provides managed, geo-redundant storage with native Azure integration.
- **ADLS on both:** Not available on Hetzner. Cannot self-host.
- **Ceph/Rook:** Powerful but extremely complex to operate. Overkill for CAVE's object storage needs. MinIO is simpler for S3-compatible object storage.


## Consequences

**Positive:**
- MinIO: S3-native, WORM support, erasure coding, self-hosted sovereignty.
- ADLS: managed, geo-redundant (GRS), immutable blob, Private Endpoint.
- Unified Bucket XR abstracts both behind same API.

**Negative:**
- MinIO AGPL-3.0 (acceptable — internal platform use, not distributed as SaaS).
- MinIO cluster management (scaling, disk management, erasure coding config).
- S3 ↔ ADLS API differences handled by Crossplane Compositions but some edge cases may need provider-specific handling.
- TB-scale migration between MinIO and ADLS requires rclone (ADR-066 migration state machine).

Compliance Mapping

SOC2 CC7.5 (data retention — WORM). SOC2 CC6.7 (encryption at rest). ISO A.8.13 (backup storage). ISO A.5.33 (protection of records — WORM). NIS2 Art.21 (data protection). GDPR Art.32 (encryption, availability).

Absorbed Decisions:

The following tool-level decisions are absorbed into this ADR for traceability

MinIO Selection Rationale

Decision:

MinIO as object storage on Hetzner. Rejection: Ceph (operational complexity, RBD focus), SeaweedFS (smaller ecosystem, less mature erasure coding). MinIO Object Lock delivers WORM.

