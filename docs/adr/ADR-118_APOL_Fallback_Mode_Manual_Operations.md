# ADR-118: APOL Fallback Mode — Manual Operations

**Status:** Accepted

**Category:** AI Governance

**Related ADRs:** 112

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## APOL depends on LiteLLM + Prometheus + Argo Workflows. If all are unavailable simultaneously, the platform must continue operating with human-driven procedures.

## Candidates

## | Mode | APOL Normal | APOL Fallback |
|---|---|---|
| Class C (autonomous execution) | ✅ Active | ❌ Disabled |
| Reflex Engine (KEDA playbooks) | ✅ Active | ✅ Active (no LLM dependency) |
| Alerting | APOL + Grafana OnCall | Grafana OnCall only |
| Operations | AI-driven | Manual (cave-ctl + top-20 runbook) |

## Decision

## When APOL fully unavailable: `cave-ctl apol fallback --enable`. Class C disabled. Reflex Engine pre-approved playbooks continue via KEDA (Prometheus-triggered, no LLM). Alerts escalate to Grafana OnCall. Guardian uses cave-ctl CLI + documented top-20 operations runbook. Resume: `cave-ctl apol fallback --disable` after 10-minute stability window.

## Rejected

## - **No fallback:** APOL failure = platform operationally blind. No automated remediation. Alerts may not escalate.
- **Automatic AI restart without stability check:** Could cause oscillation (APOL starts → fails → restarts → fails → ...).
- **APOL required for platform operation:** Circular dependency. Platform must operate without AI — AI is an enhancement, not a prerequisite.

## Consequences

## **Positive:**
- Platform remains operable without AI.
- Pre-approved KEDA playbooks handle critical remediation without LLM.
- Clear fallback activation/deactivation procedure.
- 10-minute stability window prevents oscillation.

**Negative:**
- Manual operations slower than AI-driven. MTTR increases during fallback.
- Top-20 runbook may not cover all scenarios (covers most frequent 80% of incidents).
- Guardian must be available during fallback (APOL unavailable = higher human burden).

## Compliance Mapping

## SOC2 CC7.2 (continuity of monitoring during failure). ISO A.5.29 (operations during disruption). NIS2 Art.21 (business continuity).
