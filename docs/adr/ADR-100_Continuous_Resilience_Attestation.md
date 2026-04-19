# ADR-100: Continuous Resilience Attestation

**Status:** Accepted

**Scope:** Universal

**Category:** Resilience

**Related ADRs:** 039

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's survivability design intent (≥99% tenant pod continuity during single-component failure) must be continuously validated, not just asserted.

## Candidates

## | Approach | Continuous (Chaos Mesh 24/7) | Quarterly manual | Annual DR drill only |
|---|---|---|---|
| Failure detection latency | Minutes (automated) | Months (next drill) | Year (next drill) |
| Attestation frequency | Hourly (prod) | Quarterly | Annual |
| Coverage | All blast-radius catalog components | Subset per drill | Resurrection only |

## Decision

## Chaos Mesh 24/7: hourly prod, 4-hourly staging. Each experiment produces `Resilience Proof` attestation in Sovereign Ledger: experiment ID, fault injected, MTTR measured, SLA comparison (pass/fail). MTTR > SLA target → P2 alert. Survivability rate measured quarterly per chaos drill (target ≥99% tenant pod continuity).

## Rejected

## - **Quarterly-only drills:** 73 components × multiple failure modes = hundreds of scenarios. Quarterly testing covers a fraction. Continuous testing covers them all over time.
- **No chaos testing:** Survivability is an untested assertion. "We designed for resilience" is not evidence — "we continuously prove resilience" is evidence.
- **Manual failure injection:** Not reproducible, not schedulable, not attestable. Human-injected faults don't produce systematic evidence.

## Consequences

## **Positive:**
- Continuous, automated evidence of platform resilience.
- MTTR measured objectively, not estimated.
- Resilience regressions detected within hours, not months.
- Signed attestations satisfy SOC2 CC7.1 and NIS2 resilience requirements.

**Negative:**
- Chaos experiments can cause real disruption on production (mitigated: blast-radius-scoped experiments, namespace-isolated).
- Experiment scheduling must avoid maintenance windows and high-traffic periods.
- Alert fatigue risk from frequent P2 alerts if SLA targets are too tight.

## Compliance Mapping

## SOC2 CC7.1 (resilience testing evidence). ISO A.5.30 (ICT readiness for business continuity). NIS2 Art.21 (business continuity testing).
