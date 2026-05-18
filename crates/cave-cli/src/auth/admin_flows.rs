// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak v22.0.0 services/.../admin/AuthenticationManagementResource.java.

//! `cavectl auth admin-flows` — Authentication flows / executions /
//! required-actions admin REST. Parity tracked by `crates/cave-auth/src/admin_flows/`.

/// `cavectl auth admin-flows flows` — `/admin/realms/{r}/authentication/flows`.
pub const PATH_FLOWS: &str = "/api/auth/admin/authentication/flows";

/// `cavectl auth admin-flows executions` — per-flow execution chain.
pub const PATH_EXECUTIONS: &str = "/api/auth/admin/authentication/executions";

/// `cavectl auth admin-flows required-actions` — required-actions list.
pub const PATH_REQUIRED_ACTIONS: &str = "/api/auth/admin/authentication/required-actions";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_admin_flows_prefix() {
        for p in [PATH_FLOWS, PATH_EXECUTIONS, PATH_REQUIRED_ACTIONS] {
            assert!(p.starts_with("/api/auth/admin/authentication/"));
        }
    }
}
