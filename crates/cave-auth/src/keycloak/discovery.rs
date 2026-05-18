// SPDX-License-Identifier: AGPL-3.0-or-later
//! OpenID Connect Discovery endpoint — GET /realms/{realm}/.well-known/openid-configuration
//!
//! upstream: https://github.com/keycloak/keycloak/blob/v22.0.0/services/src/main/java/org/keycloak/services/resources/RealmsResource.java

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};

use crate::keycloak::token_endpoint::KeycloakTokenService;

pub async fn discovery_endpoint(
    Path(realm): Path<String>,
    State(_svc): State<KeycloakTokenService>,
) -> impl IntoResponse {
    let base = format!("http://localhost:8080/realms/{realm}");
    let body = serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/protocol/openid-connect/auth"),
        "token_endpoint": format!("{base}/protocol/openid-connect/token"),
        "userinfo_endpoint": format!("{base}/protocol/openid-connect/userinfo"),
        "end_session_endpoint": format!("{base}/protocol/openid-connect/logout"),
        "introspection_endpoint": format!("{base}/protocol/openid-connect/token/introspect"),
        "jwks_uri": format!("{base}/protocol/openid-connect/certs"),
        "response_types_supported": ["code", "none"],
        "grant_types_supported": ["authorization_code", "implicit", "refresh_token", "password", "client_credentials"],
        "subject_types_supported": ["public", "pairwise"],
        "id_token_signing_alg_values_supported": ["RS256", "HS256", "ML-DSA65-EdDSA"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
        "scopes_supported": ["openid", "profile", "email", "offline_access"],
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "preferred_username", "email", "email_verified", "name"]
    });
    (StatusCode::OK, Json(body))
}

pub fn router(svc: KeycloakTokenService) -> Router {
    Router::new()
        .route("/realms/{realm}/.well-known/openid-configuration", get(discovery_endpoint))
        .with_state(svc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::{
        client::ClientStore,
        realm::{RealmRequest, RealmStore},
        token_endpoint::KeycloakTokenService,
        user::UserStore,
    };
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn empty_realm_request(id: &str) -> RealmRequest {
        RealmRequest {
            id: id.to_string(),
            display_name: None,
            enabled: None,
            ssl_required: None,
            registration_allowed: None,
            login_with_email_allowed: None,
            duplicate_emails_allowed: None,
            access_token_lifespan: None,
            sso_session_idle_timeout: None,
        }
    }

    async fn fetch_discovery(realm: &str) -> (StatusCode, Value) {
        let realms = RealmStore::new();
        realms.create(empty_realm_request(realm)).await.unwrap();
        let svc = KeycloakTokenService::new(realms, UserStore::new(), ClientStore::new());
        let app = router(svc);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/realms/{realm}/.well-known/openid-configuration"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = body_json(resp).await;
        (status, body)
    }

    // upstream: keycloak/keycloak RealmsResource.java:testDiscoveryEndpoint
    #[tokio::test]
    async fn test_discovery_endpoint() {
        let (status, body) = fetch_discovery("testrealm").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["token_endpoint"].is_string());
        assert!(body["jwks_uri"].is_string());
        assert!(body["issuer"].is_string());
        assert!(body["userinfo_endpoint"].is_string());
        assert!(body["introspection_endpoint"].is_string());
        assert!(body["end_session_endpoint"].is_string());
    }

    // upstream: openid-connect-discovery-1_0.html §3 — issuer must be the
    // exact realm URL with no trailing slash.
    #[tokio::test]
    async fn discovery_issuer_matches_realm_url() {
        let (_, body) = fetch_discovery("acme").await;
        let issuer = body["issuer"].as_str().unwrap();
        assert!(issuer.ends_with("/realms/acme"), "issuer={issuer}");
        assert!(!issuer.ends_with('/'), "issuer must not have trailing slash");
    }

    // upstream: openid-connect-discovery-1_0.html §3 — every endpoint URL
    // is rooted at the realm URL. Catches regressions where a refactor
    // forgets to interpolate `{realm}` into one of the URLs.
    #[tokio::test]
    async fn discovery_all_endpoints_share_realm_root() {
        let (_, body) = fetch_discovery("globex").await;
        let root = body["issuer"].as_str().unwrap().to_string();
        for ep in [
            "authorization_endpoint",
            "token_endpoint",
            "userinfo_endpoint",
            "end_session_endpoint",
            "introspection_endpoint",
            "jwks_uri",
        ] {
            let url = body[ep].as_str().unwrap_or_else(|| panic!("{ep} missing"));
            assert!(
                url.starts_with(&root),
                "endpoint {ep}={url} not rooted at issuer {root}"
            );
        }
    }

    // upstream: openid-connect-discovery-1_0.html §3 — required arrays
    // (response_types, grant_types, subject_types, scopes, claims) must be
    // present and non-empty.
    #[tokio::test]
    async fn discovery_required_arrays_non_empty() {
        let (_, body) = fetch_discovery("required").await;
        for key in [
            "response_types_supported",
            "grant_types_supported",
            "subject_types_supported",
            "id_token_signing_alg_values_supported",
            "token_endpoint_auth_methods_supported",
            "scopes_supported",
            "claims_supported",
        ] {
            let arr = body[key].as_array().unwrap_or_else(|| panic!("{key} not an array"));
            assert!(!arr.is_empty(), "{key} must not be empty");
        }
    }

    // upstream: ADR-PORTAL-AUTH-001 — discovery must advertise the hybrid
    // PQC signing algorithm so RPs can negotiate it.
    #[tokio::test]
    async fn discovery_advertises_pqc_hybrid_alg() {
        let (_, body) = fetch_discovery("pqc").await;
        let algs = body["id_token_signing_alg_values_supported"]
            .as_array()
            .unwrap();
        assert!(
            algs.iter().any(|v| v == "ML-DSA65-EdDSA"),
            "PQC hybrid alg must be advertised; got {algs:?}"
        );
    }

    // upstream: openid-connect-core-1_0.html §3.1.2.1 — `code` response_type
    // must be supported by every conformant authorization server.
    #[tokio::test]
    async fn discovery_supports_authorization_code_flow() {
        let (_, body) = fetch_discovery("code").await;
        let rt = body["response_types_supported"].as_array().unwrap();
        assert!(rt.iter().any(|v| v == "code"));
        let gt = body["grant_types_supported"].as_array().unwrap();
        assert!(gt.iter().any(|v| v == "authorization_code"));
        assert!(gt.iter().any(|v| v == "refresh_token"));
    }

    // upstream: openid-connect-core-1_0.html §5.4 — `sub`, `iss`, `aud`,
    // `exp`, `iat` are required claims; `email` + `email_verified` are part
    // of the `email` scope.
    #[tokio::test]
    async fn discovery_advertises_required_id_token_claims() {
        let (_, body) = fetch_discovery("claims").await;
        let claims = body["claims_supported"].as_array().unwrap();
        for required in ["sub", "iss", "aud", "exp", "iat"] {
            assert!(
                claims.iter().any(|v| v == required),
                "required claim {required} missing"
            );
        }
        for email_claim in ["email", "email_verified"] {
            assert!(
                claims.iter().any(|v| v == email_claim),
                "email-scope claim {email_claim} missing"
            );
        }
    }

    // upstream: rfc6749 §2.3.1 + openid-connect-core-1_0.html §9 — both
    // client_secret_post and client_secret_basic must be advertised so RPs
    // can pick the form they support.
    #[tokio::test]
    async fn discovery_supports_client_secret_post_and_basic() {
        let (_, body) = fetch_discovery("auth").await;
        let methods = body["token_endpoint_auth_methods_supported"]
            .as_array()
            .unwrap();
        assert!(methods.iter().any(|v| v == "client_secret_post"));
        assert!(methods.iter().any(|v| v == "client_secret_basic"));
    }

    // upstream: openid-connect-core-1_0.html §11 — `offline_access` scope
    // is what RPs request to receive a refresh_token; surface it.
    #[tokio::test]
    async fn discovery_supports_offline_access_scope() {
        let (_, body) = fetch_discovery("scopes").await;
        let scopes = body["scopes_supported"].as_array().unwrap();
        for s in ["openid", "profile", "email", "offline_access"] {
            assert!(scopes.iter().any(|v| v == s), "missing scope {s}");
        }
    }

    // upstream: keycloak/keycloak RealmsResource.java — the discovery
    // endpoint does NOT validate realm existence; it serves config for any
    // realm name the path supplies. Document the contract so a future
    // hardening change doesn't accidentally tighten without a test update.
    #[tokio::test]
    async fn discovery_does_not_enforce_realm_existence() {
        let realms = RealmStore::new();
        // Note: no realm created.
        let svc = KeycloakTokenService::new(realms, UserStore::new(), ClientStore::new());
        let app = router(svc);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/realms/never-created/.well-known/openid-configuration")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
