# ADR-138: Evidence Tiering

**Status:** Accepted

**Category:** Platform Governance

**Related ADRs:** 093 (Sovereign Ledger), 100 (Resilience Attestation), 101 (SLSA L3), 132 (Version Channels), 135 (Parity)

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's governance model demands extensive evidence: compatibility proof, parity proof, signed provenance, Ledger attestations, soak evidence, rollback rehearsal, chaos evidence, policy verification. This proof burden is justified for production deployments affecting regulated tenants but creates disproportionate overhead for dev/staging changes and non-critical updates.

Risk: platform team spends more time producing evidence than building platform.

---

## Candidates

## | Approach | Three-tier evidence (chosen) | Tier 1 everywhere (max evidence) | No evidence (CI pass only) | Per-component custom rules |
|---|---|---|---|---|
| Evidence cost | ✅ Proportional to risk | ❌ Expensive for all changes | ✅ Cheapest | ⚠️ Variable |
| Audit-readiness | ✅ Tier 2 export covers regulated | ✅ Always ready | ❌ Insufficient | ⚠️ Inconsistent |
| Velocity for advisory changes | ✅ Tier 3 is CI + review | ❌ Slowed by unnecessary evidence | ✅ | ⚠️ |
| Maintainability | ✅ Three tiers to understand | ✅ One standard | ✅ | ❌ N rules to maintain |
| Compliance gap risk | ✅ Tier 2 for regulated | ✅ Covers everything | ❌ Major gap | ⚠️ |

## Decision

## Three evidence tiers matched to deployment risk:

| Tier | Scope | When Applied |
|---|---|---|
| **1 — Prod Required** | Every prod promotion | Default for all prod changes |
| **2 — Regulated Tenant** | Tenants with compliance requirements (SOC2, ISO 27001, NIS2, GDPR) | Triggered by tenant classification or explicit compliance flag |
| **3 — Advisory** | Dev/staging, non-critical tooling changes | Default for non-prod |

### Evidence Requirements per Tier

| Evidence Type | Tier 1 (Prod) | Tier 2 (Regulated) | Tier 3 (Advisory) |
|---|---|---|---|
| Compatibility matrix pass | Required | Required | Optional |
| SLSA L3 provenance + cosign signature | Required | Required | Required (CI default) |
| Canary evidence (Argo Rollouts metrics) | Required | Required | Not required |
| Rollback rehearsal | Required | Required | Not required |
| Soak window completion | Required | Required | Not required |
| Parity test (ADR-135) | Required | Required | Not required |
| Chaos resilience proof | Required | Required + tenant-specific scenarios | Not required |
| Data residency verification | Only if tenant has EU constraint | Required | Not required |
| Compliance export package | Not required | Required (auto-generated) | Not required |
| Ledger attestation | `Upgrade Safe` | `Upgrade Safe` + `Compliance Verified` | CI audit log only |

### Tier Escalation

- Change touches Tier A constitutional artifact → always Tier 1 evidence minimum
- Change affects tenant with compliance flag → Tier 2 evidence mandatory
- Security-related change (OPA policy, identity, encryption) → always Tier 1
- CI automatically determines tier based on change scope + affected namespaces

---

## Rejected

## - **Same evidence for all changes (Tier 1 everywhere):** Prohibitively expensive. Every change requiring canary + chaos + soak + parity + Ledger would slow delivery to a crawl. Evidence must be proportional to risk.
- **No evidence requirements (trust CI pass):** CI pass alone doesn't prove production readiness. Canary analysis, soak windows, and parity tests catch issues that unit/integration tests miss.
- **Per-component evidence (custom per ADR):** Too many custom evidence rules to maintain. Three tiers provide sufficient granularity while remaining manageable.

## Consequences

## ### Positive
- Dev/staging velocity not bottlenecked by prod-grade evidence requirements
- Platform team effort focused on high-risk evidence
- Regulated tenants get stronger evidence guarantees than non-regulated
- Clear expectation setting: tenants know their evidence tier

### Negative
- Tier 3 changes may miss issues that Tier 1 evidence would catch (accepted: that's what staging soak catches)
- Tier assignment logic adds CI pipeline complexity

## Compliance Mapping

## SOC2 CC7.1-7.2 (evidence requirements scaled by risk). SOC2 CC8.1 (change evidence proportional to impact). ISO A.5.36 (compliance — evidence production). ISO A.18.2 (information security reviews — evidence standards). NIS2 Art.21 (compliance evidence — tiered requirements).
