// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Integration smoke tests for `/admin/auth/webauthn`.

use cave_portal::admin::auth::webauthn::{group_by_format, list_credentials, render};
use cave_portal::admin::permission::{Permission, RequestCtx};
use cave_portal::admin::state::AdminState;

fn ctx(perms: &[Permission]) -> RequestCtx {
    RequestCtx::developer("acme", perms)
}

#[test]
fn route_handler_returns_html_for_authorised_tenant() {
    let s = AdminState::seeded();
    let html = render(&s, &ctx(&[Permission::WebauthnRead])).unwrap();
    assert!(html.contains("WebAuthn"));
    assert!(html.contains("webauthn-register"));
}

#[test]
fn route_handler_rejects_when_perm_missing() {
    let s = AdminState::seeded();
    assert!(render(&s, &ctx(&[])).is_err());
}

#[test]
fn list_credentials_returns_two_seeded_rows() {
    let s = AdminState::seeded();
    let rows = list_credentials(&s, &ctx(&[Permission::WebauthnRead])).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn group_by_format_pairs_packed_and_none() {
    let s = AdminState::seeded();
    let rows = list_credentials(&s, &ctx(&[Permission::WebauthnRead])).unwrap();
    let groups: std::collections::BTreeMap<_, _> = group_by_format(&rows).into_iter().collect();
    assert_eq!(groups.get("packed").copied(), Some(1));
    assert_eq!(groups.get("none").copied(), Some(1));
}
