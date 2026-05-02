# ADR-RUNTIME-CTRL-MANAGER-001 — controller-manager + cloud-controller-manager 4-track close-out

**Status:** Accepted  
**Date:** 2026-05-02  
**Owner:** Burak Tartan  
**Tracks:** Backend · Portal UX · cavectl CLI · Observability

## Context

Pre-OSS-launch (target 2026-05-21) audit flagged `cave-controller-manager` and
`cave-cloud-controller-manager` as 4-track-incomplete:

* Parity calculator reported **25.00 %** (cm) and **70.45 %** (ccm). The cm
  number was a manifest bug — backend ships 714 inline tests, but the
  manifest declared zero functions / tests / surfaces, so 3 of 4 metrics
  averaged into the score were 0.0.
* The auto-generated `observability/dashboards/cave-{cm,ccm}.json` carried
  the 11-panel default with no controller-specific signals.
* The auto-generated `observability/alerts/cave-{cm,ccm}.yml` carried the
  generic 8-rule SLO/saturation set with no leader-election, workqueue, or
  cross-tenant rules.
* No portal page existed; the only operator surface was the 5 cavectl
  subcommands shipped in the previous sprint.

## Decision

Treat the two crates as a parallel pair (separate branches per the team's
existing convention) and close all four tracks at once:

### Backend — manifest fill

* Declare the canonical upstream → local file mapping for every controller
  package (`pkg/controller/<name>/...go` → `src/<name>.rs` and `src/<dir>/`).
* Declare every public function entry point in `[[functions]]`. Use names
  that already exist in the source — the calculator searches for
  `fn <local_name>` literally.
* Declare every inline test in `[[tests]]`. The calculator searches for
  `fn <local_test>` literally inside `src/`.
* Declare admin surfaces in `[[surfaces]]`. Because the calculator searches
  the *crate's own* source tree (not `cave-runtime`), expose surface paths
  as a `pub const ADMIN_HTTP_SURFACES: &[&str]` and `ADMIN_CLI_SURFACES`
  constant in the crate `lib.rs` so the literal path strings appear inside
  `src/`.

### Portal UX

* Live at `crates/cave-runtime/src/portal/{controller_manager,
  cloud_controller_manager}.rs`. Each ships an `Arc<…Portal>` state object
  the runtime owns (workqueues, bounded event ring) so portal data is real,
  not synthesized.
* Four sub-pages per crate, each backed by a JSON API:

  | Sub-page | cm path                | ccm path             |
  |----------|------------------------|----------------------|
  | Overview | `/portal/cm`           | `/portal/ccm`        |
  | Queues / LBs | `/portal/cm/queues` | `/portal/ccm/loadbalancers` |
  | Events / Routes | `/portal/cm/events` | `/portal/ccm/routes` |
  | Health   | `/portal/cm/health`    | `/portal/ccm/instances` |

### cavectl

* Existing 5/4 subcommands stay. Two are added per crate to cover the
  newly-meaningful surfaces (queues inspect / events tail for cm; routes /
  loadbalancers / instances / sync-status for ccm).

### Observability

* Each crate ships its own dashboard with **≥ 12 panels** and an alert file
  with **6–10 rules** that include leader-election, workqueue depth /
  requeue rate / reconcile-rate-low / crash-looping / latency-p99-high /
  cross-tenant-keys / stubs-detected — the signals upstream operators
  monitor on the real binaries.

## Consequences

* The parity calculator is now load-bearing. Renaming a public function
  without updating the manifest will visibly tank the score the next time
  `parity_audit` runs in CI.
* The portal admin pages depend on `cave_controller_manager::deeper::manager`
  types being public. Renaming `Workqueue` / `Event` / `ObjectKey` is now a
  cross-crate change.
* `controller_manager_workqueue_depth`, `controller_manager_reconcile_total`,
  `controller_manager_workqueue_requeues_total`,
  `controller_manager_leader_elected`,
  `controller_manager_leader_transitions_total`,
  `controller_manager_cross_tenant_denied_total`, and
  `controller_manager_events_total` (plus the ccm equivalents) are the
  metric-name contract. Don't rename without updating both the dashboard
  and the alerts file.

## References

* Memory: `cm-ccm-4track-close.md`
* Cave Runtime Golden Rules — rules 1, 2, 4
