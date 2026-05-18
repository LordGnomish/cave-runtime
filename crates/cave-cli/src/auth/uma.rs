// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + Kantara UMA 2.0 Federated Authz + keycloak v22.0.0
// services/.../authorization/.

//! `cavectl auth uma` — User-Managed Access resource-set / permission-ticket /
//! RPT admin. Parity surface tracked by `crates/cave-auth/src/uma/`.

/// `cavectl auth uma resource-set` — registered resources list.
pub const PATH_RESOURCE_SET: &str = "/api/auth/uma/resource-set";

/// `cavectl auth uma permission-ticket` — outstanding tickets.
pub const PATH_PERMISSION_TICKET: &str = "/api/auth/uma/permission-ticket";

/// `cavectl auth uma rpt` — Requesting Party Token mint/inspect status.
pub const PATH_RPT: &str = "/api/auth/uma/rpt";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_uma_prefix() {
        for p in [PATH_RESOURCE_SET, PATH_PERMISSION_TICKET, PATH_RPT] {
            assert!(p.starts_with("/api/auth/uma/"));
        }
    }
}
