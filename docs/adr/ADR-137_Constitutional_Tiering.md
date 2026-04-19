# ADR-137: Constitutional Tiering

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Platform Governance

**Related ADRs:** 093 (Sovereign Ledger), 112 (APOL), 133 (Compatibility Matrix), 136 (Bounded Autonomy)

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE designates certain artifacts as "constitutional" — protected from automation and requiring multi-sig governance. However, grouping all governed artifacts at the same protection level creates:

- **Guardian bottleneck:** Every soak window change, FinOps threshold adjustment, or alert routing update requires 2-of-3 guardian multi-sig + hardware key. Guardians become approval bottlenecks for routine operational tuning.
- **Change velocity reduction:** Platform evolution slows because trivial config changes get the same governance overhead as identity root modifications.
- **APOL ineffectiveness:** APOL downgrades to recommendation-only mode whenever it needs to adjust governed artifacts, even if those artifacts are operational parameters (not security-critical).

The solution is tiered protection: highest protection for identity/trust roots, moderate for evolvable policies, lightest for operational parameters.

---

## Candidates

## | Approach | Three-tier constitutional (chosen) | No tiering (guardian for all) | Binary (critical/non-critical) | Tagging without enforcement |
|---|---|---|---|---|
| Guardian bottleneck | ✅ Tier C is team-lead approval | ❌ All changes need guardian | ⚠️ Only 2 tiers | ✅ |
| Change velocity | ✅ Proportional to impact | ❌ Slow for all changes | ⚠️ | ✅ Fastest |
| Immutability guarantee | ✅ Tier A is multi-sig | ❌ Single approver bypass | ⚠️ | ❌ Advisory only |
| Granularity | ✅ Three meaningful levels | ❌ Single class | ❌ Too coarse | ⚠️ |
| Enforcement | ✅ Constitutional Registry + Ledger | ✅ | ✅ | ❌ Cosmetic |

## Decision

## Three constitutional tiers with proportional governance:

| Tier | Name | Change Process | Approval | Ledger |
|---|---|---|---|---|
| **A** | Immutable Constitutional | PR + 2-of-3 guardian multi-sig + hardware-backed signing key | Multi-sig | `Constitutional Change` attestation |
| **B** | Protected Evolvable | PR + single guardian approval + standard signing | Guardian | `Protected Change` attestation |
| **C** | Governed Operational | PR + team lead approval | Team lead | Standard CI audit |

### Tier Assignment

| Tier A (Immutable) | Tier B (Protected) | Tier C (Operational) |
|---|---|---|
| Identity root config (Keycloak master realm, Okta org) | Core OPA policy bundles | Soak window durations |
| Sovereign Ledger trust root + WORM config | Crossplane Composition interfaces | Rollout thresholds |
| Policy bundle signing keys + trust chain | Backup/recovery contracts (RPO/RTO) | FinOps budget thresholds |
| Compatibility matrix **schema** (not values) | Tenant SLA definitions | APOL concurrency limits |
| XRD schema contracts | RBAC mapping templates | Alert routing rules |
| Break-glass Kit access list | Egress quarantine policy | Dashboard configurations |
| | Parity test contracts (ADR-135) | Chaos experiment parameters |

### APOL Interaction

- APOL may never modify Tier A artifacts (Class D action, ADR-136)
- APOL may propose Tier B changes (Class B: prepare PR) but not execute
- APOL may execute Tier C changes autonomously (Class C) within signed playbook bounds

---

## Rejected

## - **No tiering (all changes require guardian approval):** Guardian bottleneck for routine operational changes. Slows platform iteration. Guardians fatigue from reviewing low-impact changes.
- **Two tiers only (critical/non-critical):** Too coarse. "Protected but evolvable" (Tier B) is a distinct category — more impactful than operational tuning (Tier C) but not immutable like identity root (Tier A).
- **Tiering without enforcement (advisory only):** Tier labels without corresponding approval workflow are cosmetic. Constitutional registry + Ledger attestation makes tiering enforceable.

## Consequences

## ### Positive
- Guardian load reduced by ~60% (Tier C no longer requires guardian approval)
- APOL can autonomously tune operational parameters (Tier C) without freeze
- Security-critical artifacts (Tier A) retain maximum protection
- Platform evolution accelerates for non-security changes

### Negative
- Tier misclassification risk (operational artifact incorrectly placed in Tier C that should be Tier B)
- More complex governance documentation (3 tiers vs 1)

### Mitigations
- Tier assignment reviewed quarterly during guardian review
- Any artifact involved in a security incident automatically escalated to next tier pending review
- `docs/constitutional-registry.yaml` tracks tier assignment with justification

## Compliance Mapping

## SOC2 CC6.1 (governance — tiered change control). SOC2 CC8.1 (change management — approval levels by impact). ISO A.5.1 (policy governance — tiered policy management). ISO A.5.4 (management responsibilities — guardian multi-sig for Tier A). NIS2 Art.21 (governance — structured change control).
