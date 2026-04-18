# ADR-136: APOL Bounded Autonomy Model

**Status:** Accepted

**Category:** Platform Governance — AI Operations

**Related ADRs:** 092 (AI Guardrails), 095 (Reflex Engine), 112 (APOL), 118 (APOL Fallback), 119 (Crossplane Operations), 125 (APOL CoT Audit), 128 (Attestation Redaction)

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## APOL's four AI roles (AI SRE, Compliance Officer, FinOps Governor, Change Manager) operate via cave-ctl MCP. ADR-092 defines allowlist/denylist/ceiling, and ADR-112 defines roles and trust boundaries. However, the boundary between "autonomous execution" and "recommendation only" is not formally structured.

Without explicit boundaries:
- An AI SRE could chain multiple autonomous actions that individually are safe but collectively create blast-radius exceeding expectations
- Multiple APOL roles could execute concurrent changes that interact unexpectedly
- No mechanism exists to globally freeze autonomous operations during instability

---

## Candidates

## | Approach | Class A-D bounded autonomy (chosen) | Full autonomy (no bounds) | No AI automation | Per-action human approval |
|---|---|---|---|---|
| Blast-radius control | ✅ Class C guardrails enforce | ❌ Unbounded | N/A | ✅ Pre-approved |
| Operator fatigue | ✅ Routine operations automated | ✅ | ❌ Full manual burden | ❌ Approval fatigue |
| Conflict resolution | ✅ Serializer + priority order | ❌ Race conditions | N/A | ⚠️ Slow |
| Rollback capability | ✅ Required for Class C | ❌ Not enforced | N/A | ✅ |
| Audit trail | ✅ Reasoning trace per action | ⚠️ | N/A | ✅ |

## Decision

## ### Action Classification

| Class | Capability | Autonomous? | Examples |
|---|---|---|---|
| **A — Observe** | Read metrics, logs, state. Analyze. Report. | Always | Health checks, anomaly detection, cost analysis, drift detection |
| **B — Prepare** | Create PRs, draft remediation plans, prepare rollback. | Always | Renovate PRs, scaling proposals, migration plans, RCA drafts |
| **C — Execute** | Run signed remediation playbook within bounds. | Yes, with guardrails | Pod restart, HPA adjustment, cert renewal, secret rotation, egress quarantine (per tier thresholds) |
| **D — Constitutional** | Modify XRDs, OPA core policies, ADR set, Ledger config, identity root, compatibility matrix. | Never autonomous | Always guardian multi-sig (2-of-3 + hardware key) |

### Class C Guardrails

Every autonomous execution (Class C) requires ALL of:

| Guardrail | Requirement |
|---|---|
| **Signed playbook** | Playbook must be cosign-signed, stored in Git, version-controlled |
| **Blast-radius classification** | `pod` / `namespace` / `cluster` / `cross-cluster`. Max autonomous: `namespace`. |
| **Rollback path** | Pre-verified rollback procedure. Rollback must be executable without human. |
| **SLO guardrail** | Action must not degrade any platform SLO below target. Pre-check required. |
| **Budget guardrail** | Estimated cost impact < threshold (configurable per profile). Exceeds → Class A (recommend only). |
| **Tenant impact estimate** | Number of affected tenants. >3 tenants → human approval required. |
| **Ledger attestation** | Reasoning trace written before execution (not after). |

### Concurrency Limits

| Profile | Max Concurrent Autonomous Changes | Rationale |
|---|---|---|
| prod | 3 | Limit blast-radius of interacting changes |
| staging | 5 | Higher tolerance for validation |
| dev | Unlimited | Development environment |

If concurrency limit reached, new Class C actions queue or downgrade to Class A (recommendation).

### Global Autonomy Freeze Triggers

All Class C actions immediately halt (downgrade to Class A) when ANY of these conditions are detected:

| Trigger | Detection | Resume Condition |
|---|---|---|
| Control-plane instability | ArgoCD sync failures > 3 in 5 min, Crossplane reconciliation stall | ArgoCD healthy + Crossplane reconciling for 10 min |
| Observability blind spot | Prometheus scrape failures > 20%, Loki ingestion stopped | Full observability restored for 5 min |
| Identity drift unresolved | `cave-ctl identity drift` finds > 5 orphaned bindings | Drift resolved to 0 |
| Ledger verification failure | Ledger integrity check fails | Ledger verified from WORM escrow |
| Policy bundle signature failure | OPA bundle fails cosign verification | Valid signed bundle deployed |
| Guardian manual freeze | `cave-ctl apol freeze --reason <r>` | `cave-ctl apol unfreeze` (guardian only) |

### APOL Observability

- Grafana APOL dashboard: action history, class distribution, freeze events, false-positive rate
- APOL KPIs: actions/day, autonomous success rate, escalation rate, mean decision time, false-positive rate (target <5%, ADR-112)
- Quarterly guardian review of APOL action log

---

## Rejected

## ### 3.1 Full Autonomy (no action classes) — Rejected
Unbounded autonomy = unbounded blast-radius. With 73 components and multi-tenant workloads, a single incorrect autonomous action could affect multiple tenants. Action classification provides proportional autonomy.

### 3.2 No Autonomy (recommendation-only) — Rejected
Defeats APOL's purpose. "0 FTE ops" target requires autonomous execution of routine remediation. Class C covers the routine cases. Class D correctly prohibits constitutional changes.

### 3.3 Per-Role Autonomy Limits — Rejected as insufficient
ADR-092 already limits per-role (allowlist/denylist). But cross-role interaction is the gap. A concurrency limit and global freeze triggers address the cross-role coordination problem.

---

## Consequences

## ### Positive
- Clear boundary between autonomous and human-approved actions
- Blast-radius bounded by design
- Global freeze prevents autonomous actions during instability
- Concurrency limits prevent interacting changes
- Guardian has explicit `freeze/unfreeze` control
- Auditable: every Class C action pre-attested in Ledger

### Negative
- Concurrency limits may slow remediation during cascading failures (mitigated: guardian can override)
- False-positive freeze triggers may halt legitimate automation (mitigated: resume conditions are specific and auto-detected)
- Playbook signing adds operational overhead (mitigated: Renovate automates signature refresh)

## Compliance Mapping

## SOC2 CC6.1 (access controls — AI bounded authority). SOC2 CC7.2 (monitoring — AI reasoning traces). ISO A.5.23 (automated system governance). ISO A.8.16 (monitoring activities — continuous AI audit). NIS2 Art.21 (automated security measures — bounded). GDPR Art.22 (automated decision-making — bounded autonomy with human override).
