# ADR-047: PostgreSQL — CloudNativePG (Hetzner) / Azure PG Flexible (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** Data

**Related ADRs:** 067, 135

## Context

PostgreSQL is CAVE's primary relational database for tenant workloads, Backstage catalog, Keycloak, DevLake, Unleash, and Langfuse. Needs HA, automated backup, PITR, and Crossplane XR abstraction across both providers.


## Candidates

| Criteria | CNPG (Hz) + Azure PG Flexible (Az) | CNPG both | Zalando PG Operator | CrunchyData PGO |
|---|---|---|---|---|
| K8s operator | ✅ CNPG (CNCF Sandbox, EDB) | ✅ | ✅ Zalando | ✅ Crunchy |
| HA (auto-failover) | ✅ Streaming replication + auto-failover | ✅ | ✅ Patroni-based | ✅ |
| Backup/PITR | ✅ Barman Cloud → MinIO/ADLS | ✅ | ✅ WAL-G | ✅ pgBackRest |
| Managed option (Azure) | ✅ Azure PG Flexible (zone-redundant HA) | ❌ Self-managed on AKS | ❌ | ❌ |
| Connection pooling | ✅ PgBouncer integrated | ✅ | ⚠️ External | ⚠️ External |
| License | Apache 2.0 (CNPG) | Apache 2.0 | Apache 2.0 | Apache 2.0 |
| Community | Large (CNCF, EDB backing) | Large | Large (Zalando) | Moderate (Crunchy) |


## Decision

**CloudNativePG (CNPG)** on Hetzner (self-hosted, 3-replica HA, Barman Cloud → MinIO). **Azure PG Flexible Server** on Azure (zone-redundant HA, managed backup). Unified Database XRD via Crossplane. Dynamic credentials via OpenBao/Key Vault (ADR-020, ADR-083).


## Rejected Options

- **CNPG on both:** Would require self-managing PostgreSQL on AKS — operational burden when Azure PG Flexible provides managed HA, auto-backup, and zone-redundancy.
- **Zalando PG Operator:** Patroni-based HA is proven but CNPG is newer (CNCF), has integrated PgBouncer, and native Barman Cloud backup. CNPG's declarative model aligns better with Crossplane XR pattern.
- **CrunchyData PGO:** Capable but smaller community than CNPG/Zalando. pgBackRest is excellent but Barman Cloud is sufficient.


## Consequences

**Positive:**
- CNPG: CNCF project, active development, integrated PgBouncer, Barman Cloud backup to MinIO.
- Azure PG Flexible: managed HA, zone-redundancy, automatic backup, Private Link.
- Same XR API — developer doesn't know which backend serves their database.
- Dynamic credentials via ESO (ADR-053) — no static passwords.

**Negative:**
- CNPG + Azure PG Flexible have different operational characteristics — parity tests (ADR-135) must cover backup semantics, failover behavior, and PITR.
- CNPG operator updates must be validated against running clusters (in-place operator upgrade).
- Azure PG Flexible provider-side changes (SKU deprecation, default changes) require monitoring.

## Notes

**Universal scope** — Platform tenant DB + Cave Runtime cave-pg upstream parity. **Runtime mirror EXISTS**: `cave-pg` crate (Mirror-001 blanket; ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001'de detaylı multi-upstream case). Hibrit deployment'da CNPG ve Azure PG Flexible parity test corpus'ı (ADR-135) cave-pg behavioral parity hedefine de besleme yapar — sovereign deployment'da cave-pg ikisinin de wire'ını servis eder. Barman Cloud backup → cave-backup (ADR-046 mirror) entegrasyonu.

## Compliance Mapping

SOC2 CC6.7 (credential management — dynamic DB secrets). SOC2 CC7.5 (backup — automated PITR). ISO A.8.13 (information backup). ISO A.8.24 (encryption — TLS in transit, encryption at rest). GDPR Art.32 (security of processing — HA, encryption).

