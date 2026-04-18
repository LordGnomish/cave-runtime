# ADR-060: Schema Registry & Evolution Policy

**Status:** Accepted

**Category:** Data & Messaging

**Related ADRs:** 021, 059, 139

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Event-driven architecture requires schema management to prevent breaking changes between producers and consumers. Schema evolution must be governed to protect downstream consumers.

## Candidates

## | Criteria | Apicurio (Hz) + Confluent SR (Az) | Confluent SR both | AWS Glue SR | Custom |
|---|---|---|---|---|
| Self-hosted | ✅ Apicurio on K8s | ❌ Confluent SR requires Confluent Platform | ❌ AWS | ❌ |
| Managed | ✅ Confluent SR (Azure) | ✅ | ✅ | ❌ |
| Avro/JSON/Protobuf | ✅ All | ✅ All | ⚠️ Avro, JSON | ⚠️ |
| Compatibility modes | ✅ BACKWARD, FORWARD, FULL, NONE | ✅ | ⚠️ | ❌ |
| Kafka integration | ✅ Serializer/Deserializer | ✅ | ⚠️ | ❌ |
| License | Apicurio: Apache 2.0. Confluent SR: Confluent Community License | Confluent CL | AWS terms | N/A |

## Decision

## **Apicurio Registry** (Apache 2.0) on Hetzner. **Confluent Schema Registry** on Azure (included with Confluent Cloud). Default compatibility mode: BACKWARD (consumers can read data produced with older schema). CI validates schema compatibility before topic deployment. Schema evolution blocked during cross-provider migration freeze.

## Rejected

## - **Confluent SR on both:** Confluent Community License is more restrictive than Apache 2.0. Apicurio is fully open source.
- **AWS Glue SR:** AWS-only.
- **No schema registry:** Breaking changes between producers and consumers undetected until runtime failure.

## Consequences

## **Positive:**
- Schema evolution governed — breaking changes caught in CI before deployment.
- BACKWARD compatibility ensures consumers always handle older data.
- Apicurio (Apache 2.0) + Confluent SR (managed on Azure) — best of both.

**Negative:**
- Two SR implementations — compatibility mode behavior may differ slightly.
- Apicurio operational overhead (PostgreSQL backend, K8s deployment).
- Schema migration between Apicurio ↔ Confluent SR during provider migration requires export/import.

## Compliance Mapping

## SOC2 CC8.1 (data contract management). ISO A.14.2 (secure development — data contract validation).
