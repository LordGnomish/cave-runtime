# ADR-126: Workload Criticality Labels for Kill-Switch Ethics

**Status:** Accepted

**Category:** FinOps

**Related ADRs:** 096

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## FinOps kill-switch must know which workloads can be suspended and which are business-critical. Without labels, kill-switch could suspend payment processing.

## Candidates

## | Label | Budget Threshold | Suspension | Example Workloads |
|---|---|---|---|
| `business-critical` | Never auto-suspended | ❌ Human-only | Payment service, auth, core data pipeline |
| `standard` | 150% | ✅ Automatic (graceful) | Application services, APIs, dashboards |
| `batch` | 120% | ✅ Automatic (graceful) | CI runners, ML training, report generation |

## Decision

## Three mandatory labels on all Deployments/StatefulSets. OPA rejects unlabeled workloads. K8s PriorityClass enforces scheduling priority (business-critical > standard > batch). Kill-switch ethics: business-critical and identity/auth services NEVER auto-suspended. Graceful shutdown: SIGTERM → preStop → drain → checkpoint verification. StatefulSet suspension aborted if checkpoint fails.

## Rejected

## - **No criticality labels:** Kill-switch cannot distinguish payment service from ML training job. Suspending payment processing = revenue loss.
- **Binary (critical/non-critical):** Too coarse. Batch workloads should be suspended before standard workloads. Three tiers enable graduated response.
- **Optional labels:** Unlabeled workloads get default treatment — but default is ambiguous. Mandatory labels + OPA enforcement eliminates ambiguity.

## Consequences

## **Positive:**
- Kill-switch makes informed decisions about what to suspend.
- Business-critical workloads protected from automated cost controls.
- Graduated suspension (batch at 120%, standard at 150%) minimizes business impact.
- Graceful shutdown with checkpoint verification prevents data corruption.

**Negative:**
- Label maintenance burden — every deployment must have criticality label.
- Misclassification risk (standard labeled as business-critical to avoid suspension).
- Graceful shutdown adds time to suspension process (~30-60s per pod).

## Compliance Mapping

## SOC2 CC6.1 (risk-based access controls). ISO A.5.12 (information classification — applied to workload criticality). NIS2 Art.21 (availability management).
