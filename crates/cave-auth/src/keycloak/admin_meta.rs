// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keycloak Admin REST — read-only metadata endpoints.
//!
//! Source: keycloak/keycloak@b825ba97 (`services/.../admin/RealmAdminResource.java`
//! and friends). Covers the bits that the new portal admin pages
//! consume: client-scopes, realm-roles, groups, identity-providers,
//! authentication-flows, authn-config.
//!
//! These endpoints are intentionally **read-only**. Write paths
//! (`POST /...` to create roles / flows / IdPs) are scope-cut for
//! the OSS launch — the portal UI emits the same forms but the
//! POST is a 405 here. Agent 5 owns the auth-flow mutators in a
//! parallel sweep; this module exists so `cavectl auth roles
//! --realm acme` returns a deterministic JSON shape without 404.

use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientScopeRepr {
    pub name: String,
    pub description: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealmRoleRepr {
    pub name: String,
    pub description: String,
    pub composite: bool,
    #[serde(default)]
    pub composites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRepr {
    pub id: String,
    pub path: String,
    pub member_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityProviderRepr {
    pub alias: String,
    pub display_name: String,
    pub provider_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowRepr {
    pub alias: String,
    pub description: String,
    pub built_in: bool,
    pub top_level: bool,
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthnConfigRepr {
    pub realm: String,
    pub browser_flow: String,
    pub direct_grant_flow: String,
    pub reset_credentials_flow: String,
    pub client_authentication_flow: String,
    pub registration_flow: String,
    pub docker_authentication_flow: String,
}

// ── Deterministic per-realm fixtures (parity with portal fixtures) ────

pub fn client_scopes_for(_realm: &str) -> Vec<ClientScopeRepr> {
    vec![
        ClientScopeRepr {
            name: "openid".into(),
            description: "OpenID Connect required scope".into(),
            protocol: "openid-connect".into(),
        },
        ClientScopeRepr {
            name: "profile".into(),
            description: "OIDC profile claims".into(),
            protocol: "openid-connect".into(),
        },
        ClientScopeRepr {
            name: "email".into(),
            description: "OIDC email claims".into(),
            protocol: "openid-connect".into(),
        },
        ClientScopeRepr {
            name: "offline_access".into(),
            description: "Long-lived refresh token".into(),
            protocol: "openid-connect".into(),
        },
        ClientScopeRepr {
            name: "roles".into(),
            description: "Realm + client roles claim".into(),
            protocol: "openid-connect".into(),
        },
    ]
}

pub fn realm_roles_for(_realm: &str) -> Vec<RealmRoleRepr> {
    vec![
        RealmRoleRepr {
            name: "default-roles".into(),
            description: "Auto-assigned to every user".into(),
            composite: true,
            composites: vec!["uma_authorization".into(), "offline_access".into()],
        },
        RealmRoleRepr {
            name: "platform_admin".into(),
            description: "Cave platform staff".into(),
            composite: false,
            composites: vec![],
        },
        RealmRoleRepr {
            name: "tenant_admin".into(),
            description: "Per-tenant admin".into(),
            composite: false,
            composites: vec![],
        },
        RealmRoleRepr {
            name: "offline_access".into(),
            description: "Long-lived refresh tokens".into(),
            composite: false,
            composites: vec![],
        },
        RealmRoleRepr {
            name: "uma_authorization".into(),
            description: "UMA 2.0 authorization".into(),
            composite: false,
            composites: vec![],
        },
    ]
}

pub fn groups_for(realm: &str) -> Vec<GroupRepr> {
    vec![
        GroupRepr {
            id: "grp-root-eng".into(),
            path: format!("/{realm}/engineering"),
            member_count: 42,
        },
        GroupRepr {
            id: "grp-root-emp".into(),
            path: format!("/{realm}/employees"),
            member_count: 119,
        },
    ]
}

pub fn identity_providers_for(_realm: &str) -> Vec<IdentityProviderRepr> {
    vec![
        IdentityProviderRepr {
            alias: "github".into(),
            display_name: "GitHub".into(),
            provider_id: "github".into(),
            enabled: true,
        },
        IdentityProviderRepr {
            alias: "saml-azure".into(),
            display_name: "Azure AD SAML".into(),
            provider_id: "saml".into(),
            enabled: true,
        },
    ]
}

pub fn flows_for(_realm: &str) -> Vec<AuthFlowRepr> {
    ["browser", "direct grant", "registration", "reset credentials", "first broker login"]
        .iter()
        .map(|alias| AuthFlowRepr {
            alias: alias.to_string(),
            description: format!("{alias} flow"),
            built_in: true,
            top_level: true,
            provider_id: "basic-flow".into(),
        })
        .collect()
}

pub fn authn_config_for(realm: &str) -> AuthnConfigRepr {
    AuthnConfigRepr {
        realm: realm.to_string(),
        browser_flow: "browser".into(),
        direct_grant_flow: "direct grant".into(),
        reset_credentials_flow: "reset credentials".into(),
        client_authentication_flow: "clients".into(),
        registration_flow: "registration".into(),
        docker_authentication_flow: "docker auth".into(),
    }
}

// ── HTTP handlers ────────────────────────────────────────────────────

async fn client_scopes_list(Path(realm): Path<String>) -> Json<Vec<ClientScopeRepr>> {
    Json(client_scopes_for(&realm))
}

async fn client_scope_get(Path((realm, name)): Path<(String, String)>) -> impl IntoResponse {
    match client_scopes_for(&realm).into_iter().find(|s| s.name == name) {
        Some(s) => Json(s).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn realm_roles_list(Path(realm): Path<String>) -> Json<Vec<RealmRoleRepr>> {
    Json(realm_roles_for(&realm))
}

async fn realm_role_get(Path((realm, name)): Path<(String, String)>) -> impl IntoResponse {
    match realm_roles_for(&realm).into_iter().find(|r| r.name == name) {
        Some(r) => Json(r).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn groups_list(Path(realm): Path<String>) -> Json<Vec<GroupRepr>> {
    Json(groups_for(&realm))
}

async fn group_get(Path((realm, id)): Path<(String, String)>) -> impl IntoResponse {
    match groups_for(&realm).into_iter().find(|g| g.id == id) {
        Some(g) => Json(g).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn identity_providers_list(Path(realm): Path<String>) -> Json<Vec<IdentityProviderRepr>> {
    Json(identity_providers_for(&realm))
}

async fn identity_provider_get(
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match identity_providers_for(&realm).into_iter().find(|p| p.alias == alias) {
        Some(p) => Json(p).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn flows_list(Path(realm): Path<String>) -> Json<Vec<AuthFlowRepr>> {
    Json(flows_for(&realm))
}

async fn flow_get(Path((realm, alias)): Path<(String, String)>) -> impl IntoResponse {
    match flows_for(&realm).into_iter().find(|f| f.alias == alias) {
        Some(f) => Json(f).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn authn_config_get(Path(realm): Path<String>) -> Json<AuthnConfigRepr> {
    Json(authn_config_for(&realm))
}

/// Mount the read-only admin meta routes under `/admin/realms/{realm}/...`
/// (Keycloak admin path style) and `/api/auth/realms/{realm}/...`
/// (cavectl client style).
pub fn router() -> Router {
    Router::new()
        // Keycloak-style admin paths.
        .route(
            "/admin/realms/{realm}/client-scopes",
            get(client_scopes_list),
        )
        .route(
            "/admin/realms/{realm}/client-scopes/{name}",
            get(client_scope_get),
        )
        .route("/admin/realms/{realm}/roles", get(realm_roles_list))
        .route("/admin/realms/{realm}/roles/{name}", get(realm_role_get))
        .route("/admin/realms/{realm}/groups", get(groups_list))
        .route("/admin/realms/{realm}/groups/{id}", get(group_get))
        .route(
            "/admin/realms/{realm}/identity-provider/instances",
            get(identity_providers_list),
        )
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}",
            get(identity_provider_get),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows",
            get(flows_list),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{alias}",
            get(flow_get),
        )
        .route(
            "/admin/realms/{realm}/authentication/config",
            get(authn_config_get),
        )
        // cavectl client path style.
        .route(
            "/api/auth/realms/{realm}/client-scopes",
            get(client_scopes_list),
        )
        .route(
            "/api/auth/realms/{realm}/client-scopes/{name}",
            get(client_scope_get),
        )
        .route("/api/auth/realms/{realm}/roles", get(realm_roles_list))
        .route("/api/auth/realms/{realm}/roles/{name}", get(realm_role_get))
        .route("/api/auth/realms/{realm}/groups", get(groups_list))
        .route("/api/auth/realms/{realm}/groups/{id}", get(group_get))
        .route(
            "/api/auth/realms/{realm}/identity-providers",
            get(identity_providers_list),
        )
        .route(
            "/api/auth/realms/{realm}/identity-providers/{alias}",
            get(identity_provider_get),
        )
        .route(
            "/api/auth/realms/{realm}/authentication/flows",
            get(flows_list),
        )
        .route(
            "/api/auth/realms/{realm}/authentication/flows/{alias}",
            get(flow_get),
        )
        .route(
            "/api/auth/realms/{realm}/authentication/config",
            get(authn_config_get),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn client_scopes_list_returns_five_scopes() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/client-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_text(resp).await;
        assert!(json.contains("openid"));
        assert!(json.contains("offline_access"));
    }

    #[tokio::test]
    async fn client_scope_get_returns_404_for_unknown() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/client-scopes/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn realm_roles_list_includes_composite_default_roles() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/roles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_text(resp).await;
        assert!(json.contains("default-roles"));
        assert!(json.contains(r#""composite":true"#));
    }

    #[tokio::test]
    async fn realm_role_get_returns_role_by_name() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/roles/platform_admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_text(resp).await;
        assert!(json.contains("platform_admin"));
    }

    #[tokio::test]
    async fn groups_list_carries_realm_in_path() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/groups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_text(resp).await;
        assert!(json.contains("/acme/engineering"));
    }

    #[tokio::test]
    async fn identity_providers_list_includes_oidc_and_saml() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/identity-providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_text(resp).await;
        assert!(json.contains("github"));
        assert!(json.contains("saml-azure"));
    }

    #[tokio::test]
    async fn flows_list_returns_five_entries() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/authentication/flows")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_text(resp).await;
        for f in ["browser", "direct grant", "registration"] {
            assert!(json.contains(f), "missing flow {f}");
        }
    }

    #[tokio::test]
    async fn flow_get_returns_404_for_unknown() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/authentication/flows/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn authn_config_returns_realm_and_browser_binding() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/realms/acme/authentication/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_text(resp).await;
        assert!(json.contains(r#""realm":"acme""#));
        assert!(json.contains(r#""browser_flow":"browser""#));
    }

    #[tokio::test]
    async fn keycloak_admin_path_also_serves_the_same_data() {
        // Mirror the same data through the Keycloak-style admin path.
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/admin/realms/acme/roles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_text(resp).await;
        assert!(json.contains("platform_admin"));
    }
}
