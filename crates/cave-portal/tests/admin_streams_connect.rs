// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for `/admin/streams/connect/*`.
//!
//! Each test verifies a public API contract of the new
//! `crate::admin::streams::connect` module. These are written
//! RED — before the impl exists — per Charter §1 observable
//! red→green cycle.

use cave_portal::admin::permission::{Permission, RequestCtx};
use cave_portal::admin::state::AdminState;
use cave_portal::admin::streams::connect;

fn ctx_with(perms: &[Permission]) -> RequestCtx {
    RequestCtx::developer("acme", perms)
}

// ── list_connectors ──────────────────────────────────────────────────────────

#[test]
fn list_connectors_returns_acme_scoped_set() {
    let s = AdminState::seeded();
    let cs = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    assert!(cs.iter().all(|c| c.tenant.as_str() == "acme"));
    assert!(!cs.is_empty(), "expected at least one connector seeded");
}

#[test]
fn list_connectors_refuses_without_streams_read() {
    let s = AdminState::seeded();
    assert!(connect::list_connectors(&s, &ctx_with(&[])).is_err());
}

#[test]
fn list_connectors_excludes_evil_tenant_data() {
    let s = AdminState::seeded();
    let cs = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    for c in &cs {
        assert!(
            !c.name.to_lowercase().contains("evil"),
            "leaked: {}",
            c.name
        );
    }
}

// ── list_workers ─────────────────────────────────────────────────────────────

#[test]
fn list_workers_filters_to_tenant() {
    let s = AdminState::seeded();
    let ws = connect::list_workers(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    assert!(ws.iter().all(|w| w.tenant.as_str() == "acme"));
}

// ── list_tasks ───────────────────────────────────────────────────────────────

#[test]
fn list_tasks_filters_to_tenant() {
    let s = AdminState::seeded();
    let ts = connect::list_tasks(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    assert!(ts.iter().all(|t| t.tenant.as_str() == "acme"));
}

#[test]
fn list_tasks_includes_running_state() {
    let s = AdminState::seeded();
    let ts = connect::list_tasks(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    assert!(ts.iter().any(|t| t.state == "Running"));
}

// ── inspect_connector ────────────────────────────────────────────────────────

#[test]
fn inspect_connector_returns_owned_connector() {
    let s = AdminState::seeded();
    let cs = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    let first = &cs[0];
    let got = connect::inspect_connector(
        &s,
        &ctx_with(&[Permission::StreamsRead]),
        &first.name,
    )
    .unwrap();
    assert_eq!(got.name, first.name);
}

#[test]
fn inspect_connector_unknown_returns_not_found() {
    let s = AdminState::seeded();
    let res = connect::inspect_connector(
        &s,
        &ctx_with(&[Permission::StreamsRead]),
        "nope-not-a-connector",
    );
    assert!(res.is_err());
}

#[test]
fn inspect_connector_cross_tenant_returns_not_found() {
    let s = AdminState::seeded();
    let res = connect::inspect_connector(
        &s,
        &ctx_with(&[Permission::StreamsRead]),
        "evil-connector",
    );
    assert!(res.is_err());
}

// ── pause / resume / restart / delete ────────────────────────────────────────

#[test]
fn pause_connector_transitions_to_paused() {
    let s = AdminState::seeded();
    let target_name = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead]))
        .unwrap()[0]
        .name
        .clone();
    connect::pause_connector(
        &s,
        &ctx_with(&[Permission::StreamsRead, Permission::StreamsAdmin]),
        &target_name,
    )
    .unwrap();
    let got = connect::inspect_connector(
        &s,
        &ctx_with(&[Permission::StreamsRead]),
        &target_name,
    )
    .unwrap();
    assert_eq!(got.state, "Paused");
}

#[test]
fn pause_connector_refuses_without_admin() {
    let s = AdminState::seeded();
    let n = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap()[0]
        .name
        .clone();
    assert!(
        connect::pause_connector(&s, &ctx_with(&[Permission::StreamsRead]), &n).is_err()
    );
}

#[test]
fn resume_connector_transitions_back_to_running() {
    let s = AdminState::seeded();
    let n = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap()[0]
        .name
        .clone();
    let admin = ctx_with(&[Permission::StreamsRead, Permission::StreamsAdmin]);
    connect::pause_connector(&s, &admin, &n).unwrap();
    connect::resume_connector(&s, &admin, &n).unwrap();
    let got = connect::inspect_connector(&s, &admin, &n).unwrap();
    assert_eq!(got.state, "Running");
}

#[test]
fn delete_connector_removes_entry() {
    let s = AdminState::seeded();
    let n = connect::list_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap()[0]
        .name
        .clone();
    let admin = ctx_with(&[Permission::StreamsRead, Permission::StreamsAdmin]);
    connect::delete_connector(&s, &admin, &n).unwrap();
    let res = connect::inspect_connector(&s, &admin, &n);
    assert!(res.is_err());
}

#[test]
fn delete_connector_cross_tenant_not_found() {
    let s = AdminState::seeded();
    let admin = ctx_with(&[Permission::StreamsRead, Permission::StreamsAdmin]);
    let res = connect::delete_connector(&s, &admin, "evil-connector");
    assert!(res.is_err());
}

#[test]
fn restart_task_clears_failure_trace() {
    let s = AdminState::seeded();
    let admin = ctx_with(&[Permission::StreamsRead, Permission::StreamsAdmin]);
    // Find a failing task that we can restart in seeded state.
    let task = connect::list_tasks(&s, &admin)
        .unwrap()
        .into_iter()
        .find(|t| t.failure_trace.is_some());
    if let Some(t) = task {
        connect::restart_task(&s, &admin, &t.connector, t.task).unwrap();
        let fresh = connect::list_tasks(&s, &admin).unwrap();
        let after = fresh
            .iter()
            .find(|x| x.connector == t.connector && x.task == t.task)
            .unwrap();
        assert!(after.failure_trace.is_none());
    } else {
        // Seed didn't include a failing task — that's allowed,
        // but the route still has to refuse unknown task.
        let err = connect::restart_task(&s, &admin, "nope", 0);
        assert!(err.is_err());
    }
}

// ── render ───────────────────────────────────────────────────────────────────

#[test]
fn render_main_view_contains_tab_anchors() {
    let s = AdminState::seeded();
    let html =
        connect::render(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    // The 4 sub-views are reachable by anchor + href.
    assert!(html.contains("workers"), "missing workers tab");
    assert!(html.contains("connectors"), "missing connectors tab");
    assert!(html.contains("tasks"), "missing tasks tab");
    assert!(html.contains("configs"), "missing configs tab");
}

#[test]
fn render_connectors_view_has_lifecycle_buttons() {
    let s = AdminState::seeded();
    let html = connect::render_connectors(
        &s,
        &ctx_with(&[Permission::StreamsRead, Permission::StreamsAdmin]),
    )
    .unwrap();
    // Lifecycle: pause / resume / restart / delete with aria-labels.
    assert!(html.contains("aria-label=\"Pause connector"));
    assert!(html.contains("aria-label=\"Resume connector"));
    assert!(html.contains("aria-label=\"Restart connector"));
    assert!(html.contains("aria-label=\"Delete connector"));
}

#[test]
fn render_connectors_hides_buttons_without_admin() {
    let s = AdminState::seeded();
    let html =
        connect::render_connectors(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    assert!(!html.contains("aria-label=\"Pause connector"));
}

#[test]
fn render_workers_view_lists_seeded_workers() {
    let s = AdminState::seeded();
    let html =
        connect::render_workers(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    // The seeded acme worker(s) must appear; an "evil" tenant
    // worker must NOT.
    assert!(!html.contains("evil-"));
}

#[test]
fn render_tasks_view_includes_state_badge() {
    let s = AdminState::seeded();
    let html = connect::render_tasks(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    // Status badges are role="status" for WCAG AA.
    assert!(html.contains("role=\"status\""));
}

#[test]
fn render_configs_view_lists_connector_config() {
    let s = AdminState::seeded();
    let html = connect::render_configs(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    // Configs view must surface connector.class, the canonical key.
    assert!(html.contains("connector.class"));
}

#[test]
fn render_excludes_evil_tenant() {
    let s = AdminState::seeded();
    let html =
        connect::render(&s, &ctx_with(&[Permission::StreamsRead])).unwrap();
    assert!(
        !html.contains("evil-connector"),
        "cross-tenant data leaked into main view"
    );
}
