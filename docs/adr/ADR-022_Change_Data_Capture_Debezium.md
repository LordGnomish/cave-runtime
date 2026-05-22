# ADR-022: Change Data Capture — Debezium

**Status:** Accepted

**Scope:** Hyperscaler, Sovereign, Universal

**Category:** Data & Messaging

**Related ADRs:** 021, 047

## Context

CAVE tenants need real-time data integration between PostgreSQL and downstream systems (Kafka, search indices, analytics). CDC captures database changes as events without modifying application code.

## Candidates

| Criteria | Debezium | Maxwell | pg_logical_replication | Custom triggers |
|---|---|---|---|---|
| Source databases | ✅ PostgreSQL, MySQL, MongoDB, SQL Server, Oracle | MySQL only | PostgreSQL only | Any |
| Kafka integration | ✅ Native (Kafka Connect) | ✅ | ❌ Direct replication only | ❌ Custom |
| Schema evolution | ✅ Avro/JSON with Schema Registry | ✅ JSON | ❌ | ❌ |
| Snapshot mode | ✅ Initial snapshot + streaming | ✅ | ❌ | ❌ |
| Community | Very large (Red Hat, CNCF ecosystem) | Small (Zendesk) | PostgreSQL native | N/A |
| License | Apache 2.0 | Apache 2.0 | PostgreSQL | N/A |

## Decision

**Debezium** via Kafka Connect for CDC from PostgreSQL (CNPG/Azure PG) to Kafka topics. Deployed as Strimzi KafkaConnector CRD (sovereign) or Confluent managed connector (Azure). Avro serialization with Schema Registry (ADR-060).

## Rejected Options

### Maxwell — Rejected

**Primary:** MySQL-only. CAVE's primary relational database is PostgreSQL (CNPG on the sovereign profile, Azure PG Flexible on Azure — ADR-047). Maxwell has zero PostgreSQL support.

### pg_logical_replication — Rejected

**Primary:** No Kafka integration. PostgreSQL's native logical replication only streams to another PostgreSQL instance. CAVE needs CDC events on Kafka topics for downstream consumers (search indexing, analytics, event sourcing).

**Secondary:** No schema evolution. Logical replication streams raw SQL changes. No Avro/JSON serialization, no Schema Registry integration.

### Custom Database Triggers — Rejected

**Primary:** Application-coupled anti-pattern. Triggers execute inside the database transaction — trigger failure blocks the write. CDC should be decoupled: capture happens asynchronously from WAL.

**Secondary:** No standardized event format, no snapshot mode, no offset tracking, no automatic recovery. Debezium provides all of these out of the box.

## Consequences

**Positive:**
- Real-time CDC without application code changes. Database changes appear as Kafka events within seconds.
- Avro + Schema Registry enables schema evolution governance.
- Debezium connector managed as K8s CRD (Strimzi) — GitOps-compatible.

**Negative:**
- Debezium connector failures require monitoring and restart procedures.
- WAL retention on source database must be configured to prevent WAL overflow during connector outages.
- Snapshot mode for large tables can be resource-intensive.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Debezium connector lag under high write load | Medium | Medium | Monitor connector lag via Prometheus. Scale Kafka Connect workers. Tune batch size and poll interval. |
| Schema evolution breaks CDC pipeline | Medium | High | Schema Registry (ADR-060) validates schema compatibility before deployment. Debezium schema history topic preserves old schemas. |
| Debezium upgrade breaks connector config | Low | Medium | Pin connector version. Staging validates before prod. Automated connector config backup. |
| WAL retention fills PostgreSQL disk | Low | High | Monitor replication slots. Alert on WAL growth. Auto-drop stale slots after 24h. |

## Compliance Mapping

SOC2 CC8.1 (data integration via controlled pipeline). GDPR Art.30 (processing records — CDC events as processing activity).
