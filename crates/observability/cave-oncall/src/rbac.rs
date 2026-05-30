// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Role-based access control — a faithful port of grafana/oncall
//! `engine/apps/api/permissions.py` (v1.10.0).
//!
//! Upstream models authorization two ways: fine-grained RBAC permission
//! strings (`grafana-oncall-app.<resource>:<action>`) and a legacy basic-role
//! fallback. cave-oncall ports the legacy basic-role path, which every
//! permission still carries via its `fallback_role`:
//!
//!   * [`Role`] mirrors `LegacyAccessControlRole` — an IntEnum where a *lower*
//!     value is *more* privileged (ADMIN=0 … NONE=3).
//!   * [`get_most_authorized_role`] returns the most-privileged fallback role
//!     among a set of permissions (the `min` by value).
//!   * [`user_is_authorized`] grants access iff the user's role is at least as
//!     privileged as the strictest required permission.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Role ladder
// ---------------------------------------------------------------------------

/// Port of `LegacyAccessControlRole` (IntEnum). Lower value = more access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "serde", rename_all = "snake_case")]
pub enum Role {
    /// `ADMIN = 0`
    Admin,
    /// `EDITOR = 1`
    Editor,
    /// `VIEWER = 2`
    Viewer,
    /// `NONE = 3`
    NoAccess,
}

impl Role {
    /// The numeric IntEnum value (ADMIN=0 … NONE=3).
    pub fn level(self) -> u8 {
        match self {
            Role::Admin => 0,
            Role::Editor => 1,
            Role::Viewer => 2,
            Role::NoAccess => 3,
        }
    }
}

// ---------------------------------------------------------------------------
// Permission
// ---------------------------------------------------------------------------

/// Prefix for the OnCall Grafana plugin (`PluginID.ONCALL`).
pub const PLUGIN_PREFIX: &str = "grafana-oncall-app";

/// Port of `LegacyAccessControlCompatiblePermission` — a `resource:action`
/// pair with the basic role it falls back to when RBAC is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    pub resource: &'static str,
    pub action: &'static str,
    pub fallback_role: Role,
}

impl Permission {
    const fn new(resource: &'static str, action: &'static str, fallback_role: Role) -> Self {
        Self {
            resource,
            action,
            fallback_role,
        }
    }

    /// The RBAC permission string: `{prefix}.{resource}:{action}`.
    pub fn value(&self) -> String {
        format!("{PLUGIN_PREFIX}.{}:{}", self.resource, self.action)
    }

    /// `permission.user_has_permission(user)` — basic-role path.
    pub fn user_has_permission(&self, user_role: Role) -> bool {
        user_is_authorized(user_role, &[*self])
    }
}

// ---------------------------------------------------------------------------
// Permission catalog (RBACPermission.Permissions)
// ---------------------------------------------------------------------------

macro_rules! perm {
    ($name:ident, $res:literal, $act:literal, $role:expr) => {
        pub const $name: Permission = Permission::new($res, $act, $role);
    };
}

perm!(ADMIN, "admin", "admin", Role::Admin);

perm!(ALERT_GROUPS_READ, "alert-groups", "read", Role::Viewer);
perm!(ALERT_GROUPS_WRITE, "alert-groups", "write", Role::Editor);
perm!(ALERT_GROUPS_DIRECT_PAGING, "alert-groups", "direct-paging", Role::Editor);

perm!(INTEGRATIONS_READ, "integrations", "read", Role::Viewer);
perm!(INTEGRATIONS_TEST, "integrations", "test", Role::Editor);
perm!(INTEGRATIONS_WRITE, "integrations", "write", Role::Admin);

perm!(ESCALATION_CHAINS_READ, "escalation-chains", "read", Role::Viewer);
perm!(ESCALATION_CHAINS_WRITE, "escalation-chains", "write", Role::Admin);

perm!(SCHEDULES_READ, "schedules", "read", Role::Viewer);
perm!(SCHEDULES_WRITE, "schedules", "write", Role::Editor);
perm!(SCHEDULES_EXPORT, "schedules", "export", Role::Editor);

perm!(CHATOPS_READ, "chatops", "read", Role::Viewer);
perm!(CHATOPS_WRITE, "chatops", "write", Role::Editor);
perm!(CHATOPS_UPDATE_SETTINGS, "chatops", "update-settings", Role::Admin);

perm!(OUTGOING_WEBHOOKS_READ, "outgoing-webhooks", "read", Role::Viewer);
perm!(OUTGOING_WEBHOOKS_WRITE, "outgoing-webhooks", "write", Role::Admin);

perm!(MAINTENANCE_READ, "maintenance", "read", Role::Viewer);
perm!(MAINTENANCE_WRITE, "maintenance", "write", Role::Editor);

perm!(API_KEYS_READ, "api-keys", "read", Role::Admin);
perm!(API_KEYS_WRITE, "api-keys", "write", Role::Admin);

perm!(NOTIFICATIONS_READ, "notifications", "read", Role::Editor);

perm!(NOTIFICATION_SETTINGS_READ, "notification-settings", "read", Role::Viewer);
perm!(NOTIFICATION_SETTINGS_WRITE, "notification-settings", "write", Role::Editor);

perm!(USER_SETTINGS_READ, "user-settings", "read", Role::Viewer);
perm!(USER_SETTINGS_WRITE, "user-settings", "write", Role::Editor);
perm!(USER_SETTINGS_ADMIN, "user-settings", "admin", Role::Admin);

perm!(OTHER_SETTINGS_READ, "other-settings", "read", Role::Viewer);
perm!(OTHER_SETTINGS_WRITE, "other-settings", "write", Role::Admin);

perm!(LABEL_CREATE, "label", "create", Role::Editor);
perm!(LABEL_READ, "label", "read", Role::Viewer);
perm!(LABEL_WRITE, "label", "write", Role::Editor);

/// Every catalogued permission (mirrors `RBACPermission.Permissions`).
pub fn catalog() -> Vec<Permission> {
    vec![
        ADMIN,
        ALERT_GROUPS_READ,
        ALERT_GROUPS_WRITE,
        ALERT_GROUPS_DIRECT_PAGING,
        INTEGRATIONS_READ,
        INTEGRATIONS_TEST,
        INTEGRATIONS_WRITE,
        ESCALATION_CHAINS_READ,
        ESCALATION_CHAINS_WRITE,
        SCHEDULES_READ,
        SCHEDULES_WRITE,
        SCHEDULES_EXPORT,
        CHATOPS_READ,
        CHATOPS_WRITE,
        CHATOPS_UPDATE_SETTINGS,
        OUTGOING_WEBHOOKS_READ,
        OUTGOING_WEBHOOKS_WRITE,
        MAINTENANCE_READ,
        MAINTENANCE_WRITE,
        API_KEYS_READ,
        API_KEYS_WRITE,
        NOTIFICATIONS_READ,
        NOTIFICATION_SETTINGS_READ,
        NOTIFICATION_SETTINGS_WRITE,
        USER_SETTINGS_READ,
        USER_SETTINGS_WRITE,
        USER_SETTINGS_ADMIN,
        OTHER_SETTINGS_READ,
        OTHER_SETTINGS_WRITE,
        LABEL_CREATE,
        LABEL_READ,
        LABEL_WRITE,
    ]
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

/// Port of `get_most_authorized_role` — the `min` fallback role by value
/// (most privileged). Empty input yields `NONE`, matching upstream.
pub fn get_most_authorized_role(permissions: &[Permission]) -> Role {
    permissions
        .iter()
        .map(|p| p.fallback_role)
        .min_by_key(|r| r.level())
        .unwrap_or(Role::NoAccess)
}

/// Port of `user_has_minimum_required_basic_role` — `user.role <= required`.
pub fn user_has_minimum_required_basic_role(user_role: Role, required: Role) -> bool {
    user_role.level() <= required.level()
}

/// Port of `user_is_authorized` (basic-role path). A user is authorized iff
/// their role meets the most-privileged role required by the permission set —
/// i.e. they satisfy the strictest permission, hence all of them.
pub fn user_is_authorized(user_role: Role, required_permissions: &[Permission]) -> bool {
    user_has_minimum_required_basic_role(user_role, get_most_authorized_role(required_permissions))
}
