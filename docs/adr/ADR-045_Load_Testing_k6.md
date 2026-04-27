# ADR-045: Load Testing — k6

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 010

## Context

CAVE needs performance validation before production promotion. Load tests must be repeatable, CI-integrated, and produce quantifiable results (latency percentiles, error rates, throughput).

## Candidates

| Criteria | k6 | JMeter | Locust | Gatling |
|---|---|---|---|---|
| Script language | ✅ JavaScript/TypeScript | ❌ XML/GUI | ✅ Python | ✅ Scala/Java |
| CI integration | ✅ CLI, exit codes | ⚠️ Heavy JVM | ✅ | ✅ |
| K8s distributed | ✅ k6-operator (CRD) | ⚠️ | ⚠️ | ⚠️ |
| Cloud export | ✅ Prometheus remote write | ⚠️ | ⚠️ | ⚠️ |
| Resource footprint | ✅ Lightweight (Go binary) | ❌ JVM heavy | ✅ | ❌ JVM |
| License | AGPL-3.0 | Apache 2.0 | MIT | Apache 2.0 |

## Decision

**k6** for load testing (Phase 2 — Cave Runtime parity benchmarks require it). Golden Path templates include k6 test scaffold. k6-operator for distributed load tests. Results exported to Prometheus. SLO thresholds defined per service.

## Rejected

- **JMeter:** JVM-heavy. XML config is maintenance nightmare. Poor CI experience.
- **Locust:** Good but k6 has better K8s operator and Prometheus integration.
- **Gatling:** JVM-based. Scala scripting is niche.

## Consequences

**Positive:**
- JavaScript test scripts — accessible to most developers.
- K8s operator enables distributed load generation from within cluster.
- Prometheus export integrates with existing observability stack.

**Negative:**
- AGPL-3.0 (acceptable for internal CI tool, not distributed as SaaS).
- k6 scripts require maintenance as APIs change.

## Notes

**Universal scope** — Platform tenant performance validation + Cave Runtime crate'lerin sürekli benchmarking'i. **Runtime mirror REQUIRED**: cave-loadgen crate (Mirror-001 blanket scope, single upstream, no override ADR) k6 davranışını runtime içinde reimpl eder; sovereign/disconnected deployment'larda CI-time k6 runner kırılırsa runtime kendi load generation'ını koşar — cave-self-improver gece pump'ında her crate'i kendi performance regression suite'iyle doğrular, Reflex Engine load-anomaly detection için load-bearing.

**Phase rebalance:** Önceki "Phase 4 on-demand" kararı Phase 2'ye çekildi — Cave Runtime parity testleri zaten benchmark gerektiriyor (cave-apiserver request latency, cave-streams throughput, cave-net packet rate); load testing altyapı seviyesinde, opsiyonel değil.

## Compliance Mapping

SOC2 CC7.1 (performance testing). ISO A.14.2.9 (system acceptance — performance validation). NIS2 Art.21 (availability — capacity testing).
