// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak v22.0.0 services/.../oauth2/granttype/ +
// endpoints/{Authorization,TokenRevocation}Endpoint + RFC 7009 + RFC 9126 +
// RFC 8628 + OpenID CIBA.

//! `cavectl auth oauth` — OAuth/OIDC endpoint admin (PAR / Device / CIBA /
//! Revoke). Parity surface tracked by `crates/cave-auth/src/oauth_endpoints/`.

/// `cavectl auth oauth par` — RFC 9126 Pushed Authorization Requests status.
pub const PATH_PAR: &str = "/api/auth/oauth/par";

/// `cavectl auth oauth device` — RFC 8628 device authorization grant status.
pub const PATH_DEVICE: &str = "/api/auth/oauth/device";

/// `cavectl auth oauth ciba` — OpenID CIBA back-channel auth-req status.
pub const PATH_CIBA: &str = "/api/auth/oauth/ciba";

/// `cavectl auth oauth revoke` — RFC 7009 token revocation surface.
pub const PATH_REVOKE: &str = "/api/auth/oauth/revoke";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_oauth_prefix() {
        for p in [PATH_PAR, PATH_DEVICE, PATH_CIBA, PATH_REVOKE] {
            assert!(p.starts_with("/api/auth/oauth/"));
        }
    }
}
