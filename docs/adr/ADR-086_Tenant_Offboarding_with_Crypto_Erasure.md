# ADR-086: Tenant Offboarding with Crypto-Erasure

**Status:** Accepted

**Scope:** Runtime

**Category:** Multi-Tenancy

**Related ADRs:** 139

## Context

When a tenant is offboarded, all tenant data must be verifiably removed including encrypted backups, WORM-stored evidence, and derived artifacts. GDPR Art.17 (right to erasure) requires effective deletion.

## Candidates

| Data Store | Erasure Method | Verification |
|---|---|---|
| PostgreSQL (active) | DROP DATABASE + connection kill | DB existence check |
| PostgreSQL (backups) | Crypto-shredding: per-tenant encryption key destroyed | Key destruction attested |
| MinIO/ADLS (objects) | Prefix deletion + lifecycle policy | Object count verification |
| MinIO/ADLS (WORM backups) | Crypto-shredding: key destruction (WORM objects undecryptable) | Key destruction attested |
| Kafka | Topic drain + deletion + consumer group removal | Topic existence check |
| Search/Vector indices | Index deletion | Index existence check |
| ML artifacts (MLflow) | Experiment + model registry purge + backing objects | API + object store check |
| Sovereign Ledger | Tenant metadata redacted (anonymized hash), evidence chain preserved | Redaction attested |

## Decision

30-day grace period → tenant notification → data backup → workload termination → crypto-erasure sequence → Backstage/Grafana/Harbor/Kong cleanup → `Tenant Offboarded` attestation with complete resource inventory. `cave-ctl tenant offboard --tenant <n>` orchestrates the full sequence.

## Rejected

- **Immediate deletion (no grace period):** Tenant cannot retrieve their data. Risk of accidental data loss.
- **No crypto-erasure (standard deletion only):** WORM backups and encrypted backup snapshots remain readable. GDPR non-compliance — data is not effectively erased.
- **Keep backups indefinitely:** Storage cost grows. Data retention liability increases. GDPR Art.5(1)(e) storage limitation principle violated.

## Consequences

**Positive:**
- Complete, verifiable, GDPR-compliant tenant removal.
- Crypto-shredding ensures encrypted backups become irrecoverable without key.
- Sovereign Ledger evidence chain preserved (anonymized) for audit continuity.
- Automated via cave-ctl — no manual cleanup steps.

**Negative:**
- 30-day grace period delays resource reclamation.
- Crypto-shredding depends on per-tenant encryption being properly implemented at provisioning.
- Sovereign Ledger redaction (anonymized hash) preserves evidence chain but removes tenant identity — may complicate future audit requests.
- MLflow artifacts may reference external storage paths that must also be cleaned.

## Compliance Mapping

GDPR Art.17 (right to erasure). GDPR Art.5(1)(e) (storage limitation). SOC2 CC6.5 (deprovisioning). ISO A.8.10 (information deletion).
