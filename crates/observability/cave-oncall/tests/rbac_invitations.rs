// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! RED→GREEN strict-TDD port of grafana/oncall `user_management` RBAC and the
//! alert-group `Invitation` flow.
//!
//!   * RBAC: `apps/api/permissions.py` — LegacyAccessControlRole (ADMIN=0,
//!     EDITOR=1, VIEWER=2, NONE=3), the permission catalog with fallback
//!     roles, `get_most_authorized_role` (min by value) and
//!     `user_has_minimum_required_basic_role` (user.role <= required.value).
//!   * Invitations: `apps/alerts/models/invitation.py` — ATTEMPTS_LIMIT,
//!     `time_deltas_by_attempts`, `get_delay_by_attempt`, and the
//!     invite/re-invite/stop lifecycle.

use cave_oncall::invitations::{get_delay_by_attempt, InvitationOutcome, InvitationStore, ATTEMPTS_LIMIT};
use cave_oncall::rbac::{
    self, get_most_authorized_role, user_is_authorized, Permission, Role,
};
use uuid::Uuid;

// ── Role ladder — LegacyAccessControlRole IntEnum (lower = more privileged) ──

#[test]
fn test_role_levels_match_upstream_intenum() {
    assert_eq!(Role::Admin.level(), 0);
    assert_eq!(Role::Editor.level(), 1);
    assert_eq!(Role::Viewer.level(), 2);
    assert_eq!(Role::NoAccess.level(), 3);
}

// ── Permission value format: "{prefix}.{resource}:{action}" ──────────────────

#[test]
fn test_permission_value_string() {
    let p = rbac::ALERT_GROUPS_WRITE;
    assert_eq!(p.value(), "grafana-oncall-app.alert-groups:write");
    assert_eq!(p.fallback_role, Role::Editor);
    assert_eq!(rbac::INTEGRATIONS_WRITE.value(), "grafana-oncall-app.integrations:write");
    assert_eq!(rbac::INTEGRATIONS_WRITE.fallback_role, Role::Admin);
}

// ── get_most_authorized_role = min fallback by value; NONE for empty ─────────

#[test]
fn test_most_authorized_role_is_min() {
    let perms: &[Permission] = &[rbac::ALERT_GROUPS_READ, rbac::ALERT_GROUPS_WRITE];
    // VIEWER(2) and EDITOR(1) -> most authorized is EDITOR(1)
    assert_eq!(get_most_authorized_role(perms), Role::Editor);
    assert_eq!(get_most_authorized_role(&[]), Role::NoAccess);
    assert_eq!(
        get_most_authorized_role(&[rbac::ADMIN, rbac::SCHEDULES_READ]),
        Role::Admin
    );
}

// ── user_is_authorized — basic-role check (user.role <= required) ────────────

#[test]
fn test_admin_authorized_for_everything() {
    for p in rbac::catalog() {
        assert!(
            user_is_authorized(Role::Admin, &[p]),
            "admin must satisfy {}",
            p.value()
        );
    }
}

#[test]
fn test_viewer_read_only() {
    assert!(user_is_authorized(Role::Viewer, &[rbac::ALERT_GROUPS_READ]));
    assert!(user_is_authorized(Role::Viewer, &[rbac::SCHEDULES_READ]));
    // Viewer cannot write or admin
    assert!(!user_is_authorized(Role::Viewer, &[rbac::ALERT_GROUPS_WRITE]));
    assert!(!user_is_authorized(Role::Viewer, &[rbac::INTEGRATIONS_WRITE]));
    assert!(!user_is_authorized(Role::Viewer, &[rbac::ADMIN]));
}

#[test]
fn test_editor_can_write_but_not_admin_resources() {
    assert!(user_is_authorized(Role::Editor, &[rbac::ALERT_GROUPS_WRITE]));
    assert!(user_is_authorized(Role::Editor, &[rbac::SCHEDULES_WRITE]));
    assert!(user_is_authorized(Role::Editor, &[rbac::ALERT_GROUPS_READ]));
    // integrations:write and escalation-chains:write fall back to ADMIN
    assert!(!user_is_authorized(Role::Editor, &[rbac::INTEGRATIONS_WRITE]));
    assert!(!user_is_authorized(Role::Editor, &[rbac::ESCALATION_CHAINS_WRITE]));
    assert!(!user_is_authorized(Role::Editor, &[rbac::ADMIN]));
}

#[test]
fn test_no_access_denied_everywhere_but_empty_is_allowed() {
    assert!(!user_is_authorized(Role::NoAccess, &[rbac::ALERT_GROUPS_READ]));
    // Empty required-permissions set => authorized (matches upstream).
    assert!(user_is_authorized(Role::NoAccess, &[]));
}

#[test]
fn test_authorized_requires_all_permissions_strictest_wins() {
    // Mixed set: read(VIEWER) + write(ADMIN). Editor satisfies read but not
    // the ADMIN-gated write, so the combined check must fail for Editor.
    let mixed: &[Permission] = &[rbac::ALERT_GROUPS_READ, rbac::OUTGOING_WEBHOOKS_WRITE];
    assert!(!user_is_authorized(Role::Editor, mixed));
    assert!(user_is_authorized(Role::Admin, mixed));
}

// ── Invitations — apps/alerts/models/invitation.py ───────────────────────────

#[test]
fn test_attempts_limit_and_left() {
    assert_eq!(ATTEMPTS_LIMIT, 10);
}

#[test]
fn test_get_delay_by_attempt_table() {
    // time_deltas_by_attempts = [6m, 16m, 31m, 1h1m, 3h1m]
    assert_eq!(get_delay_by_attempt(0).num_seconds(), 6 * 60);
    assert_eq!(get_delay_by_attempt(1).num_seconds(), 16 * 60);
    assert_eq!(get_delay_by_attempt(2).num_seconds(), 31 * 60);
    assert_eq!(get_delay_by_attempt(3).num_seconds(), 3660); // 1h1m
    assert_eq!(get_delay_by_attempt(4).num_seconds(), 10860); // 3h1m
    // Beyond the table, the last (largest) delay is reused.
    assert_eq!(get_delay_by_attempt(9).num_seconds(), 10860);
}

#[test]
fn test_invite_then_reinvite_lifecycle() {
    let mut store = InvitationStore::default();
    let alert = Uuid::new_v4();

    // First invite -> TYPE_INVITE
    let (outcome, first_id) = store.invite_user("bob", alert, "alice");
    assert_eq!(outcome, InvitationOutcome::Invite);
    assert_eq!(store.active_for(alert).len(), 1);
    let inv = store.get(first_id).unwrap();
    assert!(inv.is_active);
    assert_eq!(inv.attempts_left(), ATTEMPTS_LIMIT);

    // Re-invite same invitee on same alert -> old one deactivated, TYPE_RE_INVITE
    let (outcome2, second_id) = store.invite_user("bob", alert, "alice");
    assert_eq!(outcome2, InvitationOutcome::ReInvite);
    assert_ne!(first_id, second_id);
    assert!(!store.get(first_id).unwrap().is_active);
    assert!(store.get(second_id).unwrap().is_active);
    // Still exactly one active invitation for this alert
    assert_eq!(store.active_for(alert).len(), 1);
}

#[test]
fn test_stop_invitation_deactivates() {
    let mut store = InvitationStore::default();
    let alert = Uuid::new_v4();
    let (_, id) = store.invite_user("carol", alert, "alice");
    assert!(store.get(id).unwrap().is_active);
    store.stop_invitation(id).expect("stop");
    assert!(!store.get(id).unwrap().is_active);
    assert_eq!(store.active_for(alert).len(), 0);
}

#[test]
fn test_stop_unknown_invitation_errors() {
    let mut store = InvitationStore::default();
    assert!(store.stop_invitation(Uuid::new_v4()).is_err());
}
