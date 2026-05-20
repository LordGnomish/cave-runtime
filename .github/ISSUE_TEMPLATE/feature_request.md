---
name: Feature request
about: Propose a new capability or improvement
title: "feat(<crate>): <one-line summary>"
labels: ["enhancement", "needs-triage"]
assignees: []
---

<!--
Cave Runtime follows line-by-line upstream parity. Before proposing
a "new" feature, please verify it isn't already an unmapped upstream
subsystem — in that case the `parity_gap` template is a better fit.

Genuine new features (cross-module verbs, sovereign-only capabilities,
operator ergonomics) are very welcome. This template is for those.
-->

## Use case

<!-- What problem does this solve? Who is affected? Be specific:
       * "Operator needs to drain a tenant across mesh+gateway+apiserver in one shot."
       * "Tenant admin wants a one-glance compliance summary."
     Avoid generic statements like "improve observability".
-->

## Proposed capability

<!-- Sketch what the user-visible surface would look like. CLI flag,
     portal page, HTTP endpoint, metric, alert — concrete examples. -->

## Why this belongs in Cave Runtime (not an upstream)

<!-- Cave Runtime adds cross-module verbs that no upstream CLI can
     express because Cave owns all of them under one roof. If this
     feature is achievable in a single upstream alone, consider
     contributing it upstream first. -->

## Affected tracks

A capability that affects users must land in all four tracks in the
same PR. Please indicate which are touched:

- [ ] **Backend** — crate(s): `cave-<name>` (…)
- [ ] **Portal** — pages: `/admin/...`
- [ ] **cavectl** — subcommands: `cave <verb> ...`
- [ ] **Observability** — metrics / alerts / dashboards added or updated

If the proposal is `infra_only`, please mark that and explain why no
user-visible track is needed.

## Alternatives considered

<!-- What did you consider and reject? Why? Cave's "no backwards-compat,
     no shims" stance often makes "obvious" alternatives wrong. -->

## ADR impact

<!-- Does this change an ADR-bound invariant (charter, golden rules,
     multi-tenancy, PQC, self-improvement)? If yes, an ADR PR is
     required before implementation. -->

- [ ] No ADR change required
- [ ] Touches existing ADR: <ADR-XXX-...>
- [ ] Needs a new ADR (will submit alongside this issue)

## Additional context

<!-- Mocks, sketches, links to upstream discussions, prior art, etc. -->
