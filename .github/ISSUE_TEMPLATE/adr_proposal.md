---
name: ADR proposal
about: Propose a new Architecture Decision Record (ADR-RUNTIME-…)
title: "ADR proposal: <decision in one line>"
labels: ["adr", "needs-triage"]
assignees: []
---

<!--
Cave Runtime ADRs live under docs/adr/ as ADR-RUNTIME-<NNN> or
ADR-PORTAL-<NNN>. Numbering is sequential and assigned by the
maintainer at merge time — please leave the number off when you open
this issue. The next free number is shown in docs/adr/README.md.

Before opening: check that an existing ADR doesn't already cover the
question. Amending an ADR is preferred over superseding it when the
underlying decision still holds.
-->

## Context

<!-- What forces are at play? What's the problem? What constraints
     are non-negotiable (Linux 7.1+, PQC-only, AGPL, Charter v2,
     line-by-line parity)? Cite the relevant sections. -->

## Decision

<!-- One paragraph: the concrete decision being proposed. Imperative
     voice. -->

## Rationale

<!-- Why this option over the alternatives? Quantify when possible
     (build time, binary size, audit cost, dependency count). -->

## Alternatives considered

- **Option A — …**: rejected because …
- **Option B — …**: rejected because …

## Consequences

### Positive

-

### Negative / accepted cost

-

### Open questions

-

## Affected crates

<!-- List the crates whose parity.manifest.toml or src/ this decision
     touches. Tag each as: -->

- `cave-…` — paperwork only / src changes / new module / scope_cut

## Charter v2 gate impact

<!-- Will this decision change how any of the 8 gates are evaluated? -->

- Gate 1 (TDD): <!-- no change / new rule / relaxation -->
- Gate 4 (no-stubs): <!-- no change / new pattern allowed -->
- Gate 5 (no-backcompat): <!-- no change / new exception -->
- Other: <!-- … -->

## Migration / rollout

<!-- If accepted, how do existing crates come into compliance? One-shot
     sweep, gradual, opt-in until v0.2? -->

## Links

- Related ADR(s):
- Related issue(s):
- Relevant Charter section(s):
- Upstream evidence (RFC/PEP/RFD links):
