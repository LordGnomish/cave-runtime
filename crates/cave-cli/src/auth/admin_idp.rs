// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak v22.0.0 services/.../admin/IdentityProviderResource.java.

//! `cavectl auth admin-idp` — Identity Provider admin REST surface.
//! Parity tracked by `crates/cave-auth/src/admin_idp/`.

/// `cavectl auth admin-idp instances` — `/admin/realms/{r}/identity-provider/instances`.
pub const PATH_INSTANCES: &str = "/api/auth/admin/identity-provider/instances";

/// `cavectl auth admin-idp mappers` — per-instance attribute mappers.
pub const PATH_MAPPERS: &str = "/api/auth/admin/identity-provider/mappers";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_admin_idp_prefix() {
        for p in [PATH_INSTANCES, PATH_MAPPERS] {
            assert!(p.starts_with("/api/auth/admin/identity-provider/"));
        }
    }
}
