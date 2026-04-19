# ADR-106: Loki WORM-Backed Storage for Forensic Integrity

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** Security

**Related ADRs:** 093

## Context

Application and platform logs are critical forensic evidence during security incidents. If logs can be modified or deleted after the fact, they cannot be used as legal evidence.

## Candidates

| Storage Type | Tamper-proof | Delete-proof | Provider |
|---|---|---|---|
| MinIO Object Lock (Hetzner) | ✅ WORM | ✅ Object Lock retention | Self-hosted |
| ADLS Immutable Blob (Azure) | ✅ WORM | ✅ Legal hold + time-based | Azure |
| Standard Loki (S3/GCS) | ❌ Mutable | ❌ Deletable | Any |
| Elasticsearch | ❌ Mutable | ❌ Deletable (SSPL license issue too) | Self-hosted |

## Decision

Loki log chunks and index → MinIO Object Lock (Hetzner) / ADLS immutable blob storage (Azure). Delete APIs disabled via IAM deny policies. Logs cannot be tampered with or deleted during retention period.

## Rejected

- **Standard Loki storage (S3/GCS):** Mutable and deletable. Platform admin can modify or delete logs. Forensic evidence tamperable.
- **Elasticsearch:** SSPL license (same BSL concern as Vault). Mutable storage. Not WORM.
- **No WORM:** Forensic logs can be altered post-incident. Legal proceedings may reject tampered evidence.

## Consequences

**Positive:**
- Log evidence immutable for forensic investigations.
- WORM compliance satisfies legal evidence preservation requirements.
- Same Loki queries, same Grafana dashboards — WORM is transparent to consumers.
- IAM deny policies prevent even admin deletion during retention.

**Negative:**
- WORM storage costs higher than standard (retention lock prevents early deletion for cost savings).
- Retention policy must be carefully configured — over-retention increases storage costs and data liability.
- Loki compaction and retention GC must work within WORM constraints.

## Compliance Mapping

SOC2 CC7.2 (evidence preservation). ISO A.8.15 (logging). ISO A.5.33 (protection of records). NIS2 Art.21 (incident evidence integrity). GDPR Art.30 (processing records).
