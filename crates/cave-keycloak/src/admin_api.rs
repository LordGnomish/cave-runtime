// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admin REST API surface — pure URL builders + DTO shapes so cave-cli
//! and cave-portal-ui can talk to the running server without duplicating
//! the route prefix logic.
//!
//! Upstream: `services/src/main/java/org/keycloak/services/resources/admin/*Resource.java`.

use serde::{Deserialize, Serialize};

/// Conventional prefix Keycloak uses (`/admin/realms/{realm}/…`).
pub const ADMIN_PREFIX: &str = "/admin/realms";

pub fn realms_url(base: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), ADMIN_PREFIX)
}

pub fn realm_url(base: &str, realm_id: &str) -> String {
    format!("{}/{}", realms_url(base), realm_id)
}

pub fn users_url(base: &str, realm_id: &str) -> String {
    format!("{}/users", realm_url(base, realm_id))
}

pub fn user_url(base: &str, realm_id: &str, user_id: &str) -> String {
    format!("{}/{}", users_url(base, realm_id), user_id)
}

pub fn roles_url(base: &str, realm_id: &str) -> String {
    format!("{}/roles", realm_url(base, realm_id))
}

pub fn clients_url(base: &str, realm_id: &str) -> String {
    format!("{}/clients", realm_url(base, realm_id))
}

pub fn groups_url(base: &str, realm_id: &str) -> String {
    format!("{}/groups", realm_url(base, realm_id))
}

pub fn sessions_url(base: &str, realm_id: &str) -> String {
    format!("{}/sessions", realm_url(base, realm_id))
}

pub fn events_url(base: &str, realm_id: &str) -> String {
    format!("{}/events", realm_url(base, realm_id))
}

/// Common response wrapper — `kind` lets the caller pivot quickly when
/// cave-portal-ui renders a generic list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListResponse<T> {
    pub kind: String,
    pub total: usize,
    pub items: Vec<T>,
}

impl<T> ListResponse<T> {
    pub fn new(kind: &str, items: Vec<T>) -> Self {
        Self { kind: kind.to_string(), total: items.len(), items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_urls_compose_correctly() {
        assert_eq!(realms_url("https://iam"), "https://iam/admin/realms");
        assert_eq!(realm_url("https://iam", "master"), "https://iam/admin/realms/master");
        assert_eq!(users_url("https://iam", "r1"), "https://iam/admin/realms/r1/users");
        assert_eq!(
            user_url("https://iam", "r1", "u-1"),
            "https://iam/admin/realms/r1/users/u-1"
        );
        assert_eq!(roles_url("https://iam", "r1"), "https://iam/admin/realms/r1/roles");
        assert_eq!(clients_url("https://iam", "r1"), "https://iam/admin/realms/r1/clients");
        assert_eq!(groups_url("https://iam", "r1"), "https://iam/admin/realms/r1/groups");
        assert_eq!(sessions_url("https://iam", "r1"), "https://iam/admin/realms/r1/sessions");
        assert_eq!(events_url("https://iam", "r1"), "https://iam/admin/realms/r1/events");
    }

    #[test]
    fn trailing_slash_does_not_double() {
        let u = realms_url("https://iam/");
        // strip scheme so we can assert no `//` in the path portion
        let after_scheme = u.split_once("://").map(|(_, r)| r).unwrap_or(&u);
        assert!(!after_scheme.contains("//"), "{}", u);
    }

    #[test]
    fn list_response_wraps_items() {
        let r: ListResponse<String> = ListResponse::new("user", vec!["u1".into(), "u2".into()]);
        assert_eq!(r.kind, "user");
        assert_eq!(r.total, 2);
    }
}
