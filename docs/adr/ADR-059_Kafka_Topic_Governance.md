# ADR-059: Kafka Topic Governance

**Status:** Accepted

**Category:** Data & Messaging

**Related ADRs:** 021, 060, 139

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's multi-tenant Kafka (Strimzi on Hetzner, Confluent on Azure) requires strict topic governance. Without it: topic naming sprawl, uncontrolled partition allocation, inconsistent retention, cross-tenant data access, and orphaned topics after tenant offboarding.

## Candidates

## | Approach | Platform-enforced governance (chosen) | Application-managed | No governance |
|---|---|---|---|
| Naming convention | ✅ OPA validates at creation | ❌ Developer choice | ❌ |
| ACL enforcement | ✅ SASL/SCRAM per tenant | ❌ Application-level | ❌ |
| Schema enforcement | ✅ Schema Registry (ADR-060) | ⚠️ Optional | ❌ |
| Cleanup automation | ✅ cave-ctl tenant offboard | ❌ Manual | ❌ |

## Decision

## **Platform-enforced Kafka topic governance:**

| Aspect | Policy |
|---|---|
| **Naming** | `<tenant-id>.<domain>.<event-type>.<version>` (e.g., `acme.billing.invoice-created.v1`) |
| **Partitioning** | Minimum 3 partitions (matching RF=3). Key-based for ordering guarantees. |
| **Retention** | Tier-based: Soft 7d, Hard 30d, Dedicated custom |
| **ACL** | SASL/SCRAM per tenant. Producer/consumer restricted to own tenant prefix. |
| **Schema** | Avro with Schema Registry (ADR-060). BACKWARD compatibility enforced. |
| **OPA validation** | Topic creation validated against naming convention. Cross-tenant prefix blocked. |
| **Cleanup** | Tenant offboarding (ADR-086) drains and deletes all tenant topics + consumer groups. |
| **Cross-provider** | Kafka not synced cross-provider. Migration = clean cutover (consumers replay from earliest). |

## Rejected

## - **Application-managed topics:** Developers create topics with arbitrary names. Topic sprawl. No naming convention. No automated cleanup. Cross-tenant access possible without ACL enforcement.
- **No governance:** Complete anarchy. Topics accumulate. Retention inconsistent. ACLs missing.
- **Topic-per-microservice (not per-tenant):** Doesn't provide tenant isolation. Cross-tenant data visible on shared topics.

## Consequences

## **Positive:**
- Structured naming enables automated cleanup, monitoring, and cost attribution.
- ACL isolation prevents cross-tenant data access at Kafka protocol level.
- Schema enforcement prevents breaking changes in event contracts.
- OPA validation catches governance violations at creation time, not post-deployment.

**Negative:**
- Naming convention enforcement requires OPA admission policy + Strimzi/Confluent ACL config.
- Partition count changes require topic recreation (Kafka limitation) — initial partition count must be planned.
- Schema Registry adds operational complexity.
- Cross-provider migration loses consumer offsets — acknowledged trade-off (ADR-066).

## Compliance Mapping

## SOC2 CC6.1 (topic access controls — SASL/SCRAM per tenant). GDPR Art.32 (tenant data isolation in messaging). ISO A.8.22 (data segregation — topic-level isolation). NIS2 Art.21 (secure communications — encrypted Kafka with TLS).
