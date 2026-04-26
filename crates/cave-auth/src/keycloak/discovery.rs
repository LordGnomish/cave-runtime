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

    // upstream: keycloak/keycloak RealmsResource.java:testDiscoveryEndpoint
    #[tokio::test]
    async fn test_discovery_endpoint() {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "testrealm".to_string(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();

        let svc = KeycloakTokenService::new(realms, UserStore::new(), ClientStore::new());
        let app = router(svc);

        let resp = app.oneshot(
            Request::builder()
                .method("GET")
                .uri("/realms/testrealm/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert!(body["token_endpoint"].is_string());
        assert!(body["jwks_uri"].is_string());
        assert!(body["issuer"].is_string());
        assert!(body["userinfo_endpoint"].is_string());
        assert!(body["introspection_endpoint"].is_string());
        assert!(body["end_session_endpoint"].is_string());
    }
}
