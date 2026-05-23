// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OIDC Discovery — `/realms/{realm}/.well-known/openid-configuration`.
//!
//! Upstream: `services/src/main/java/org/keycloak/protocol/oidc/representations/OIDCConfigurationRepresentation.java`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryDocument {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
    pub end_session_endpoint: String,
    pub introspection_endpoint: String,
    pub revocation_endpoint: String,
    pub device_authorization_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub backchannel_logout_supported: bool,
    pub frontchannel_logout_supported: bool,
}

pub fn discovery_for(realm_id: &str, base_url: &str) -> DiscoveryDocument {
    let prefix = format!("{}/realms/{}", base_url.trim_end_matches('/'), realm_id);
    DiscoveryDocument {
        issuer: prefix.clone(),
        authorization_endpoint: format!("{}/protocol/openid-connect/auth", prefix),
        token_endpoint: format!("{}/protocol/openid-connect/token", prefix),
        userinfo_endpoint: format!("{}/protocol/openid-connect/userinfo", prefix),
        jwks_uri: format!("{}/protocol/openid-connect/certs", prefix),
        end_session_endpoint: format!("{}/protocol/openid-connect/logout", prefix),
        introspection_endpoint: format!("{}/protocol/openid-connect/token/introspect", prefix),
        revocation_endpoint: format!("{}/protocol/openid-connect/revoke", prefix),
        device_authorization_endpoint: format!("{}/protocol/openid-connect/auth/device", prefix),
        response_types_supported: vec!["code".into(), "id_token".into(), "code id_token".into()],
        subject_types_supported: vec!["public".into()],
        grant_types_supported: vec![
            "authorization_code".into(),
            "refresh_token".into(),
            "client_credentials".into(),
            "urn:ietf:params:oauth:grant-type:device_code".into(),
        ],
        token_endpoint_auth_methods_supported: vec![
            "client_secret_basic".into(),
            "client_secret_post".into(),
            "none".into(),
            "private_key_jwt".into(),
        ],
        id_token_signing_alg_values_supported: vec!["ES256".into(), "EdDSA".into(), "ML-DSA-65".into()],
        scopes_supported: vec!["openid".into(), "profile".into(), "email".into(), "offline_access".into(), "roles".into()],
        code_challenge_methods_supported: vec!["S256".into(), "plain".into()],
        backchannel_logout_supported: true,
        frontchannel_logout_supported: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_contains_realm_in_issuer() {
        let d = discovery_for("master", "https://iam.cave.svc");
        assert!(d.issuer.ends_with("/realms/master"));
        assert!(d.authorization_endpoint.ends_with("/auth"));
        assert!(d.token_endpoint.ends_with("/token"));
    }

    #[test]
    fn discovery_trailing_slash_in_base_does_not_double() {
        let d = discovery_for("r1", "https://iam.cave.svc/");
        assert!(!d.jwks_uri.contains("//realms"));
    }

    #[test]
    fn discovery_advertises_s256() {
        let d = discovery_for("r1", "https://x");
        assert!(d.code_challenge_methods_supported.contains(&"S256".to_string()));
    }

    #[test]
    fn discovery_advertises_pqc_alg_placeholder() {
        let d = discovery_for("r1", "https://x");
        assert!(d.id_token_signing_alg_values_supported.contains(&"ML-DSA-65".to_string()));
    }
}
