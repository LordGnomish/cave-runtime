<!--
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR-138: Evidence Tiering for Runtime Changes

## Status
Accepted

## Context
Cave Runtime is multi-tenant. A change to the Runtime potentially
affects every tenant on every cluster that ships it. To keep audit
trails honest, every change carries *evidence* — provenance, soak
results, rollback rehearsal, parity tests, signed attestations.

Producing the maximum bundle of evidence for every change (including
docs typos and dev-only tooling) burns operator time without buying any
real safety. Producing none of it makes the audit trail meaningless.

This ADR defines three evidence tiers and a clear rule for which tier a
given change falls into.

## Decision
Three tiers, scoped by *where* the change lands, not who proposed it.

| Tier | Where it applies |
|------|------------------|
| 1. Production-required | Every change that reaches a production cluster |
| 2. Regulated-tenant | Production changes affecting a tenant flagged as regulated (SOC2 / ISO / NIS2 / GDPR scope) |
| 3. Advisory | Dev or staging only, or Runtime-internal changes that cannot affect tenant data paths |

### Evidence obligations per tier

| Evidence | Tier 1 | Tier 2 | Tier 3 |
|----------|--------|--------|--------|
| Compatibility-matrix pass | Required | Required | Optional |
| Signed build provenance (SLSA L3 or equivalent) | Required | Required | Required (CI default) |
| Canary signal capture | Required | Required | Not required |
| Rollback rehearsal | Required | Required | Not required |
| Soak-window completion | Required | Required | Not required |
| Parity test against upstream (where applicable) | Required | Required | Not required |
| Resilience / chaos evidence | Required | Required + tenant-specific scenarios | Not required |
| Data-residency check | Only if a tenant has a residency constraint | Required | Not required |
| Compliance-export bundle (auto-generated) | Not required | Required | Not required |
| Signed-ledger attestation | `upgrade_safe` | `upgrade_safe` + `compliance_verified` | CI audit log only |

### Tier escalation rules
- Any change touching a Tier-A Constitutional artefact (ADR-137) is
  automatically Tier 1 minimum.
- Any change affecting a regulated-flagged tenant is automatically
  Tier 2.
- Any security-sensitive change (cave-auth, cave-policy, cave-vault,
  encryption defaults) is automatically Tier 1.
- The tier is computed by CI from the change's blast radius and the
  affected tenant set — it is not self-declared.

## Rejected Alternatives
- **One bundle for everything.** Prohibitively expensive and trains
  operators to rubber-stamp evidence.
- **No evidence beyond CI green.** CI green does not prove a change is
  safe in production — soak and rollback rehearsal exist precisely
  because pre-merge CI cannot catch all regressions.
- **Per-component custom rules.** Quickly degenerates into an
  unmaintainable matrix; three tiers is the smallest set that
  preserves the distinctions operators care about.

## Consequences

Positive
- Dev velocity is not held hostage by production-grade evidence.
- Regulated tenants get a stronger guarantee than non-regulated.
- Operators have a clear "what does CI ask of me?" rule.

Negative
- Tier-3 changes can miss issues that Tier-1 evidence would have
  caught. Mitigation: the staging soak that runs ahead of any
  promotion is the catch-net.
- Tier computation adds complexity to CI; the source of truth is the
  classifier in `cave-runtime/registry/`, not a wiki.

## Compliance Anchors
External mappings (SOC2 CC7.1–7.2 / CC8.1, ISO 27001 A.5.36 / A.18.2,
NIS2 Art.21) reference this ADR as the policy on tiered evidence
production. Mapping detail lives in `docs/compliance/`.

## Related ADRs
- ADR-137 (Constitutional Tiering — change-control tiers)
- ADR-140 (Waiver Framework — temporary tier downgrades)
