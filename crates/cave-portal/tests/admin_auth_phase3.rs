// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// cave-portal `/admin/auth/{webauthn,flows,idp}` sub-page surface — Keycloak
// Phase 3 four-track close-out. Asserts each new sub-page exposes
// `list_*` + `render` + a Permission gate consistent with the rest of
// `admin/auth/`.

use cave_portal::admin::auth::{flows, idp, webauthn};
use cave_portal::admin::permission::{Permission, RequestCtx};
use cave_portal::admin::state::AdminState;

fn ctx(perms: &[Permission]) -> RequestCtx {
    RequestCtx::developer("acme", perms)
}

// ── /admin/auth/webauthn ────────────────────────────────────────────────────

#[test]
fn webauthn_lists_registered_credentials_per_user() {
    let rows = webauthn::list_credentials(
        &AdminState::seeded(),
        &ctx(&[Permission::AuthSessionsRead]),
    )
    .unwrap();
    assert!(
        rows.iter().any(|r| !r.user_id.is_empty()),
        "expected at least one webauthn credential row from seeded sessions"
    );
}

#[test]
fn webauthn_excludes_other_tenants() {
    let rows = webauthn::list_credentials(
        &AdminState::seeded(),
        &ctx(&[Permission::AuthSessionsRead]),
    )
    .unwrap();
    assert!(rows.iter().all(|r| !r.user_id.contains("evil")));
}

#[test]
fn webauthn_rejects_without_permission() {
    assert!(webauthn::list_credentials(&AdminState::seeded(), &ctx(&[])).is_err());
}

#[test]
fn webauthn_render_lists_credentials_panel() {
    let html = webauthn::render(
        &AdminState::seeded(),
        &ctx(&[Permission::AuthSessionsRead]),
    )
    .unwrap();
    assert!(html.contains("Passkeys"));
    assert!(html.contains("/admin/auth/webauthn"));
}

// ── /admin/auth/flows ───────────────────────────────────────────────────────

#[test]
fn flows_lists_authentication_flows() {
    let rows =
        flows::list_flows(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
    // Keycloak ships built-in flows: browser, direct-grant, reset-credentials,
    // registration, clients, first-broker-login, http-challenge, docker-auth.
    // We mirror the headline set so the operator sees a realistic chain.
    assert!(rows.iter().any(|r| r.alias == "browser"));
    assert!(rows.iter().any(|r| r.alias == "direct-grant"));
}

#[test]
fn flows_marks_browser_flow_as_top_level() {
    let rows =
        flows::list_flows(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
    let browser = rows.iter().find(|r| r.alias == "browser").unwrap();
    assert!(browser.top_level, "browser flow must be top-level");
}

#[test]
fn flows_rejects_without_permission() {
    assert!(flows::list_flows(&AdminState::seeded(), &ctx(&[])).is_err());
}

#[test]
fn flows_render_emits_flow_table() {
    let html =
        flows::render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
    assert!(html.contains("Authentication Flows"));
    assert!(html.contains("browser"));
}

// ── /admin/auth/idp ─────────────────────────────────────────────────────────

#[test]
fn idp_lists_identity_provider_instances() {
    let rows = idp::list_instances(
        &AdminState::seeded(),
        &ctx(&[Permission::AuthSessionsRead]),
    )
    .unwrap();
    assert!(
        rows.iter().any(|r| r.alias == "saml-broker"),
        "must surface the saml broker instance"
    );
}

#[test]
fn idp_rejects_without_permission() {
    assert!(idp::list_instances(&AdminState::seeded(), &ctx(&[])).is_err());
}

#[test]
fn idp_render_emits_provider_table() {
    let html = idp::render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
    assert!(html.contains("Identity Providers"));
    assert!(html.contains("saml-broker"));
}
