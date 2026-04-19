# ADR-139: Data Contract Governance

**Status:** Accepted

**Scope:** Azure, Runtime, Universal

**Category:** Platform Governance — Data

**Related ADRs:** 021 (Strimzi/Confluent), 047 (CNPG), 059 (Schema Registry), 060 (Debezium CDC), 074 (MLflow), 086 (Tenant Offboarding), 102 (Data Classification), 113 (Data Residency)

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's data platform spans PostgreSQL, Kafka, OpenSearch, Qdrant, MinIO/ADLS, MLflow, and Databricks. Data flows between these systems via CDC (Debezium), event streaming (Kafka), ML pipelines (Spark/Argo Workflows), and search indexing. Without formal data contract governance:

- Schema breaking changes propagate across services causing cascading failures
- Tenant offboarding leaves orphaned data across downstream systems
- Legal hold requests cannot be fulfilled because data lineage is unknown
- Cross-tenant data leakage is possible through shared analytics pipelines
- Replay of historical events has no defined boundary

---

## Candidates

## | Approach | Platform-enforced data contracts (chosen) | Application-managed (no platform rules) | Platform owns all schemas | Eventual consistency |
|---|---|---|---|---|
| Breaking change prevention | ✅ CI-enforced BACKWARD | ❌ Runtime failures | ✅ | ❌ Degrades over time |
| Team ownership clarity | ✅ Tenant owns domain, platform enforces | ❌ Unclear | ❌ Platform bottleneck | ⚠️ |
| Cross-tenant boundary | ✅ Schema + classification label | ❌ Unclear | ✅ | ❌ |
| Evolution capability | ✅ Governed (BACKWARD default) | ✅ Free-for-all | ❌ Platform slow | ✅ |
| Audit trail | ✅ Schema Registry + Git history | ❌ | ✅ | ❌ |

## Decision

## ### Schema Ownership

| Data Type | Owner | Registry | Breaking Change Policy |
|---|---|---|---|
| Database schemas | Tenant Developer | Flyway/Liquibase (CI stage 6) | Forward + rollback scripts. CI validates both. Staging: auto-rollback on failure. Prod: human approval. (ADR-116) |
| Kafka event schemas | Tenant Developer + Platform review | Apicurio Schema Registry (Hz) / Confluent Schema Registry (Az) | BACKWARD compatibility default. Breaking change: 14-day deprecation notice + consumer migration verification. |
| Search index mappings | Tenant Developer | Backstage catalog | Non-breaking: auto. Breaking: rebuild policy documented per XR. |
| Vector collection schemas | Tenant Developer | Backstage catalog | Rebuild from source data. Treated as derived (not primary). |
| ML model artifacts | Data Scientist | MLflow Model Registry | Version-tracked. Previous version retained for rollback. |

### Tenant Data Lifecycle

| Phase | Actions | Evidence |
|---|---|---|
| **Onboarding** | Namespaces created, XRs provisioned, schemas initialized, Kafka topics created, search indices created | Ledger `Tenant Onboarded` attestation |
| **Active** | Normal CRUD, streaming, ML training. Classification enforced (ADR-102). Retention per classification. | Continuous OPA enforcement |
| **Legal hold** | `cave-ctl tenant hold create --tenant <n> --scope <s>` suspends deletion for specified data categories. Overrides normal retention. | Ledger `Legal Hold Active` attestation |
| **Offboarding** | 30-day grace → backup → downstream lineage cleanup: Kafka topics drained + deleted, search indices purged, vector collections dropped, ML artifacts archived, CDC connectors disabled, MinIO/ADLS tenant prefix purged | Ledger `Tenant Offboarded` with evidence chain listing every deleted resource |

### Cross-Tenant Boundaries

| Rule | Enforcement |
|---|---|
| No cross-tenant data access | OPA admission + Cilium NetworkPolicy (ADR-084) |
| No cross-tenant Kafka consumption | Topic ACLs per tenant-id. Schema Registry subjects scoped per tenant. |
| No cross-tenant search queries | OpenSearch/Azure AI Search index-level tenant isolation |
| Anonymized aggregate only | Cross-tenant analytics requires Tier B waiver (ADR-140). Aggregation must prove k-anonymity (k≥5). |

---

## Rejected

## - **No schema governance (application-managed):** Breaking schema changes detected only at runtime. Consumer failures. Data pipeline corruption. No platform-level enforcement.
- **Platform owns all schemas:** Platform team becomes bottleneck for every schema change. Tenants must own their domain schemas — platform enforces compatibility rules.
- **Eventual consistency (no contract enforcement):** Data quality degrades over time. Cross-tenant data boundaries become unclear. Audit impossible without contracts.

## Consequences

## ### Positive
- Schema breaking changes detected in CI before deployment
- Tenant offboarding is deterministic with evidence trail
- Legal hold enforceable across all data stores
- Cross-tenant data leakage prevented at admission + network + application layers

### Negative
- Schema registry adds operational complexity to Kafka stack
- Offboarding evidence chain requires coordination across 6+ data services
- Legal hold complicates normal retention automation

## Compliance Mapping

## SOC2 CC8.1 (data contract management). ISO A.5.12 (information classification — contract-level). GDPR Art.25 (data protection by design — schema governance). GDPR Art.30 (processing records — data contract documentation). NIS2 Art.21 (data governance).
