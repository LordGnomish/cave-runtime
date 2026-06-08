# ADR-RUNTIME-ARGO-EVENTS-IN-WORKFLOWS-001: Argo Events lives in cave-workflows, not cave-knative

**Status:** Accepted
**Date:** 2026-05-28
**Deciders:** Platform Engineering
**Tags:** parity, argoproj, argo-events, argo-workflows, knative, crate-ownership

## Context

During the 2026-05-24 Argo close-out wave, the Argo Events parity port
(`argoproj/argo-events v1.9.10` — EventSource / Sensor / EventBus CRDs +
trigger templates + filter reducers) was placed in **cave-knative** as
`src/argo_events.rs`, with its 4 mapped parity subsystems
(`argo-events-eventsource-crd`, `argo-events-sensor-crd`,
`argo-events-trigger-templates`, `argo-events-eventbus-crd`) recorded in
the cave-knative manifest.

This is a mismapping. cave-knative ports **Knative Serving + Eventing**
(`knative/serving` + `knative/eventing`), a serverless serving/eventing
stack from the Knative project — a different upstream organization from
argoproj. Argo Events shares no code, CRD schema, or release cadence with
Knative; it only happened to land in cave-knative because the close-out
ray was touching eventing-shaped code at the time.

Argo Events is, by design, the **event-trigger companion to Argo
Workflows**: its Sensors fire `ArgoWorkflow` triggers that submit/resume
Workflows, and both ship from the same argoproj upstream family. The
natural home is therefore **cave-workflows**, which already ports
`argoproj/argo-workflows v4.0.5`.

## Decision

Move the Argo Events module from cave-knative to cave-workflows:

- `crates/orchestration/cave-knative/src/argo_events.rs` →
  `crates/orchestration/cave-workflows/src/events.rs`
  (module renamed `argo_events` → `events`).
- Migrate the 4 mapped parity subsystems from the cave-knative manifest
  to the cave-workflows manifest, with `note` references updated from
  `src/argo_events.rs` to `src/events.rs`.

## Consequences

- **cave-workflows** now owns the Argo Events parity items via
  `src/events.rs`. Manifest counts: mapped 16 → 20, total 24 → 28;
  `fill_ratio` stays 1.0 `((20 + 0 + 8) / 28)`; `honest_ratio` rises
  16/24 = 0.6667 → 20/28 = 0.7143 (the 4 ported subsystems are honest
  mapped coverage, not scope cuts).
- **cave-knative** scope is now strictly Knative Serving + Eventing.
  Manifest counts: mapped 30 → 26, total 34 → 30; `fill_ratio` stays
  1.0 `((26 + 0 + 4) / 30)`; the previously inflated `honest_ratio` 1.0
  is corrected to 26/30 = 0.8667.
- The module is renamed `argo_events` → `events`; the public types
  (`EventSource`, `EventSourceSpec`, `Sensor`, `EventBus`, the
  `TriggerTemplate` variants, the `matches_filters` / `evaluate_sensor`
  reducers, etc.) are unchanged and reachable as `cave_workflows::events`.
- No cross-crate references existed to `cave_knative::argo_events`, so no
  downstream callers needed updating.
