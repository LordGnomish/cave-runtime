# ADR-022: Change Data Capture — Debezium

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** Data & Messaging

**Related ADRs:** 021, 047

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE tenants need real-time data integration between PostgreSQL and downstream systems (Kafka, search indices, analytics). CDC captures database changes as events without modifying application code.

## Candidates

## | Criteria | Debezium | Maxwell | pg_logical_replication | Custom triggers |
|---|---|---|---|---|
| Source databases | ✅ PostgreSQL, MySQL, MongoDB, SQL Server, Oracle | MySQL only | PostgreSQL only | Any |
| Kafka integration | ✅ Native (Kafka Connect) | ✅ | ❌ Direct replication only | ❌ Custom |
| Schema evolution | ✅ Avro/JSON with Schema Registry | ✅ JSON | ❌ | ❌ |
| Snapshot mode | ✅ Initial snapshot + streaming | ✅ | ❌ | ❌ |
| Community | Very large (Red Hat, CNCF ecosystem) | Small (Zendesk) | PostgreSQL native | N/A |
| License | Apache 2.0 | Apache 2.0 | PostgreSQL | N/A |

## Decision

## **Debezium** via Kafka Connect for CDC from PostgreSQL (CNPG/Azure PG) to Kafka topics. Deployed as Strimzi KafkaConnector CRD (Hetzner) or Confluent managed connector (Azure). Avro serialization with Schema Registry (ADR-060).

## Rejected

## - **Maxwell:** MySQL-only. CAVE's primary database is PostgreSQL.
- **pg_logical_replication:** PostgreSQL-native but only replicates to another PostgreSQL. No Kafka integration. No schema evolution.
- **Custom triggers:** Brittle, application-coupled, no standardized event format. Anti-pattern for platform-managed CDC.

## Consequences

## **Positive:**
- Real-time CDC without application code changes. Database changes appear as Kafka events within seconds.
- Avro + Schema Registry enables schema evolution governance.
- Debezium connector managed as K8s CRD (Strimzi) — GitOps-compatible.

**Negative:**
- Debezium connector failures require monitoring and restart procedures.
- WAL retention on source database must be configured to prevent WAL overflow during connector outages.
- Snapshot mode for large tables can be resource-intensive.

## Compliance Mapping

## SOC2 CC8.1 (data integration via controlled pipeline). GDPR Art.30 (processing records — CDC events as processing activity).
