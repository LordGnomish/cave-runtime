# ADR-124: Crossplane MRAP — ManagedResourceActivationPolicy

**Status:** Accepted

**Scope:** Azure, Runtime, Universal

**Category:** Platform

**Related ADRs:** 067

## Context

Crossplane providers install hundreds of CRDs (Azure provider alone: 400+). Most are unused. CRD count impacts API server memory and discovery latency.

## Candidates

| Approach | MRAP (targeted CRDs) | Wildcard (all CRDs) | No CRDs (manual) |
|---|---|---|---|
| API server memory | ✅ ~60% reduction (only used types) | ❌ All CRDs loaded | N/A |
| CRD count | ✅ ~20-30 per provider | ❌ 400+ per provider | N/A |
| Provider upgrade | ✅ Only activated types updated | ❌ All types updated | N/A |

## Decision

ManagedResourceActivationPolicy per profile: only used MR types installed as CRDs. Dev profile activates fewer types than prod. Default wildcard MRAP replaced with targeted policy. `cave-ctl doctor` validates only used MR types active.

## Rejected

- **Wildcard MRAP (all CRDs):** 400+ CRDs per Azure provider. API server memory 40% higher. Discovery slower. Unnecessary CRDs create attack surface.
- **No MRAP (pre-v2 behavior):** All provider CRDs always installed. Same issues as wildcard.

## Consequences

**Positive:**
- API server memory reduced ~40% (measured on dev profile).
- Faster API discovery.
- Smaller attack surface (unused CRDs not available).
- Per-profile activation — dev doesn't carry prod's CRD weight.

**Negative:**
- MRAP must be updated when new XR types are added (manual step).
- Provider upgrades may introduce new required CRDs that MRAP blocks — cave-ctl upgrade check validates.

## Compliance Mapping

SOC2 CC6.1 (minimal attack surface). ISO A.8.8 (reduce unnecessary components).
