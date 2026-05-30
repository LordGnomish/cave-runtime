<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-karpenter deep-port — continuation ray #3 progress

**Branch:** `claude/cave-karpenter-honest-100-cont3-2026-05-28`
**Base:** `02448962` (continuation ray #2, LOC ratio 0.1061)
**Upstream:** kubernetes-sigs/karpenter **v1.12.1** (sha `ed490e8`, Apache-2.0)
**Scope:** core port, cloud-agnostic — AWS/cloud providers excluded per
ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001.

## Method

Strict TDD, two-commit cadence per module: a `test(...)` commit carrying the
RED proof (compile-fail / assertion-fail output pasted in the body), then a
`feat(...)` commit carrying the GREEN proof. No single commit mixes test +
impl. `git log --oneline` is the audit trail.

LOC honest_ratio is measured as **total `src/**.rs` line count / upstream-core
non-test LOC (34,772)** — the same basis rays #1/#2 used, so the delta is
comparable.

## Wave 2 — apis/v1 portable surface, completed ✅

cont2 finished the `pkg/scheduling` package and began apis/v1 (budgets,
labels). This ray closes the rest of the cont2 "next" queue: the two
validation files, the duration type, and — by porting a cron engine — the
cron-schedule budget window that cont2 had to scope-cut.

| Cycle | Module | Upstream file | RED SHA | GREEN SHA | Tests |
|-------|--------|---------------|---------|-----------|-------|
| 5 | `validation`           | `pkg/apis/v1/nodeclaim_validation.go` | `45459bde` | `3ead9f03` | 28 |
| 6 | `nodepool_validation`  | `pkg/apis/v1/nodepool_validation.go`  | `a97c9762` | `ddd7af02` | 11 |
| 7 | `duration`             | `pkg/apis/v1/duration.go`             | `d357d9c1` | `1016f94f` | 15 |
| 8 | `cron`                 | robfig/cron/v3 (Budget.IsActive dep)  | `6711e54b` | `20ba4b85` | 12 |
| 9 | `budgets::IsActive`    | `pkg/apis/v1/nodepool.go` (IsActive)  | `b592d3ea` | `fb67868a` | 9  |

**Cycle 5 — nodeclaim_validation.** `SUPPORTED_NODE_SELECTOR_OPS` /
`SUPPORTED_RESERVED_RESOURCES` / `SUPPORTED_EVICTION_SIGNALS`, a `multierr`-
style `ValidationError` aggregator, the two k8s.io/apimachinery helpers
`is_qualified_name` / `is_valid_label_value` (+ `IsDNS1123Subdomain` prefix
check) hand-rolled to keep the crate regex-free, `validate_requirement`
(operator support, restricted label, well-known value gating, qualified-name,
label-value, In/MinValues, Gt/Lt/Gte/Lte single-positive-int), and
`validate_taints` (empty-key, qualified-name on key+value, duplicate
key/effect across `taints` + `startupTaints`). The controller-runtime log line
in `validateWellKnownValues` is dropped (invalid-but-tolerated values are not
errors — silent proceed, behaviour preserved).

**Cycle 6 — nodepool_validation.** `validate_labels` (nodepool-key
reservation, qualified-name key, label-value, IsRestrictedLabel),
`validate_requirements_node_pool_key_does_not_exist`, and `runtime_validate`
(the `RuntimeValidate` fan-out). Added `validation::validate_requirements` (the
nodeclaim per-list aggregator) and made `ValidationError::{append, absorb,
into_result}` `pub(crate)` so the sibling fan-out module can build/merge them.

**Cycle 7 — duration.** Faithful port of Go stdlib `time.ParseDuration`
(leadingInt/leadingFraction, unitMap incl. `us`/`µs`/`μs`, sign, i128
accumulation with i64-ns overflow detection) and `time.Duration.String`
(fmtFrac/fmtInt right-to-left build, sub-second unit selection, h/m/s
compound, negative), wrapped by `NillableDuration` with the `"Never"` sentinel
and serde (de)serialize that preserves the raw form to avoid GitOps drift.

**Cycle 8 — cron.** The robfig/cron/v3 subset `Budget.IsActive` delegates to:
`parse_standard` (5-field standard cron, TZ= prefix, getField/getRange/getBits,
JAN-DEC / SUN-SAT names, steps, lists, ranges, star-bit) and
`CronSchedule::next` (faithful `SpecSchedule.Next` month→day→hour→minute→second
descent with truncate-on-bump + WRAP, dom/dow AND-when-star / OR-otherwise).
Self-contained civil-time layer (Hinnant `days_from_civil`/`civil_from_days`,
weekday, carry arithmetic) — no chrono/regex pulled. NOTICE gains robfig/cron
(MIT).

**Cycle 9 — Budget.IsActive.** Clock-threaded `budget_is_active_at` over the
cycle-8 cron + cycle-7 duration: `checkpoint = now - duration`,
`next = schedule.next(checkpoint)`, active iff `next <= now`. Clock-aware
allowance twins (`budget_allowed_disruptions_at` etc.) mirror Go's
`clock.Clock` threading. The cont2 no-clock helpers are untouched, so their 14
tests stay green. `BudgetError` gains `InvalidCron` / `InvalidDuration`.

## Wave 3 — pkg/utils (start) ✅

| Cycle | Module | Upstream file | RED SHA | GREEN SHA | Tests |
|-------|--------|---------------|---------|-----------|-------|
| 10 | `pretty`           | `pkg/utils/pretty/pretty.go`      | `c5cb52d0` | `6591e805` | 6  |
| 11 | `pod`              | `pkg/utils/pod/scheduling.go`     | (RED)      | `a270c599` | 18 |

**Cycle 10 — pretty.** `concise` (compact JSON), `slice`/`map` truncation with
the `and N other(s)` tail, `taint` pretty-print, `to_snake_case` (both upstream
regex passes — `(.)([A-Z][a-z]+)` then `([a-z0-9])([A-Z])` — reproduced by hand
so the crate stays regex-free), and `sentence`.

**Cycle 11 — pod.** A focused `corev1.Pod` reduction plus the pod-state
predicate set the provisioning + disruption controllers gate on:
`is_terminal`/`is_terminating`/`is_active`, `is_stuck_terminating` (clock),
ownership GVK matchers (StatefulSet/DaemonSet/Node), `is_provisionable`,
`is_reschedulable` (with the StatefulSet-terminating exception),
`is_pod_eligible_for_forced_eviction`, `is_do_not_disrupt_active` (true /
duration-window / invalid / absent / no-start-time fail-safe, clock),
`is_disruptable`, `is_drainable`, `is_waiting_eviction`,
`tolerates_disrupted_no_schedule_taint`, and the anti-affinity / DRA helpers.
The `events.Recorder` emission inside `IsDoNotDisruptActive` is dropped
(non-behavioral). Reuses `scheduling::taints::Toleration` and
`duration::parse_duration`.

## Metrics

| Metric | Ray #2 end | Ray #3 end | Δ |
|--------|-----------|-----------|---|
| cave `src` total LOC | 3,689 | 5,555 | +1,866 |
| LOC honest_ratio (/34,772) | 0.1061 | **0.1597** | +0.0536 |
| cave-karpenter active tests | 205 | **304** | +99 |
| TDD cycles (this ray) | — | **7** | — |
| manifest `mapped_count` | 19 | 19 | 0 (anti-inflation) |

`pkg/apis/v1` is now substantially ported: CRD model (cont1/2), requirements,
taints, labels, budget math, **both validation files**, **duration**, and
**the cron-schedule budget window**. The manifest gains descriptive `[[files]]`
rows for the new modules; the scalar `mapped_count` / `honest_ratio`
(19 / 0.8636) stay pinned to the v1.4.0 subsystem baseline (the index is
hook-regenerated from the scalars, not the `[[files]]` count).

## Remaining work (for continuation ray #4)

**apis/v1 tail:**
- NodePool `Hash()` — drift-detection hash over the CRD; needs a
  hashstructure-equivalent (mitchellh/hashstructure with `hash:"ignore"`
  field handling). Deferred — a self-contained effort.
- NodePool/NodeClaim defaults are upstream no-ops (nothing to port).

**Wave 3 — utils (continued):** `pkg/utils/pretty` ✅ + `pkg/utils/pod`
(scheduling predicates) ✅ done this ray. Remaining: `pkg/utils/node`,
`pkg/utils/nodeclaim`, `pkg/utils/nodepool`, `pkg/utils/pdb`,
`pkg/utils/ringbuffer`, `pkg/utils/atomic`, `pkg/utils/env`. Pure helpers are
directly TDD-able; controller-runtime client paths are scope-cut.

**Wave 4 — controllers (large):** nodepool (555), disruption (3,092),
provisioning sim (4,539), cluster state (3,017).

**Merge gate:** LOC honest_ratio ≥ 0.95 before any merge to main. At 0.1479
this branch is **held, not merged** — honest in-progress state, reviewed
together with rays #1 (`dcd82fb1`) and #2 (`02448962`).
