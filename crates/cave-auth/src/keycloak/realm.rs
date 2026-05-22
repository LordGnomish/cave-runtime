// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keycloak Realm Admin CRUD — POST/GET/PUT/DELETE /admin/realms
//!
//! upstream: https://github.com/keycloak/keycloak/blob/v22.0.0/services/src/main/java/org/keycloak/services/resources/admin/RealmsAdminResource.java

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ─── Model ───────────────────────────────────────────────────────────────────

/// A Keycloak realm (== tenant boundary, ADR-MULTI-TENANT-001).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Realm {
    /// Realm name acts as the ID (same as Keycloak).
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    /// "external" | "all" | "none"
    pub ssl_required: String,
    pub registration_allowed: bool,
    pub login_with_email_allowed: bool,
    pub duplicate_emails_allowed: bool,
    /// Access token lifespan in seconds (default 300).
    pub access_token_lifespan: i64,
    /// SSO session idle timeout in seconds (default 1800).
    pub sso_session_idle_timeout: i64,
    pub created_at: DateTime<Utc>,
}

impl Default for Realm {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            enabled: true,
            ssl_required: "external".to_string(),
            registration_allowed: false,
            login_with_email_allowed: true,
            duplicate_emails_allowed: false,
            access_token_lifespan: 300,
            sso_session_idle_timeout: 1800,
            created_at: Utc::now(),
        }
    }
}

/// Request body for creating/updating a realm.
#[derive(Debug, Deserialize)]
pub struct RealmRequest {
    pub id: String,
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub ssl_required: Option<String>,
    pub registration_allowed: Option<bool>,
    pub login_with_email_allowed: Option<bool>,
    pub duplicate_emails_allowed: Option<bool>,
    pub access_token_lifespan: Option<i64>,
    pub sso_session_idle_timeout: Option<i64>,
}

/// Request body for partial realm updates (id not required).
#[derive(Debug, Deserialize)]
pub struct RealmUpdateRequest {
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub ssl_required: Option<String>,
    pub registration_allowed: Option<bool>,
    pub login_with_email_allowed: Option<bool>,
    pub duplicate_emails_allowed: Option<bool>,
    pub access_token_lifespan: Option<i64>,
    pub sso_session_idle_timeout: Option<i64>,
}

// ─── Store ───────────────────────────────────────────────────────────────────

/// In-memory realm store (key = realm id/name).
#[derive(Clone, Default)]
pub struct RealmStore {
    inner: Arc<RwLock<HashMap<String, Realm>>>,
}

impl RealmStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(&self, req: RealmRequest) -> Result<Realm, &'static str> {
        let mut store = self.inner.write().await;
        if store.contains_key(&req.id) {
            return Err("realm_exists");
        }
        let realm = Realm {
            id: req.id.clone(),
            display_name: req.display_name.unwrap_or_else(|| req.id.clone()),
            enabled: req.enabled.unwrap_or(true),
            ssl_required: req.ssl_required.unwrap_or_else(|| "external".to_string()),
            registration_allowed: req.registration_allowed.unwrap_or(false),
            login_with_email_allowed: req.login_with_email_allowed.unwrap_or(true),
            duplicate_emails_allowed: req.duplicate_emails_allowed.unwrap_or(false),
            access_token_lifespan: req.access_token_lifespan.unwrap_or(300),
            sso_session_idle_timeout: req.sso_session_idle_timeout.unwrap_or(1800),
            created_at: Utc::now(),
        };
        store.insert(req.id, realm.clone());
        Ok(realm)
    }

    pub async fn list(&self) -> Vec<Realm> {
        self.inner.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: &str) -> Option<Realm> {
        self.inner.read().await.get(id).cloned()
    }

    pub async fn update(&self, id: &str, req: RealmUpdateRequest) -> Result<Realm, &'static str> {
        let mut store = self.inner.write().await;
        let realm = store.get_mut(id).ok_or("not_found")?;
        if let Some(v) = req.display_name {
            realm.display_name = v;
        }
        if let Some(v) = req.enabled {
            realm.enabled = v;
        }
        if let Some(v) = req.ssl_required {
            realm.ssl_required = v;
        }
        if let Some(v) = req.registration_allowed {
            realm.registration_allowed = v;
        }
        if let Some(v) = req.login_with_email_allowed {
            realm.login_with_email_allowed = v;
        }
        if let Some(v) = req.duplicate_emails_allowed {
            realm.duplicate_emails_allowed = v;
        }
        if let Some(v) = req.access_token_lifespan {
            realm.access_token_lifespan = v;
        }
        if let Some(v) = req.sso_session_idle_timeout {
            realm.sso_session_idle_timeout = v;
        }
        Ok(realm.clone())
    }

    pub async fn delete(&self, id: &str) -> bool {
        self.inner.write().await.remove(id).is_some()
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn create_realm(
    State(store): State<RealmStore>,
    Json(req): Json<RealmRequest>,
) -> impl IntoResponse {
    match store.create(req).await {
        Ok(realm) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(realm).unwrap()),
        )
            .into_response(),
        Err("realm_exists") => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"realm already exists"})),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn list_realms(State(store): State<RealmStore>) -> impl IntoResponse {
    let realms = store.list().await;
    (StatusCode::OK, Json(realms))
}

pub async fn get_realm(
    State(store): State<RealmStore>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    match store.get(&realm).await {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(r).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"realm not found"})),
        )
            .into_response(),
    }
}

pub async fn update_realm(
    State(store): State<RealmStore>,
    Path(realm): Path<String>,
    Json(req): Json<RealmUpdateRequest>,
) -> impl IntoResponse {
    match store.update(&realm, req).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err("not_found") => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"realm not found"})),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn delete_realm(
    State(store): State<RealmStore>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    if store.delete(&realm).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"realm not found"})),
        )
            .into_response()
    }
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn router(store: RealmStore) -> Router {
    Router::new()
        .route("/admin/realms", post(create_realm).get(list_realms))
        .route(
            "/admin/realms/{realm}",
            get(get_realm).put(update_realm).delete(delete_realm),
        )
        .with_state(store)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    fn app() -> Router {
        router(RealmStore::new())
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testCreateRealm
    #[tokio::test]
    async fn test_create_realm_success() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({"id":"myrealm"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = body_json(resp).await;
        assert_eq!(body["id"], "myrealm");
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testCreateRealmConflict
    #[tokio::test]
    async fn test_create_realm_duplicate() {
        let app = app();
        let payload = serde_json::json!({"id":"dup-realm"}).to_string();

        let r1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::CREATED);

        let r2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::CONFLICT);
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testGetRealm
    #[tokio::test]
    async fn test_get_realm_success() {
        let store = RealmStore::new();
        store
            .create(RealmRequest {
                id: "testrealm".to_string(),
                display_name: Some("Test".to_string()),
                enabled: None,
                ssl_required: None,
                registration_allowed: None,
                login_with_email_allowed: None,
                duplicate_emails_allowed: None,
                access_token_lifespan: None,
                sso_session_idle_timeout: None,
            })
            .await
            .unwrap();

        let resp = router(store)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/realms/testrealm")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["id"], "testrealm");
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testGetRealmNotFound
    #[tokio::test]
    async fn test_get_realm_not_found() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/realms/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testUpdateRealm
    #[tokio::test]
    async fn test_update_realm() {
        let store = RealmStore::new();
        store
            .create(RealmRequest {
                id: "updatable".to_string(),
                display_name: None,
                enabled: None,
                ssl_required: None,
                registration_allowed: None,
                login_with_email_allowed: None,
                duplicate_emails_allowed: None,
                access_token_lifespan: None,
                sso_session_idle_timeout: None,
            })
            .await
            .unwrap();
        let app = router(store.clone());

        let put_resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/admin/realms/updatable")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"display_name":"New Name"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_resp.status(), StatusCode::NO_CONTENT);

        // Verify via store directly
        let realm = store.get("updatable").await.unwrap();
        assert_eq!(realm.display_name, "New Name");
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testDeleteRealm
    #[tokio::test]
    async fn test_delete_realm() {
        let store = RealmStore::new();
        store
            .create(RealmRequest {
                id: "todelete".to_string(),
                display_name: None,
                enabled: None,
                ssl_required: None,
                registration_allowed: None,
                login_with_email_allowed: None,
                duplicate_emails_allowed: None,
                access_token_lifespan: None,
                sso_session_idle_timeout: None,
            })
            .await
            .unwrap();
        let app = router(store.clone());

        let del_resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/admin/realms/todelete")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);
        assert!(store.get("todelete").await.is_none());
    }

    // upstream: keycloak/keycloak RealmsAdminResourceTest.java:testListRealms
    #[tokio::test]
    async fn test_list_realms() {
        let store = RealmStore::new();
        for name in &["realm-alpha", "realm-beta"] {
            store
                .create(RealmRequest {
                    id: name.to_string(),
                    display_name: None,
                    enabled: None,
                    ssl_required: None,
                    registration_allowed: None,
                    login_with_email_allowed: None,
                    duplicate_emails_allowed: None,
                    access_token_lifespan: None,
                    sso_session_idle_timeout: None,
                })
                .await
                .unwrap();
        }

        let resp = router(store)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/realms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body.as_array().unwrap().len(), 2);
    }
}
