# ADR-140: Waiver Framework

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Platform Governance

**Related ADRs:** 030 (OPA), 089 (Policy Provenance), 093 (Ledger), 137 (Constitutional Tiering)

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Zero-exception policy enforcement is theoretically ideal but operationally brittle. Real-world scenarios require temporary exceptions: CVE risk acceptance while fix is in progress, deprecated API usage during migration, relaxed egress policy for vendor integration testing, security policy exception for legacy tenant migration.

Without a formal waiver process, exceptions happen informally (Slack message to guardian, undocumented OPA rule override) — creating audit gaps and compliance risk.

---

## Candidates

## | Approach | TTL-bounded waiver framework (chosen) | No waivers (absolute policies) | Permanent waivers | Informal exceptions |
|---|---|---|---|---|
| Edge case handling | ✅ Documented exception path | ❌ Shadow workarounds | ✅ | ⚠️ Tribal knowledge |
| Policy integrity | ✅ Waivers expire, force re-evaluation | ✅ Maximum | ❌ Permanent erosion | ❌ Undocumented drift |
| Compliance trail | ✅ Waiver attestation in Ledger | N/A | ⚠️ Permanent | ❌ |
| Tier-appropriate approval | ✅ A: multi-sig, B: guardian, C: team lead | N/A | ⚠️ Single approver | ❌ |
| Renewal discipline | ✅ Max 90d, renewable once | N/A | ❌ No renewal trigger | ❌ |

## Decision

## | Element | Requirement |
|---|---|
| **Scope** | Specific: tenant + namespace + resource (never platform-wide, never wildcard) |
| **Justification** | Written risk assessment: what policy is waived, why, what compensating controls exist, what is the blast radius |
| **Approval** | Tier A policies: no waiver possible. Tier B: guardian approval. Tier C: team lead. |
| **TTL** | Maximum 90 days. Renewable once with fresh justification (max total: 180 days). |
| **Compensating controls** | Mandatory for security-related waivers. Optional for operational waivers. |
| **Ledger** | `Waiver Granted` attestation: hash of justification, scope, TTL, approver cave_uid |
| **Enforcement** | OPA waiver exception injected via OPAL data source. Scoped to specific tenant+namespace+resource. |
| **Expiry** | Auto-revoked at TTL. `cave-ctl compliance status` shows active waivers. |
| **Review** | Active waivers reviewed quarterly during guardian review. |
| **Escalation** | If waiver renewed 2x (>180 days), mandatory ADR for permanent resolution. |

---

## Rejected

## - **No waiver process (all policies absolute):** Some edge cases legitimately need temporary exceptions (e.g., upstream CVE without fix, performance-critical workload needing host networking). Absolute policies with no exception path create shadow workarounds.
- **Permanent waivers:** Waivers without TTL become permanent policy erosion. Max 90d (renewable once) forces re-evaluation.
- **Team-level waiver for all tiers:** Tier A (immutable) should never be waived. Tier B requires guardian judgment. Only Tier C is team-lead-adjustable.

## Consequences

## ### Positive
- Formal, auditable exception process replaces informal workarounds
- TTL ensures waivers don't become permanent drift
- Ledger attestation satisfies SOC2/ISO 27001 exception documentation requirements
- OPAL distribution ensures waiver is scoped precisely (no broad OPA override)

### Negative
- Waiver process adds governance overhead for legitimate exceptions
- Risk of "waiver fatigue" if too many exceptions requested (mitigated: quarterly review + ADR escalation)

## Compliance Mapping

## SOC2 CC6.1 (policy exception management). ISO A.5.1 (policy governance — exception process). ISO A.5.36 (compliance — documented exceptions with TTL). NIS2 Art.21 (risk management — formal exception process).
