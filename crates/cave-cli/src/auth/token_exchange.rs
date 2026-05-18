// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + RFC 8693 (OAuth Token Exchange) + keycloak v22.0.0
// services/.../protocol/oidc/grants/TokenExchangeGrantType.java.

//! `cavectl auth token-exchange` — `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`
//! admin surface. Parity tracked by `crates/cave-auth/src/token_exchange/`.

/// `cavectl auth token-exchange exchange` — drive a subject_token + actor_token
/// exchange and report the issued token-type + scope.
pub const PATH_EXCHANGE: &str = "/api/auth/token-exchange/exchange";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_token_exchange_prefix() {
        assert!(PATH_EXCHANGE.starts_with("/api/auth/token-exchange/"));
    }
}
