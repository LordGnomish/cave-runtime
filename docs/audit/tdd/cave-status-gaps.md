# TDD coverage gap audit — cave-status

- **Crate:** `cave-status` (theme: observability)
- **Upstream:** louislam/uptime-kuma @ `v1.23.0` (JavaScript)
- **Upstream test-symbol count:** 21 (across 7 test files)
- **cave test-fn count:** 11 (unit tests in live `src/`)

## Context

Upstream Uptime Kuma's test suite is overwhelmingly **Playwright E2E browser
tests** (status-page UI, monitor forms, incident-history UI, friendly-name
resolution, RSS feed) plus a Jest **i18n translation-key consistency** check.
`cave-status` is a thin **backend status-page aggregator**: it models
components/pages/incidents and computes a worst-status rollup. It is **not** the
monitor probe engine, the Vue front-end, or the i18n bundle, so most upstream
symbols are legitimately out of scope.

> Note: `src/store.rs` is an **orphan** — it is not declared in `lib.rs` and
> references types (`Component`, `ComponentGroup`, `Maintenance`,
> `OverallPageStatus`, `IncidentStatus`, `MaintenanceStatus`) that do not exist
> in `models.rs`, so it does not compile and is excluded from the build. The
> live crate is `models.rs` + `engine.rs` + `routes.rs` only. Active/resolved
> incident filtering is therefore **not** a live behavior.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Non-operational status precedence (degraded < partial < maintenance < major) | (engine analogue of status rollup; e2e status-page status badge) | yes — `engine::compute_overall_status` / `status_rank` | partial (only all-op, one-major, empty) | portable-coverage | `test_overall_status_precedence_ordering` |
| Resolved incident serde (resolved_at = Some) | incident-history `resolved incidents appear in past incidents section` | yes — `models::StatusIncident.resolved_at: Option<DateTime>` | no (only `None` branch tested) | portable-coverage | `test_status_incident_resolved_roundtrip` |
| Health endpoint serves status JSON | example.spec `dashboard` (page responds) | yes — `routes::create_router` / `health` | no | portable-coverage | `test_health_route_returns_ok_json` |
| count_by_status with maintenance bucket | (status-page badge counts) | yes — `engine::count_by_status` | partial (no UnderMaintenance bucket asserted) | portable-coverage | `test_count_by_status_includes_maintenance` |
| has_issues with only maintenance component | incident/maintenance banner shown | yes — `engine::has_issues` | partial (degraded only) | portable-coverage | `test_has_issues_true_for_maintenance` |
| i18n translation-key consistency | check-translations `should not have missing translation keys` / `placeholder parameters` | no (no i18n bundle in crate) | n/a | scope-cut | — (UI/i18n bundle, not backend scope) |
| TLD/domain-expiry notification | domain-expiry-notification `TLD enabled for new monitor` | no (monitor probe engine, not status-page) | n/a | scope-cut | — (monitor-engine concern, out of cave-status scope) |
| Monitor setup / DB reset between tests | example.spec `set up monitor` / `database is reset` | no (no monitor CRUD / DB fixture) | n/a | scope-cut | — (E2E harness + monitor engine) |
| Friendly-name resolution (hostname/URL/custom/default) | fridendly-name.spec (4 specs) | no (no monitor naming logic) | n/a | scope-cut | — (monitor-engine naming, not status aggregation) |
| Incident-history UI (hidden/pinned/pagination) | incident-history.spec (pinned-at-top, pagination) | partial (StatusIncident model only; no pin/order/paginate) | no | missing-impl | — (would need pin/order/pagination fields + logic) |
| Monitor-form conditions / response settings | monitor-form.spec (4 specs) | no (no monitor form/condition model) | n/a | scope-cut | — (monitor-engine UI form) |
| Status-page create/edit CRUD | status-page.spec `create and edit` | partial (StatusPage model; no persistence/CRUD) | no | missing-impl | — (needs store layer; orphan store.rs unbuildable) |
| RSS feed escapes malicious monitor names (XSS) | status-page.spec `RSS feed escapes malicious monitor names` | no (no RSS rendering) | n/a | scope-cut | — (RSS/HTML rendering, front-end output concern) |

## Recommended TDD fills (portable-coverage first)

These exercise behavior the **live** crate already implements but does not yet
test. All are cheap, pure-Rust unit tests with no new dependencies.

1. **`test_overall_status_precedence_ordering`** — exercises
   `engine::compute_overall_status`. Build a page mixing
   `DegradedPerformance`, `PartialOutage`, and `UnderMaintenance` (no
   `MajorOutage`) and assert the result is `UnderMaintenance` (rank 3), then add
   a `MajorOutage` and assert it overrides to `MajorOutage`. The existing tests
   only cover all-operational, single-major-outage, and empty — the interior
   rank ordering (1<2<3<4) is currently unverified.

2. **`test_status_incident_resolved_roundtrip`** — exercises
   `models::StatusIncident` with `resolved_at = Some(Utc::now())`. The existing
   `test_status_incident_roundtrip` only covers the `None` (active) branch;
   serde of the `Some` (resolved) branch is untested.

3. **`test_health_route_returns_ok_json`** — exercises `routes::create_router` /
   `health`. Drive the router with `tower::ServiceExt::oneshot` on
   `GET /api/status/health` and assert HTTP 200 and `status == "ok"` in the JSON
   body. The route has zero test coverage today. (Add `tower` as a dev-dep if
   not already present.)

4. **`test_count_by_status_includes_maintenance`** — exercises
   `engine::count_by_status`. The current test asserts Operational / MajorOutage
   / DegradedPerformance buckets but never `UnderMaintenance` or `PartialOutage`;
   add a component of each and assert the debug-keyed counts.

5. **`test_has_issues_true_for_maintenance`** — exercises `engine::has_issues`.
   Current `test_has_issues_true` uses `DegradedPerformance`; add a case with a
   single `UnderMaintenance` component to confirm maintenance also counts as a
   non-operational issue.
