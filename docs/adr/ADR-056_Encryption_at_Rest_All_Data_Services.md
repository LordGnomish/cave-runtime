# ADR-056: Encryption at Rest — All Data Services

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** Security

**Related ADRs:** 020, 086, 105

## Context

CAVE stores tenant data across multiple systems: PostgreSQL, Kafka, MinIO/ADLS, OpenSearch/AI Search, Qdrant, etcd, backup storage, Sovereign Ledger. All data at rest must be encrypted — both for compliance (GDPR Art.32, SOC2 CC6.7) and for crypto-erasure capability (ADR-086: destroying encryption key renders data irrecoverable).

## Candidates

| Approach | Per-service encryption (chosen) | Volume-level encryption only | No encryption |
|---|---|---|---|
| Granularity | ✅ Per-service, per-tenant key possible | ⚠️ Volume-level only | ❌ |
| Crypto-erasure | ✅ Per-tenant key destruction | ❌ Can't shred per-tenant | ❌ |
| Performance | ✅ AES-NI hardware acceleration | ✅ | ✅ |
| Managed service compat | ✅ Works with Azure managed + self-hosted | ⚠️ Only block storage | ❌ |

## Decision

All data services encrypted at rest with the strongest available method:

| Data Service | Hetzner Encryption | Azure Encryption | Per-Tenant Key |
|---|---|---|---|
| PostgreSQL | CNPG volume encryption + TDE if supported | Azure PG: automatic (Microsoft-managed or CMK) | ✅ CMK via Key Vault |
| Kafka | Strimzi: volume encryption | Confluent: automatic | ❌ Topic-level ACL only |
| MinIO | SSE-S3 with OpenBao-managed keys | ADLS: Microsoft-managed or CMK | ✅ Per-tenant prefix key |
| etcd | KMS encryption via OpenBao Transit (ADR-105) | AKS: Key Vault KMS provider | N/A (platform-level) |
| Search | OpenSearch: volume encryption | AI Search: automatic | ❌ Index-level ACL |
| Vector | Qdrant: volume encryption | AI Search: automatic | ❌ |
| Backup | Velero: encrypted backups to MinIO/ADLS | Same | ✅ Per-tenant key (crypto-erasure) |
| WORM | MinIO Object Lock: SSE-S3 | ADLS immutable: CMK | N/A (platform-level) |

Per-tenant encryption keys stored in OpenBao (Hetzner) / Key Vault (Azure). Key destruction during tenant offboarding (ADR-086) renders all encrypted tenant data irrecoverable — crypto-erasure for GDPR compliance.

## Rejected

- **Volume-level encryption only:** Encrypts the disk but not per-tenant. Cannot destroy per-tenant key for crypto-erasure. Insufficient for GDPR "effective erasure."
- **No encryption:** GDPR Art.32 violation. SOC2 CC6.7 violation. Unacceptable.
- **Application-level encryption only:** Would require every application to implement encryption — platform-level enforcement is more reliable.

## Consequences

**Positive:**
- Complete encryption at rest across all data stores.
- Per-tenant keys enable crypto-shredding for GDPR Art.17 erasure.
- Managed services (Azure) encrypt by default — minimal additional configuration.
- AES-NI hardware acceleration means negligible performance impact.

**Negative:**
- Per-tenant key management increases OpenBao/Key Vault key count (N tenants × M data services).
- Key rotation coordination across multiple services.
- Kafka topic-level encryption is limited — ACL isolation is primary tenant boundary, encryption protects at-rest only.
- Qdrant and OpenSearch per-tenant key support depends on operator capabilities.

## Compliance Mapping

SOC2 CC6.7 (encryption of stored data). ISO A.8.24 (cryptographic controls — encryption at rest). GDPR Art.32 (security of processing — encryption). GDPR Art.17 (right to erasure — crypto-erasure via key destruction). NIS2 Art.21 (data protection — encryption).
