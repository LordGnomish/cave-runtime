<!--
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR-140: Waiver Framework for Temporary Policy Exceptions

## Status
Accepted

## Context
Cave Runtime enforces policy through `cave-policy` and the
constitutional tiers of ADR-137. Real operations occasionally need
*temporary* relief from a rule — for example: an unfixed upstream CVE,
a tenant migration that briefly needs a relaxed egress rule, a workload
needing host networking until a feature lands.

If there is no formal way to grant an exception, operators reach for
informal workarounds (chat-approved policy edits, hand-rolled bypasses
in admission). Those workarounds are invisible to audit and tend to
become permanent. This ADR defines the only sanctioned way to grant a
temporary exception.

## Decision
The Runtime exposes a waiver primitive with the following non-negotiable
properties.

| Property | Rule |
|----------|------|
| Scope | A waiver names a specific tenant, namespace and resource selector. Cluster-wide or wildcard waivers are forbidden. |
| Justification | A written risk assessment in the PR that creates the waiver: what is being waived, why, what compensating controls are in place, what is the blast radius. |
| Approval | Tier A artefacts (ADR-137) cannot be waived. Tier B requires the named owner. Tier C requires team-lead approval. |
| TTL | Maximum 90 days. Renewable once with fresh justification. Hard cap 180 days. |
| Compensating control | Mandatory for security-relevant waivers; optional for operational ones. |
| Audit | A `waiver_granted` event is written to the signed-event log: hash of justification, scope, TTL, approver identity. |
| Enforcement | The waiver is materialised as a scoped policy exception in `cave-policy`; nothing outside that path may grant a bypass. |
| Expiry | Automatic at TTL. `cavectl compliance waivers` lists active waivers and how long they have left. |
| Escalation | A waiver renewed twice (total reach >180 days) triggers a mandatory ADR proposing a permanent resolution. |

## Rejected Alternatives
- **No waiver path (absolute policies).** Operators will route around
  the rule via undocumented bypasses; this is the worst outcome for
  audit.
- **Permanent waivers.** Without TTL, a "temporary" exception becomes a
  silent policy change. The 90-day cap forces re-evaluation.
- **Single approver for all tiers.** Treats a Tier-A exception (which
  the Runtime forbids) the same as a Tier-C operational tweak; loses
  the safety property of ADR-137.

## Consequences

Positive
- Exceptions are visible to audit by construction — they live in the
  same signed-event log as constitutional changes.
- The Runtime keeps a single source of truth for "what is currently
  not being enforced", instead of distributing that knowledge across
  ad-hoc admission edits.
- A waiver that needs to be renewed twice is a signal that the
  underlying policy needs to change — the framework surfaces that
  signal instead of hiding it.

Negative
- A waiver carries real paperwork weight; operators may try to avoid
  the process for genuinely urgent cases. The Runtime mitigates by
  making the happy path (Tier C, named compensating control)
  lightweight.
- "Waiver fatigue" is possible if many exceptions accumulate; the
  quarterly review is the relief valve.

## Compliance Anchors
External mappings (SOC2 CC6.1, ISO 27001 A.5.1 / A.5.36, NIS2 Art.21)
reference this ADR as the documented exception process. Mapping detail
lives in `docs/compliance/`.

## Related ADRs
- ADR-137 (Constitutional Tiering — which tiers may be waived at all)
- ADR-138 (Evidence Tiering — a waiver does not reduce the evidence
  required for the change it accompanies)
