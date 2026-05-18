<!--
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR-137: Constitutional Tiering of Governed Configuration

## Status
Accepted

## Context
Cave Runtime ships a number of artefacts that are *normative* — changing
them changes the meaning of the cluster. Examples: the identity-root
configuration that `cave-auth` boots from, the policy bundles that
`cave-policy` evaluates, the operational thresholds that the
self-improving daemon may touch.

If all of these are gated behind the same heavy approval flow, change
velocity collapses for routine tuning. If none of them are gated, an
operator (or the autonomous agent in ADR-SELF-IMPROVE-001) can quietly
re-write the constitution.

This ADR establishes a tiered governance taxonomy so that each governed
artefact carries a clear "how do you change me" contract.

## Decision
Three tiers, each with a specific change process and an audit obligation
in the operator's signed-event log.

| Tier | Name | Change process | Approval | Audit |
|------|------|----------------|----------|-------|
| A | Immutable Constitutional | PR + multi-party signed approval + hardware-backed signing | Multi-party (2-of-N) | `constitutional_change` event |
| B | Protected Evolvable | PR + single named approver | Named owner | `protected_change` event |
| C | Governed Operational | Normal review | Team lead | Standard audit event |

The Runtime obligation is to make tier the **source-of-truth field** on
the governed artefact itself — not a wiki note. The classification
machinery and the approval flow live in `cave-runtime/registry/` and are
enforced by `cave-policy` admission, not by social contract.

### Tier assignment (Runtime defaults)
The Runtime ships with the following default classification. Operators
may move artefacts to a higher tier but may never lower one.

| Tier A | Tier B | Tier C |
|--------|--------|--------|
| Identity root (`cave-auth` realm config, root CAs) | Core policy bundles | Soak-window durations |
| Signed-ledger trust root | Backup / restore RPO/RTO contracts | Rollout thresholds |
| Policy-bundle signing keys + trust chain | Tenant SLA definitions | Resource-budget limits |
| Compatibility-matrix schema | RBAC mapping templates | Alert routing rules |
| Break-glass access list | Egress allow-list policy | Dashboard layouts |

### Interaction with the self-improving agent
ADR-SELF-IMPROVE-001 grants the resident agent a bounded write surface.
Constitutional tiering is the gate on that surface:

- The agent **must not** modify Tier A.
- The agent **may propose** Tier B changes (prepare a PR; never merge).
- The agent **may apply** Tier C changes inside its signed-playbook
  scope, with attestation.

## Rejected Alternatives
- **No tiering (one global approval flow).** Approver fatigue;
  routine changes pile up behind security-critical changes.
- **Two tiers (critical / non-critical).** "Protected but evolvable" is
  a distinct category; collapsing it into either neighbour produces
  bad incentives.
- **Advisory tiering (labels without enforcement).** Cosmetic. A tier
  with no enforcement path will be ignored at the first deadline.

## Consequences

Positive
- Routine operational tuning does not block on multi-party approval.
- The autonomous agent has a small, machine-readable contract for what
  it may touch.
- Security-critical artefacts retain their highest-friction approval.

Negative
- Mis-classification risk. The Runtime mitigates by treating any
  artefact involved in a security incident as escalated to the next
  tier, pending re-review.
- Slightly more governance surface to document and explain to a new
  operator.

## Compliance Anchors
This ADR is the hook used by external mappings for tiered change-control
(SOC2 CC6.1 / CC8.1, ISO 27001 A.5.1 / A.5.4, NIS2 Art.21). Mapping
detail lives in `docs/compliance/`.

## Related ADRs
- ADR-SELF-IMPROVE-001 (agent's write surface honours these tiers)
- ADR-138 (Evidence Tiering — how attestations from each tier are kept)
- ADR-140 (Waiver Framework — temporary exceptions, never to Tier A)
