# TDD Coverage Audit — cave-incidents

- **Crate:** `cave-incidents` (theme: observability)
- **Upstream:** [grafana/oncall](https://github.com/grafana/oncall) @ `v1.10.0`
- **Upstream test inventory:** 279 test files / 2073 test symbols (`/tmp/tdd-audit/cave-incidents-upstream-tests.txt`)
- **Cave test functions:** 90 total — 26 unit (`#[test]` in `src/`: engine 17, models 6, silence 2, grouping 1) + 64 integration (`tests/`: `grouping_tdd.rs`, `silence_tdd.rs`, `test_routes.rs`, `test_store.rs`, `test_models_extended.rs`, `parity_self_audit.rs`, `proptest_smoke.rs`)
- **Source modules:** `engine.rs`, `grouping.rs`, `models.rs`, `oncall.rs`, `routes.rs`, `silence.rs`, `store.rs`, `lib.rs`

## Scope reality

grafana/oncall is a large Django on-call/alerting **web application**: REST API viewsets, Slack/Telegram/mobile chatops, phone/SMS/email backends, Celery escalation tasks, iCal schedule rendering, Grafana plugin sync, public API, metrics exporter, permissions/auth. The overwhelming majority of its 2073 test symbols exercise HTTP viewsets, ORM models, messaging integrations, and async task plumbing that cave-incidents does not port.

cave-incidents implements only a **pure, in-memory backend subset**: incident lifecycle state machine (open→ack→resolve→close), timeline/audit entries, MTTA/MTTR metrics, an alert grouping/dedup matcher (port of `get_or_create_grouping` + `render_group_data`), a silence state machine (port of `AlertGroup.status`/`silence`/`un_silence`), and on-call rotation math. Therefore ~99% of upstream tests are **scope-cut**.

The implemented behavior is **already well covered**: the lifecycle, metrics, serde, alert-grouping engine (`tests/grouping_tdd.rs` — distinction = md5, never-group, same/different distinction, channel isolation, resolve-signal re-attach), and the silence state machine (`tests/silence_tdd.rs` — status precedence, forever/period, idempotent silence, un_silence, expiry) all have dedicated tests. The store CRUD/filters and routes are covered by `tests/test_store.rs` and `tests/test_routes.rs`.

The **single real coverage gap** is `oncall.rs`: its rotation and escalation engine functions are public, implemented, and faithfully port upstream shift/escalation logic, yet have **no direct test**. (`store::current_on_call` only touches `current_oncall` at rotation index 0 with no elapsed periods, so the rotation arithmetic itself is untested.)

## Classification

| Upstream behavioral unit | Cave port | Class |
|---|---|---|
| lifecycle create/ack/resolve/close + MTTA/MTTR (`alerts/test_alert.py`, metrics) | `engine::*` | **covered** (`src/engine.rs`) |
| model (de)serialization | `models` serde derives | **covered** (`src/models.rs`, `tests/test_models_extended.rs`) |
| `md5` for `group_distinction`; `get_or_create_grouping` keyed by (channel, distinction); demo/None never group; resolve-signal re-attach (`alerts/models/alert_group.py`, grouping behavior) | `grouping::GroupingEngine::*` | **covered** (`tests/grouping_tdd.rs`) |
| `AlertGroup.status` precedence; `silence`/`un_silence`/forever/period (`alerts/test_silence.py`, `api/test_alert_group.py::test_silence_by_user_*`) | `silence::GroupSilenceState::*` | **covered** (`tests/silence_tdd.rs`) |
| in-memory store CRUD + `list_open`/`list_by_severity` filters | `store::IncidentStore::*` | **covered** (`tests/test_store.rs`) |
| HTTP incident endpoints | `routes::*` | **covered** (`tests/test_routes.rs`) |
| **on-call shift rotation** index/current/upcoming (`api/test_oncall_shift.py` 43, `alerts/test_notify_ical_schedule_shift.py` 13) — *pure rotation-math slice* | `oncall::{current_oncall, upcoming_shifts, rotation_index}` | **portable-coverage** (implemented, no direct test) |
| **escalation step → targets fan-out** (`alerts/test_escalation_snapshot*.py`, `api/test_escalation_policy.py` 29) — *pure target-resolution slice* | `oncall::OnCallEngine::{escalate, page_oncall}`, `default_escalation_policy` | **portable-coverage** (implemented, no test) |
| REST viewsets, auth/permissions, Slack/Telegram/mobile/phone/email chatops, Celery escalation tasks, iCal export, Grafana plugin sync, public API, metrics exporter, webhooks, labels, heartbeat, maintenance, gcom | — (live in cave-portal / cave-net / cave-noti / cave-oncall siblings) | **scope-cut** (web-app / infra / integration plumbing) |

## Recommended TDD fills (portable-coverage first)

All remaining gaps are in `oncall.rs` — implemented rotation/escalation logic that ports upstream shift and escalation-snapshot behavior but is currently untested:

1. **`oncall::OnCallEngine::current_oncall`** (drives private `rotation_index`) — with a weekly layer of 3 users and `starts_at` set 2 weeks in the past, assert it returns the user at `(current_index + 2) % 3`; empty layer and empty-users cases each return `None`. Ports the iCal "who is on-call now" computation (`test_oncall_shift.py`).
2. **`oncall::OnCallEngine::upcoming_shifts`** — `count == 3` returns exactly 3 shifts; each shift spans `rotation_period_days` (`end - start == period`); shifts are contiguous (`shift[i].end == shift[i+1].start`); user names cycle in rotation order. Ports upcoming-shift enumeration (`test_notify_ical_schedule_shift.py`).
3. **`oncall::OnCallEngine::escalate`** — for `default_escalation_policy()`, an in-range step returns one action string per target in that step; an out-of-range step index returns the single `"Step N does not exist"` message. Ports the escalation-snapshot target fan-out (`test_escalation_snapshot*.py`).
4. **`oncall::OnCallEngine::page_oncall`** — given `default_schedule()`, returns `Some(Responder)` whose `user_id`/`name`/`email` match the current on-call user and whose `role == Responder` with `acknowledged_at.is_none()`; a schedule with no users returns `None`. Ports the "page current on-call" path (`test_paging.py`, `test_direct_paging.py` — backend slice only).

Optional (constructor sanity, low value): assert `oncall::default_schedule()` yields a 3-user weekly `Primary` layer and `default_escalation_policy()` yields 3 ascending-delay steps — these are fixtures consumed by the tests above.
