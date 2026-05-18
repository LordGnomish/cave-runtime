// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak v22.0.0 services/.../protocol/wsfed/

//! `cavectl auth wsfed` — WS-Federation 1.x signin / signout / metadata.
//! Parity surface tracked by `crates/cave-auth/src/wsfed/`.

/// `cavectl auth wsfed metadata` — `<fed:FederationMetadata>` XML download.
pub const PATH_METADATA: &str = "/api/auth/wsfed/metadata";

/// `cavectl auth wsfed signin` — `wa=wsignin1.0` RST → RSTR roundtrip status.
pub const PATH_SIGNIN: &str = "/api/auth/wsfed/signin";

/// `cavectl auth wsfed signout` — `wa=wsignout1.0` SLO surface.
pub const PATH_SIGNOUT: &str = "/api/auth/wsfed/signout";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_wsfed_prefix() {
        for p in [PATH_METADATA, PATH_SIGNIN, PATH_SIGNOUT] {
            assert!(p.starts_with("/api/auth/wsfed/"));
        }
    }
}
