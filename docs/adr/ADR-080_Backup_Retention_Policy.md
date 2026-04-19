# ADR-080: Backup Retention Policy

**Status:** Accepted

**Scope:** Universal

**Category:** DR

**Related ADRs:** 046, 088

Status:

Category:

DR

Related ADRs:

046, 088

Back to Index:

## Context

CAVE needs defined backup retention periods per data type, per environment. Retention must balance: recovery capability, storage cost, compliance requirements (GDPR storage limitation), and forensic needs.


## Candidates

| Approach | Tier-based retention (chosen) | Uniform retention (all same) | Keep everything indefinitely | Minimal retention |
|---|---|---|---|---|
| Storage cost | ✅ Proportional to value | ❌ Over-provisioned dev/staging | ❌ Unbounded growth | ✅ Lowest |
| Recovery capability | ✅ Adequate per environment | ⚠️ Same window regardless | ✅ Maximum | ❌ Insufficient for prod |
| GDPR storage limitation | ✅ Defined TTL per data type | ⚠️ May violate Art.5(1)(e) | ❌ Violation | ✅ |
| Forensic window | ✅ 2y prod, 180d staging | ⚠️ Uniform | ✅ Unlimited | ❌ Short |
| Constitutional artifacts (Ledger) | ✅ Indefinite exception | ⚠️ Same as other data | ✅ | ❌ Violates audit trail |


## Decision

| Data Type | Dev | Staging | Prod | WORM |
|---|---|---|---|---|
| PostgreSQL (PITR WAL) | 1d | 3d | 7d | N/A |
| PostgreSQL (base backup) | 3d | 7d | 30d | 90d |
| Velero (K8s resources) | 3d | 7d | 30d | N/A |
| MinIO/ADLS (object) | 7d | 14d | 30d | Per WORM policy |
| etcd snapshot | 1d | 3d | 7d (5-min interval) | 30d |
| Sovereign Ledger | N/A | N/A | Indefinite (constitutional) | Indefinite |
| Forensic (Tetragon/Hubble) | 90d | 180d | 2y | Same as prod |

WORM escrow (cross-region): etcd snapshots + Git mirror + Talos configs + Sovereign Ledger replica. Retention: indefinite for Ledger, 90d for etcd, hourly for Git mirror.


## Rejected Options

- **Uniform retention across all environments:** Dev/staging don't need 30-day retention. Wastes storage. Different environments have different recovery requirements.
- **No WORM for Ledger:** Sovereign Ledger is constitutional artifact — must be indefinitely preserved. Standard retention would violate audit requirements.
- **Longer prod retention (90d+):** Storage cost scales linearly. 30d base backup + 7d WAL + WORM escrow provides adequate recovery window. Longer retention not justified by compliance requirements.
- **No retention policy (keep everything):** Storage cost grows unbounded. GDPR Art.5(1)(e) storage limitation principle requires defined retention.


## Consequences

**Positive:**
- Clear retention per data type — no ambiguity for operations or compliance.
- WORM for Ledger ensures indefinite non-repudiation.
- Short dev/staging retention minimizes storage cost.

**Negative:**
- Prod 30d base backup + 7d WAL = 37d PITR window. Longer recovery needs WORM.
- Storage cost scales with retention duration × data volume.
- GDPR storage limitation (Art.5(1)(e)) requires active purge after retention period.

Compliance Mapping

SOC2 CC7.5 (backup retention). ISO A.8.13 (backup policy). GDPR Art.5(1)(e) (storage limitation). NIS2 Art.21 (data protection — backup).

