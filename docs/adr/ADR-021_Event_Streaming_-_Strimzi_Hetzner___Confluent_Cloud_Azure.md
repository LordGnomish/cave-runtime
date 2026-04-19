# ADR-021: Event Streaming — Strimzi (Hetzner) / Confluent Cloud (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** Data & Messaging

**Related ADRs:** 059, 060, 067, 135

## Context

CAVE tenants need event streaming for asynchronous communication, event sourcing, CDC pipelines, and real-time data integration. The solution must be available on both providers via Crossplane XR abstraction.


## Candidates

| Criteria | Strimzi (Hz) + Confluent (Az) | Strimzi both | Confluent both | Amazon MSK | Redpanda |
|---|---|---|---|---|---|
| Self-hosted K8s | ✅ Strimzi operator | ✅ | ❌ SaaS only | ❌ AWS | ✅ |
| Managed option | ✅ Confluent Cloud (Az) | ❌ | ✅ | ❌ | ❌ |
| Kafka protocol | ✅ Apache Kafka | ✅ | ✅ | ✅ | ✅ Compatible |
| Schema Registry | ✅ Strimzi + Apicurio (Hz), Confluent SR (Az) | ✅ | ✅ | ⚠️ Glue SR | ✅ Bundled |
| Connect ecosystem | ✅ Kafka Connect (Debezium CDC) | ✅ | ✅ | ✅ | ⚠️ Less mature |
| Private networking | ✅ In-cluster (Hz), Private Link (Az) | ✅ | ✅ Private Link | ✅ | ✅ |
| License | Strimzi: Apache 2.0. Confluent: Commercial SaaS | Apache 2.0 | Commercial | AWS terms | BSL 1.1 (core), Apache (client) |


## Decision

**Strimzi** (Kafka operator, Apache 2.0) self-hosted on Hetzner. **Confluent Cloud** (managed Kafka) on Azure via Private Link. Unified MessageBus XRD via Crossplane. Schema Registry: Apicurio (Hetzner), Confluent Schema Registry (Azure).


## Rejected Options

- **Strimzi on both providers:** Would require self-managing Kafka on AKS — operational burden when Confluent Cloud provides managed alternative with enterprise SLA, auto-scaling, and Private Link.
- **Confluent on both:** SaaS-only. Cannot self-host on Hetzner. Contradicts sovereign profile.
- **Amazon MSK:** AWS-only. Not available on Hetzner or Azure.
- **Redpanda:** BSL 1.1 core license (same concern as Vault/Redis). Despite impressive performance claims, BSL disqualifying per CAVE's licensing principles. Additionally, Kafka Connect ecosystem compatibility is less proven.


## Consequences

**Positive:**
- Apache Kafka protocol on both providers — all Kafka clients work unmodified.
- Strimzi on Hetzner: full sovereign control, Apache 2.0, operator-managed lifecycle.
- Confluent on Azure: enterprise SLA, auto-scaling, Tiered Storage, Private Link.
- Unified XR abstracts both behind MessageBus API.
- Schema Registry on both providers enables schema evolution governance (ADR-060).

**Negative:**
- Kafka cross-provider migration is clean cutover (no offset migration). Consumers must replay from earliest. RPO for cross-provider is data-loss-accepted. Within-cluster RPO < 15min via RF=3.
- Strimzi operational complexity (broker scaling, partition rebalancing, ZooKeeper→KRaft migration).
- Confluent Cloud cost can be significant for high-throughput tenants.
- Schema Registry implementations differ (Apicurio vs Confluent SR) — compatibility mode ensures format compatibility but advanced features may differ.

### KRaft Migration Note

Apache Kafka is migrating from ZooKeeper to KRaft (Kafka Raft) for metadata management. ZooKeeper is deprecated in Kafka 3.5+ and will be removed in Kafka 4.0 (expected 2026-2027). Strimzi supports KRaft mode since Strimzi 0.40+. CAVE should deploy new Strimzi clusters in KRaft mode and plan migration of existing ZK clusters before Kafka 4.0.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Strimzi ZooKeeper removal (Kafka 4.0) | High (2027) | Medium | Deploy new clusters in KRaft mode. Migrate existing ZK clusters via Strimzi KRaft migration procedure. Test in staging first. |
| Confluent Cloud pricing increase | Medium | Medium | FinOps monitoring (ADR-096). Strimzi on AKS is fallback (self-hosted, same as Hetzner). Annual cost review. |
| Cross-provider offset migration (Hz→Az) | Low | High | Accept clean cutover model. Consumers replay from earliest. RPO = data in transit during cutover. Document in runbook. |
| Strimzi operator upgrade breaks brokers | Low | High | Pin Strimzi version. Staging validates upgrade. Broker rolling restart policy. |
| Schema Registry divergence (Apicurio vs Confluent SR) | Medium | Medium | Parity tests (ADR-135) cover schema registration and compatibility validation. Stick to Avro + backward compatibility mode on both. |
| WarpStream disrupts Kafka landscape | Low (2027+) | Low | **Watch:** Confluent acquired WarpStream (Kafka on object storage). If WarpStream becomes managed offering on Azure, evaluate as Confluent Cloud alternative with potentially lower cost. |

Compliance Mapping

SOC2 CC6.1 (access controls — SASL/SCRAM per tenant). SOC2 CC6.6 (encryption — TLS in transit). ISO A.8.24 (encryption). GDPR Art.32 (security of processing — tenant data isolation via topic ACLs). NIS2 Art.21 (secure communications).

