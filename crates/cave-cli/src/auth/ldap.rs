// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak@v22.0.0 federation/ldap/

//! `cavectl auth ldap` — LDAP federation administration commands.
//!
//! HTTP paths kept in one place so the dispatch table in main.rs
//! stays as a single-line match per variant.

/// `cavectl auth ldap test-connection` — bind against the
/// configured LDAP federation provider and report the
/// resultCode.
pub const PATH_TEST_CONNECTION: &str = "/api/auth/ldap/test-connection";

/// `cavectl auth ldap sync-users` — full user-federation sync.
pub const PATH_SYNC_USERS: &str = "/api/auth/ldap/sync-users";

/// `cavectl auth ldap sync-groups` — group + memberOf sync.
pub const PATH_SYNC_GROUPS: &str = "/api/auth/ldap/sync-groups";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_start_with_api_auth_ldap_prefix() {
        for p in [PATH_TEST_CONNECTION, PATH_SYNC_USERS, PATH_SYNC_GROUPS] {
            assert!(
                p.starts_with("/api/auth/ldap/"),
                "ldap subcommand paths must share /api/auth/ldap/ prefix: {p}"
            );
        }
    }
}
