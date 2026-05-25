// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: cave-runtime — admin handler mount tests for K1.
//
// End-to-end roundtrip via `axum::Router::oneshot`. We don't boot a real
// TCP listener; we mount `cave_auth::admin_idp::router` and
// `cave_auth::admin_flows::router` behind the same JWT middleware that
// `cave-runtime/src/main.rs` builds, then drive HTTP requests through.
//
// Coverage:
//   identity-provider/instances  — list / create / get / update / delete
//   authentication/flows         — list / create / get / update / delete
//   negative                     — missing JWT → 401, tenant_admin → 403
//
// JWT is forged with the same HS256 secret-derived key the runtime would
// honour at boot (`CAVE_JWT_SECRET`). Persona is encoded in the `roles`
// claim (`platform_admin` vs `tenant_admin`).

use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use cave_auth::{
    admin_flows::{self, AdminFlowsState},
    admin_idp::{self, AdminIdpState},
    jwt_middleware::{AuthState, JwtClaims, auth_middleware_inner},
};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

const JWT_SECRET: &str = "k1-test-secret-do-not-use-in-prod";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn forge_token(roles: &[&str]) -> String {
    let claims = JwtClaims {
        sub: "test-user".into(),
        email: "tester@example.com".into(),
        roles: roles.iter().map(|s| s.to_string()).collect(),
        exp: (chrono::Utc::now().timestamp() + 3600) as usize,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .expect("encode JWT")
}

/// Persona gate — only `platform_admin` may touch `/admin/realms/*`. Tenant
/// admins get 403, anonymous callers are already blocked at the JWT layer.
/// Mirrors `cave_portal::admin::adr` style `require_persona(PlatformAdmin)`.
async fn platform_admin_gate(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path();
    if path.starts_with("/admin/realms/") {
        let allowed = req
            .extensions()
            .get::<JwtClaims>()
            .map(|c| c.roles.iter().any(|r| r == "platform_admin"))
            .unwrap_or(false);
        if !allowed {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "platform_admin persona required" })),
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Assemble the same admin stack `cave-runtime/src/main.rs` mounts. The
/// JWT layer comes *outside* the merged routers so unauthenticated calls
/// short-circuit before the persona gate.
fn build_app() -> Router {
    let idp = admin_idp::router(AdminIdpState::new());
    let flows = admin_flows::router(AdminFlowsState::new());

    let auth_state = Arc::new(AuthState {
        jwt_secret: JWT_SECRET.into(),
        bypass_paths: vec!["_exact:/".into(), "/health".into(), "/ready".into()],
    });

    Router::new()
        .merge(idp)
        .merge(flows)
        .layer(axum::middleware::from_fn(platform_admin_gate))
        .layer(axum::middleware::from_fn(
            move |req: Request<Body>, next: Next| {
                let s = auth_state.clone();
                async move { auth_middleware_inner(s, req, next).await }
            },
        ))
}

async fn body_to_value(resp: Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn req_json(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or_else(Body::empty);
    b.body(body).unwrap()
}

// ---------------------------------------------------------------------------
// identity-provider/instances — 6 tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idp_list_empty_returns_200_and_empty_array() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let resp = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_to_value(resp).await;
    assert_eq!(v, json!([]));
}

#[tokio::test]
async fn idp_create_returns_201_with_location() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let body = json!({
        "alias": "google",
        "providerId": "oidc",
        "enabled": true,
        "config": { "clientId": "abc", "clientSecret": "shh" }
    });
    let resp = app
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            Some(body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        loc,
        "/admin/realms/master/identity-provider/instances/google"
    );
}

#[tokio::test]
async fn idp_create_then_get_roundtrip() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let body = json!({
        "alias": "github",
        "providerId": "oidc",
        "enabled": true
    });
    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            Some(body),
        ))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::CREATED);

    let r2 = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/identity-provider/instances/github",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    let v = body_to_value(r2).await;
    assert_eq!(v["alias"], "github");
    assert_eq!(v["providerId"], "oidc");
}

#[tokio::test]
async fn idp_update_returns_204_and_persists() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    // seed
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            Some(json!({ "alias": "okta", "providerId": "oidc", "enabled": false })),
        ))
        .await
        .unwrap();

    // update
    let r = app
        .clone()
        .oneshot(req_json(
            "PUT",
            "/admin/realms/master/identity-provider/instances/okta",
            Some(&token),
            Some(json!({ "alias": "okta", "providerId": "oidc", "enabled": true, "displayName": "Okta" })),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);

    let r2 = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/identity-provider/instances/okta",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    let v = body_to_value(r2).await;
    assert_eq!(v["enabled"], true);
    assert_eq!(v["displayName"], "Okta");
}

#[tokio::test]
async fn idp_delete_returns_204_then_404() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            Some(json!({ "alias": "azure", "providerId": "oidc" })),
        ))
        .await
        .unwrap();

    let r = app
        .clone()
        .oneshot(req_json(
            "DELETE",
            "/admin/realms/master/identity-provider/instances/azure",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);

    let r2 = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/identity-provider/instances/azure",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn idp_create_duplicate_alias_returns_409() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let body = json!({ "alias": "dup", "providerId": "oidc" });
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            Some(body.clone()),
        ))
        .await
        .unwrap();
    let r = app
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            Some(body),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// authentication/flows — 6 tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flows_list_empty_returns_200() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let r = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/authentication/flows",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v = body_to_value(r).await;
    assert_eq!(v, json!([]));
}

#[tokio::test]
async fn flows_create_returns_201_with_id() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let body = json!({
        "alias": "browser-custom",
        "description": "Custom browser flow",
        "providerId": "basic-flow",
        "topLevel": true,
        "builtIn": false
    });
    let r = app
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/authentication/flows",
            Some(&token),
            Some(body),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let v = body_to_value(r).await;
    assert!(v["id"].as_str().is_some());
}

#[tokio::test]
async fn flows_create_then_get_roundtrip() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/authentication/flows",
            Some(&token),
            Some(json!({
                "alias": "registration-custom",
                "providerId": "basic-flow",
                "topLevel": true,
                "builtIn": false
            })),
        ))
        .await
        .unwrap();
    let id = body_to_value(r1).await["id"].as_str().unwrap().to_string();

    let r2 = app
        .oneshot(req_json(
            "GET",
            &format!("/admin/realms/master/authentication/flows/{id}"),
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    let v = body_to_value(r2).await;
    assert_eq!(v["alias"], "registration-custom");
    assert_eq!(v["topLevel"], true);
}

#[tokio::test]
async fn flows_update_returns_204() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/authentication/flows",
            Some(&token),
            Some(json!({
                "alias": "direct-grant-x",
                "providerId": "basic-flow",
                "topLevel": true,
                "builtIn": false
            })),
        ))
        .await
        .unwrap();
    let id = body_to_value(r1).await["id"].as_str().unwrap().to_string();

    let r2 = app
        .oneshot(req_json(
            "PUT",
            &format!("/admin/realms/master/authentication/flows/{id}"),
            Some(&token),
            Some(json!({
                "id": id,
                "alias": "direct-grant-x",
                "description": "tightened",
                "providerId": "basic-flow",
                "topLevel": true,
                "builtIn": false
            })),
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn flows_delete_returns_204_then_404() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/authentication/flows",
            Some(&token),
            Some(json!({
                "alias": "kill-me",
                "providerId": "basic-flow",
                "topLevel": true,
                "builtIn": false
            })),
        ))
        .await
        .unwrap();
    let id = body_to_value(r1).await["id"].as_str().unwrap().to_string();

    let r2 = app
        .clone()
        .oneshot(req_json(
            "DELETE",
            &format!("/admin/realms/master/authentication/flows/{id}"),
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::NO_CONTENT);

    let r3 = app
        .oneshot(req_json(
            "GET",
            &format!("/admin/realms/master/authentication/flows/{id}"),
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn flows_delete_builtin_returns_400() {
    let app = build_app();
    let token = forge_token(&["platform_admin"]);

    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/admin/realms/master/authentication/flows",
            Some(&token),
            Some(json!({
                "alias": "browser-builtin",
                "providerId": "basic-flow",
                "topLevel": true,
                "builtIn": true
            })),
        ))
        .await
        .unwrap();
    let id = body_to_value(r1).await["id"].as_str().unwrap().to_string();

    let r2 = app
        .oneshot(req_json(
            "DELETE",
            &format!("/admin/realms/master/authentication/flows/{id}"),
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Negative gates — 2 tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_jwt_returns_401() {
    let app = build_app();
    let r = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/identity-provider/instances",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Production-wiring assertions — make sure main.rs actually `.merge()`s the
// admin routers (the in-test `build_app` proves they CAN be assembled; this
// pair proves they ARE assembled in the shipping binary).
// ---------------------------------------------------------------------------

#[test]
fn main_rs_mounts_admin_idp_router() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("read main.rs");
    assert!(
        src.contains("cave_auth::admin_idp::router"),
        "main.rs must .merge(cave_auth::admin_idp::router(...))"
    );
}

#[test]
fn main_rs_mounts_admin_flows_router() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("read main.rs");
    assert!(
        src.contains("cave_auth::admin_flows::router"),
        "main.rs must .merge(cave_auth::admin_flows::router(...))"
    );
}

#[tokio::test]
async fn tenant_admin_jwt_returns_403_on_realms_admin() {
    let app = build_app();
    let token = forge_token(&["tenant_admin"]);
    let r = app
        .oneshot(req_json(
            "GET",
            "/admin/realms/master/identity-provider/instances",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}
