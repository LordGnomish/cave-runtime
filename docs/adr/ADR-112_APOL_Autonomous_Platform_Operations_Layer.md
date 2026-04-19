# ADR-112: APOL — Autonomous Platform Operations Layer

**Status:** Accepted

**Scope:** Universal

**Category:** AI Governance / Operations

**Related ADRs:** 092, 118, 125, 128, 136

## Context

73 components across 7 profiles require continuous operational attention. Manual operations don't scale. AI-assisted operations need strict boundaries.

## Candidates

| Role | Function | ML Method | Class C Scope |
|---|---|---|---|
| AI SRE | Scaling, restart, anomaly detection | Prophet (seasonal), River (streaming) | Pod restart, HPA adjust, namespace quarantine |
| AI Compliance Officer | Policy drift, RBAC anomaly, cert expiry | Rule engine + statistical anomaly | Alert + PR only (no auto-fix) |
| AI FinOps Governor | Spend prediction, right-sizing | Prophet (spend forecast) | Batch suspension, right-size recommendation |
| AI Change Manager | Upgrade validation pipeline | Rule-based + LLM analysis | Renovate PR → CI → chaos → canary |

## Decision

Four AI roles via cave-ctl MCP with bounded autonomy (ADR-136). Constitution Layer: multi-sig governance for role config. Model drift control: monthly retraining on 90-day baseline, 5% false-positive threshold → auto-retrain + P3 alert. Every decision produces reasoning trace → Sovereign Ledger (redacted per ADR-128).

## Rejected

- **No automation:** Full manual ops for 73 components. At 1 incident/week average across all components, manual response is unsustainable for sub-1 FTE toil target.
- **Full autonomy without bounds:** Uncontrolled blast radius. AI deletes namespace, scales to 100x, modifies security policy. Unacceptable.
- **Single AI role:** Separation of concerns. AI SRE optimizing for stability may conflict with AI FinOps optimizing for cost. Separate roles + conflict serializer (ADR-136) resolves this.

## Consequences

**Positive:**
- Routine operations automated (scaling, restart, cert renewal, dependency updates).
- 4-role separation prevents conflicting optimization goals.
- Model drift detection prevents degrading decision quality.
- Full audit trail via Sovereign Ledger reasoning traces.

**Negative:**
- AI can make policy-compliant but operationally wrong decisions ("constitutionally legal but operationally dumb").
- Model training requires 90-day Prometheus baseline — APOL is useless in first 3 months.
- LiteLLM dependency for reasoning trace generation — APOL fallback mode (ADR-118) handles LiteLLM failure.
- Conflict serializer adds latency to concurrent remediation actions.

## Compliance Mapping

SOC2 CC7.2 (automated monitoring and response). ISO A.5.26 (automated incident response). NIS2 Art.21 (automated security measures). ISO A.8.16 (monitoring activities).
