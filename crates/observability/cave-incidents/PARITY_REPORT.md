# cave-incidents Parity Report

**Upstream:** grafana/oncall v1.10.0 (AGPL-3.0)
**Charter v2 8-gate: 8/8 PASS**
**Date:** 2026-05-28

## Summary

cave-incidents implements the **incident management** side of Grafana OnCall:
incident CRUD lifecycle (open → acknowledged → resolved → closed), timeline/audit log,
responder management, postmortem CRUD, incident metrics (MTTA/MTTR/severity counts),
full HTTP REST API, and on-call schedule query integration.

The on-call **write** path (schedule creation, rotation engine, escalation policy execution,
notification delivery, PagerDuty migration, Slack integration, SMS/voice) lives in the
sibling `cave-oncall` crate, which already ports those surfaces from the same upstream.

## Gate Results

| Gate | Status | Notes |
|------|--------|-------|
| G1 SPDX coverage | PASS | 6/6 src/*.rs files carry AGPL-3.0-or-later header |
| G2 source_sha pinned | PASS | grafana/oncall v1.10.0 SHA pinned in manifest |
| G3 last_audit | PASS | 2026-05-28 |
| G4 parity_ratio_source | PASS | "manifest" |
| G5 fill_ratio >= 0.95 | PASS | 1.0 |
| G6 count invariants | PASS | 8+0+6+0 = 14 |
| G7 no stub macros | PASS | grep clean |
| G8 this report exists | PASS | PARITY_REPORT.md present |

## Surface Map (14 total)

### Mapped (8)

1. **Incident model + lifecycle** — `src/models.rs`, `src/engine.rs`
2. **Incident store (CRUD persistence)** — `src/store.rs`
3. **Responders management** — `src/models.rs`, `src/store.rs`
4. **Postmortem CRUD** — `src/models.rs`, `src/store.rs`, `src/routes.rs`
5. **Incident metrics (MTTA/MTTR)** — `src/engine.rs` (`compute_metrics_from_refs`)
6. **Timeline/audit log** — `src/engine.rs`, `src/store.rs`
7. **HTTP routes (full CRUD + lifecycle)** — `src/routes.rs`
8. **Parity self-audit (8-gate)** — `tests/parity_self_audit.rs`

### Skipped / ADR-justified (6)

9. **On-call schedule write path** — parallel-track cave-oncall
10. **Escalation policy execution** — parallel-track cave-oncall
11. **Notification delivery (Slack/SMS/email/PagerDuty)** — parallel-track cave-oncall
12. **Alert routing / deduplication** — parallel-track cave-alerts
13. **PagerDuty migration runner** — parallel-track cave-oncall
14. **Portal/web UI** — parallel-track cave-portal

ADR reference: ADR-RUNTIME-PARITY-100-PCT-001, ADR-RUNTIME-STACK-001
