// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keycloak Client Admin CRUD — POST/GET/PUT/DELETE /admin/realms/{realm}/clients
//!
//! upstream: https://github.com/keycloak/keycloak/blob/v22.0.0/services/src/main/java/org/keycloak/services/resources/admin/ClientsResource.java

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::keycloak::realm::RealmStore;

// ─── Model ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeycloakClient {
    pub id: Uuid,
    pub realm_id: String,
    /// Human-readable, unique within the realm.
    pub client_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub public_client: bool,
    /// Only for confidential clients.
    pub secret: Option<String>,
    pub redirect_uris: Vec<String>,
    pub web_origins: Vec<String>,
    pub protocol: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateClientRequest {
    pub client_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub public_client: Option<bool>,
    pub secret: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    pub web_origins: Option<Vec<String>>,
    pub protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClientRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub public_client: Option<bool>,
    pub secret: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    pub web_origins: Option<Vec<String>>,
}

// ─── Store ───────────────────────────────────────────────────────────────────

/// Key: (realm_id, client_uuid).
#[derive(Clone, Default)]
pub struct ClientStore {
    inner: Arc<RwLock<HashMap<(String, Uuid), KeycloakClient>>>,
}

impl ClientStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(
        &self,
        realm_id: &str,
        req: CreateClientRequest,
    ) -> Result<KeycloakClient, &'static str> {
        let mut store = self.inner.write().await;
        // Uniqueness: client_id within realm
        let dup = store
            .values()
            .any(|c| c.realm_id == realm_id && c.client_id == req.client_id);
        if dup {
            return Err("client_id_exists");
        }
        let client = KeycloakClient {
            id: Uuid::new_v4(),
            realm_id: realm_id.to_string(),
            client_id: req.client_id,
            name: req.name,
            description: req.description,
            enabled: req.enabled.unwrap_or(true),
            public_client: req.public_client.unwrap_or(false),
            secret: req.secret,
            redirect_uris: req.redirect_uris.unwrap_or_default(),
            web_origins: req.web_origins.unwrap_or_default(),
            protocol: req.protocol.unwrap_or_else(|| "openid-connect".to_string()),
            created_at: Utc::now(),
        };
        store.insert((realm_id.to_string(), client.id), client.clone());
        Ok(client)
    }

    pub async fn list_in_realm(&self, realm_id: &str) -> Vec<KeycloakClient> {
        self.inner
            .read()
            .await
            .values()
            .filter(|c| c.realm_id == realm_id)
            .cloned()
            .collect()
    }

    /// Get by UUID — only within the given realm (cross-realm isolation).
    pub async fn get(&self, realm_id: &str, id: Uuid) -> Option<KeycloakClient> {
        self.inner.read().await.get(&(realm_id.to_string(), id)).cloned()
    }

    pub async fn update(
        &self,
        realm_id: &str,
        id: Uuid,
        req: UpdateClientRequest,
    ) -> Result<KeycloakClient, &'static str> {
        let mut store = self.inner.write().await;
        let client = store
            .get_mut(&(realm_id.to_string(), id))
            .ok_or("not_found")?;
        if let Some(v) = req.name { client.name = Some(v); }
        if let Some(v) = req.description { client.description = Some(v); }
        if let Some(v) = req.enabled { client.enabled = v; }
        if let Some(v) = req.public_client { client.public_client = v; }
        if let Some(v) = req.secret { client.secret = Some(v); }
        if let Some(v) = req.redirect_uris { client.redirect_uris = v; }
        if let Some(v) = req.web_origins { client.web_origins = v; }
        Ok(client.clone())
    }

    pub async fn delete(&self, realm_id: &str, id: Uuid) -> bool {
        self.inner
            .write()
            .await
            .remove(&(realm_id.to_string(), id))
            .is_some()
    }

    /// Look up a client by client_id string within a realm.
    pub async fn get_by_client_id(&self, realm_id: &str, client_id: &str) -> Option<KeycloakClient> {
        self.inner
            .read()
            .await
            .values()
            .find(|c| c.realm_id == realm_id && c.client_id == client_id)
            .cloned()
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ClientAppState {
    pub clients: ClientStore,
    pub realms: RealmStore,
}

pub async fn create_client(
    State(state): State<ClientAppState>,
    Path(realm): Path<String>,
    Json(req): Json<CreateClientRequest>,
) -> impl IntoResponse {
    // Realm must exist
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    match state.clients.create(&realm, req).await {
        Ok(c) => (StatusCode::CREATED, Json(serde_json::to_value(c).unwrap())).into_response(),
        Err("client_id_exists") => (StatusCode::CONFLICT, Json(serde_json::json!({"error":"client_id already exists in realm"}))).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn list_clients(
    State(state): State<ClientAppState>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    let clients = state.clients.list_in_realm(&realm).await;
    (StatusCode::OK, Json(clients)).into_response()
}

pub async fn get_client(
    State(state): State<ClientAppState>,
    Path((realm, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    match state.clients.get(&realm, id).await {
        Some(c) => (StatusCode::OK, Json(serde_json::to_value(c).unwrap())).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"client not found"}))).into_response(),
    }
}

pub async fn update_client(
    State(state): State<ClientAppState>,
    Path((realm, id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateClientRequest>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    match state.clients.update(&realm, id, req).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err("not_found") => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"client not found"}))).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn delete_client(
    State(state): State<ClientAppState>,
    Path((realm, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    if state.clients.delete(&realm, id).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"client not found"}))).into_response()
    }
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn router(clients: ClientStore, realms: RealmStore) -> Router {
    let state = ClientAppState { clients, realms };
    Router::new()
        .route("/admin/realms/{realm}/clients", post(create_client).get(list_clients))
        .route("/admin/realms/{realm}/clients/{id}", get(get_client).put(update_client).delete(delete_client))
        .with_state(state)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::realm::{RealmRequest, RealmStore};
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    async fn setup() -> (Router, RealmStore, ClientStore) {
        let realms = RealmStore::new();
        realms
            .create(RealmRequest {
                id: "testrealm".to_string(),
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
        let clients = ClientStore::new();
        let app = router(clients.clone(), realms.clone());
        (app, realms, clients)
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn client_payload(client_id: &str) -> String {
        serde_json::json!({
            "client_id": client_id,
            "public_client": false,
            "secret": "s3cr3t"
        })
        .to_string()
    }

    // upstream: keycloak/keycloak ClientTest.java:testCreateClient
    #[tokio::test]
    async fn test_create_client_success() {
        let (app, _, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms/testrealm/clients")
                    .header("content-type", "application/json")
                    .body(Body::from(client_payload("myapp")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = body_json(resp).await;
        assert_eq!(body["client_id"], "myapp");
        assert!(body["id"].is_string());
    }

    // upstream: keycloak/keycloak ClientTest.java:testCreateClientDuplicateClientId
    #[tokio::test]
    async fn test_create_client_duplicate_client_id() {
        let (app, _, _) = setup().await;
        let payload = client_payload("dup-client");
        let r1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms/testrealm/clients")
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
                    .uri("/admin/realms/testrealm/clients")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::CONFLICT);
    }

    // upstream: keycloak/keycloak ClientTest.java:testCreateClientWrongRealm
    #[tokio::test]
    async fn test_create_client_wrong_realm() {
        let (app, _, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms/nonexistent/clients")
                    .header("content-type", "application/json")
                    .body(Body::from(client_payload("x")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak ClientTest.java:testGetClient
    #[tokio::test]
    async fn test_get_client() {
        let (_, realms, clients) = setup().await;
        let client = clients
            .create(
                "testrealm",
                CreateClientRequest {
                    client_id: "fetch-me".to_string(),
                    name: None,
                    description: None,
                    enabled: None,
                    public_client: Some(false),
                    secret: Some("sec".to_string()),
                    redirect_uris: None,
                    web_origins: None,
                    protocol: None,
                },
            )
            .await
            .unwrap();

        let resp = router(clients, realms)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/admin/realms/testrealm/clients/{}", client.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["client_id"], "fetch-me");
    }

    // upstream: keycloak/keycloak ClientTest.java:testGetClientCrossRealmDenied
    #[tokio::test]
    async fn test_get_client_cross_realm_denied() {
        let realms = RealmStore::new();
        for r in &["realm-a", "realm-b"] {
            realms.create(RealmRequest { id: r.to_string(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        }
        let clients = ClientStore::new();
        let client = clients
            .create("realm-a", CreateClientRequest { client_id: "realm-a-client".to_string(), name: None, description: None, enabled: None, public_client: None, secret: None, redirect_uris: None, web_origins: None, protocol: None })
            .await
            .unwrap();

        // Try to access realm-a's client via realm-b path → 404 (cross-realm denied)
        let resp = router(clients, realms)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/admin/realms/realm-b/clients/{}", client.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak ClientTest.java:testUpdateClient
    #[tokio::test]
    async fn test_update_client() {
        let (_, realms, clients) = setup().await;
        let client = clients
            .create("testrealm", CreateClientRequest { client_id: "updateable".to_string(), name: None, description: None, enabled: None, public_client: None, secret: None, redirect_uris: None, web_origins: None, protocol: None })
            .await
            .unwrap();

        let put_resp = router(clients.clone(), realms.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/admin/realms/testrealm/clients/{}", client.id))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({"name":"Updated Name"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_resp.status(), StatusCode::NO_CONTENT);
        let updated = clients.get("testrealm", client.id).await.unwrap();
        assert_eq!(updated.name.as_deref(), Some("Updated Name"));
    }

    // upstream: keycloak/keycloak ClientTest.java:testDeleteClient
    #[tokio::test]
    async fn test_delete_client() {
        let (_, realms, clients) = setup().await;
        let client = clients
            .create("testrealm", CreateClientRequest { client_id: "bye".to_string(), name: None, description: None, enabled: None, public_client: None, secret: None, redirect_uris: None, web_origins: None, protocol: None })
            .await
            .unwrap();

        let del_resp = router(clients.clone(), realms.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/admin/realms/testrealm/clients/{}", client.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);
        assert!(clients.get("testrealm", client.id).await.is_none());
    }

    // upstream: keycloak/keycloak ClientTest.java:testListClientsInRealm
    #[tokio::test]
    async fn test_list_clients_in_realm() {
        let (_, realms, clients) = setup().await;
        for name in &["c1", "c2", "c3"] {
            clients.create("testrealm", CreateClientRequest { client_id: name.to_string(), name: None, description: None, enabled: None, public_client: None, secret: None, redirect_uris: None, web_origins: None, protocol: None }).await.unwrap();
        }

        let resp = router(clients, realms)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/realms/testrealm/clients")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body.as_array().unwrap().len(), 3);
    }
}
