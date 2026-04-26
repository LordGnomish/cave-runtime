//! Keycloak User Admin CRUD — POST/GET/PUT/DELETE /admin/realms/{realm}/users
//!
//! upstream: https://github.com/keycloak/keycloak/blob/v22.0.0/services/src/main/java/org/keycloak/services/resources/admin/UsersResource.java

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::keycloak::realm::RealmStore;

// ─── Model ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeycloakUser {
    pub id: Uuid,
    pub realm_id: String,
    pub username: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub enabled: bool,
    pub attributes: HashMap<String, Vec<String>>,
    /// Unix timestamp in milliseconds.
    pub created_timestamp: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub enabled: Option<bool>,
    pub attributes: Option<HashMap<String, Vec<String>>>,
    /// Hashed or plaintext password (stored as-is for this in-memory mock).
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub enabled: Option<bool>,
    pub attributes: Option<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SearchQuery {
    pub search: Option<String>,
}

// ─── Store ───────────────────────────────────────────────────────────────────

/// Key: (realm_id, user_uuid). Also stores password for password-grant testing.
#[derive(Clone, Default)]
pub struct UserStore {
    inner: Arc<RwLock<HashMap<(String, Uuid), KeycloakUser>>>,
    /// Key: (realm_id, user_uuid) → hashed/plaintext password.
    passwords: Arc<RwLock<HashMap<(String, Uuid), String>>>,
}

impl UserStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(
        &self,
        realm_id: &str,
        req: CreateUserRequest,
    ) -> Result<KeycloakUser, &'static str> {
        let mut store = self.inner.write().await;
        let dup = store
            .values()
            .any(|u| u.realm_id == realm_id && u.username == req.username);
        if dup {
            return Err("username_exists");
        }
        let id = Uuid::new_v4();
        let user = KeycloakUser {
            id,
            realm_id: realm_id.to_string(),
            username: req.username,
            email: req.email,
            email_verified: req.email_verified.unwrap_or(false),
            first_name: req.first_name,
            last_name: req.last_name,
            enabled: req.enabled.unwrap_or(true),
            attributes: req.attributes.unwrap_or_default(),
            created_timestamp: chrono::Utc::now().timestamp_millis(),
        };
        store.insert((realm_id.to_string(), id), user.clone());
        drop(store);

        if let Some(pw) = req.password {
            self.passwords
                .write()
                .await
                .insert((realm_id.to_string(), id), pw);
        }
        Ok(user)
    }

    pub async fn list(&self, realm_id: &str, search: Option<&str>) -> Vec<KeycloakUser> {
        let store = self.inner.read().await;
        store
            .values()
            .filter(|u| {
                if u.realm_id != realm_id {
                    return false;
                }
                if let Some(q) = search {
                    if q.is_empty() {
                        return true;
                    }
                    let q_lower = q.to_lowercase();
                    let match_username = u.username.to_lowercase().contains(&q_lower);
                    let match_email = u
                        .email
                        .as_deref()
                        .map(|e| e.to_lowercase().contains(&q_lower))
                        .unwrap_or(false);
                    return match_username || match_email;
                }
                true
            })
            .cloned()
            .collect()
    }

    /// Get by UUID — only within the given realm.
    pub async fn get(&self, realm_id: &str, id: Uuid) -> Option<KeycloakUser> {
        self.inner
            .read()
            .await
            .get(&(realm_id.to_string(), id))
            .cloned()
    }

    pub async fn get_by_username(&self, realm_id: &str, username: &str) -> Option<KeycloakUser> {
        self.inner
            .read()
            .await
            .values()
            .find(|u| u.realm_id == realm_id && u.username == username)
            .cloned()
    }

    pub async fn verify_password(&self, realm_id: &str, user_id: Uuid, password: &str) -> bool {
        self.passwords
            .read()
            .await
            .get(&(realm_id.to_string(), user_id))
            .map(|p| p == password)
            .unwrap_or(false)
    }

    pub async fn update(
        &self,
        realm_id: &str,
        id: Uuid,
        req: UpdateUserRequest,
    ) -> Result<KeycloakUser, &'static str> {
        let mut store = self.inner.write().await;
        let user = store
            .get_mut(&(realm_id.to_string(), id))
            .ok_or("not_found")?;
        if let Some(v) = req.email { user.email = Some(v); }
        if let Some(v) = req.email_verified { user.email_verified = v; }
        if let Some(v) = req.first_name { user.first_name = Some(v); }
        if let Some(v) = req.last_name { user.last_name = Some(v); }
        if let Some(v) = req.enabled { user.enabled = v; }
        if let Some(v) = req.attributes { user.attributes = v; }
        Ok(user.clone())
    }

    pub async fn delete(&self, realm_id: &str, id: Uuid) -> bool {
        let removed = self
            .inner
            .write()
            .await
            .remove(&(realm_id.to_string(), id))
            .is_some();
        if removed {
            self.passwords
                .write()
                .await
                .remove(&(realm_id.to_string(), id));
        }
        removed
    }
}

// ─── App state ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct UserAppState {
    pub users: UserStore,
    pub realms: RealmStore,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn create_user(
    State(state): State<UserAppState>,
    Path(realm): Path<String>,
    Json(req): Json<CreateUserRequest>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    match state.users.create(&realm, req).await {
        Ok(u) => {
            let location = format!("/admin/realms/{}/users/{}", realm, u.id);
            (
                StatusCode::CREATED,
                [(header::LOCATION, location)],
                Json(serde_json::to_value(&u).unwrap()),
            )
                .into_response()
        }
        Err("username_exists") => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"username already exists in realm"})),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn list_users(
    State(state): State<UserAppState>,
    Path(realm): Path<String>,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    let users = state.users.list(&realm, q.search.as_deref()).await;
    (StatusCode::OK, Json(users)).into_response()
}

pub async fn get_user(
    State(state): State<UserAppState>,
    Path((realm, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    match state.users.get(&realm, id).await {
        Some(u) => (StatusCode::OK, Json(serde_json::to_value(u).unwrap())).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"user not found"}))).into_response(),
    }
}

pub async fn update_user(
    State(state): State<UserAppState>,
    Path((realm, id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateUserRequest>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    match state.users.update(&realm, id, req).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err("not_found") => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"user not found"}))).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn delete_user(
    State(state): State<UserAppState>,
    Path((realm, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm not found"}))).into_response();
    }
    if state.users.delete(&realm, id).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"user not found"}))).into_response()
    }
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn router(users: UserStore, realms: RealmStore) -> Router {
    let state = UserAppState { users, realms };
    Router::new()
        .route("/admin/realms/{realm}/users", post(create_user).get(list_users))
        .route("/admin/realms/{realm}/users/{id}", get(get_user).put(update_user).delete(delete_user))
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

    async fn setup() -> (RealmStore, UserStore) {
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
        (realms, UserStore::new())
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn user_payload(username: &str) -> String {
        serde_json::json!({
            "username": username,
            "email": format!("{}@example.com", username),
            "password": "secret123"
        })
        .to_string()
    }

    // upstream: keycloak/keycloak UserTest.java:testCreateUser
    #[tokio::test]
    async fn test_create_user_success() {
        let (realms, users) = setup().await;
        let resp = router(users, realms)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/realms/testrealm/users")
                    .header("content-type", "application/json")
                    .body(Body::from(user_payload("alice")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        // Location header must contain user UUID
        let location = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(location.contains("/admin/realms/testrealm/users/"));
    }

    // upstream: keycloak/keycloak UserTest.java:testCreateUserDuplicateUsername
    #[tokio::test]
    async fn test_create_user_duplicate_username() {
        let (realms, users) = setup().await;
        let app = router(users, realms);
        let payload = user_payload("bob");

        let r1 = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/testrealm/users").header("content-type", "application/json").body(Body::from(payload.clone())).unwrap()).await.unwrap();
        assert_eq!(r1.status(), StatusCode::CREATED);

        let r2 = app.oneshot(Request::builder().method("POST").uri("/admin/realms/testrealm/users").header("content-type", "application/json").body(Body::from(payload)).unwrap()).await.unwrap();
        assert_eq!(r2.status(), StatusCode::CONFLICT);
    }

    // upstream: keycloak/keycloak UserTest.java:testGetUserById
    #[tokio::test]
    async fn test_get_user_by_id() {
        let (realms, users) = setup().await;
        let user = users.create("testrealm", CreateUserRequest { username: "charlie".to_string(), email: None, email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();
        let resp = router(users, realms)
            .oneshot(Request::builder().method("GET").uri(format!("/admin/realms/testrealm/users/{}", user.id)).body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["username"], "charlie");
    }

    // upstream: keycloak/keycloak UserTest.java:testGetUserCrossRealmDenied
    #[tokio::test]
    async fn test_get_user_cross_realm_denied() {
        let realms = RealmStore::new();
        for r in &["realm-a", "realm-b"] {
            realms.create(RealmRequest { id: r.to_string(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        }
        let users = UserStore::new();
        let user = users.create("realm-a", CreateUserRequest { username: "xuser".to_string(), email: None, email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();

        let resp = router(users, realms)
            .oneshot(Request::builder().method("GET").uri(format!("/admin/realms/realm-b/users/{}", user.id)).body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak UserTest.java:testUpdateUserEmail
    #[tokio::test]
    async fn test_update_user_email() {
        let (realms, users) = setup().await;
        let user = users.create("testrealm", CreateUserRequest { username: "dave".to_string(), email: Some("old@example.com".to_string()), email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();

        let put_resp = router(users.clone(), realms.clone())
            .oneshot(Request::builder().method("PUT").uri(format!("/admin/realms/testrealm/users/{}", user.id)).header("content-type", "application/json").body(Body::from(serde_json::json!({"email":"new@example.com"}).to_string())).unwrap())
            .await.unwrap();
        assert_eq!(put_resp.status(), StatusCode::NO_CONTENT);
        let updated = users.get("testrealm", user.id).await.unwrap();
        assert_eq!(updated.email.as_deref(), Some("new@example.com"));
    }

    // upstream: keycloak/keycloak UserTest.java:testDeleteUser
    #[tokio::test]
    async fn test_delete_user() {
        let (realms, users) = setup().await;
        let user = users.create("testrealm", CreateUserRequest { username: "eve".to_string(), email: None, email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();

        let del_resp = router(users.clone(), realms.clone())
            .oneshot(Request::builder().method("DELETE").uri(format!("/admin/realms/testrealm/users/{}", user.id)).body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);
        assert!(users.get("testrealm", user.id).await.is_none());
    }

    // upstream: keycloak/keycloak UserSearchTest.java:testSearchByUsername
    #[tokio::test]
    async fn test_search_users_by_username() {
        let (realms, users) = setup().await;
        for (name, email) in &[
            ("john", "john@acme.com"),
            ("johanna", "johanna@acme.com"),
            ("alice", "alice@acme.com"),
        ] {
            users.create("testrealm", CreateUserRequest { username: name.to_string(), email: Some(email.to_string()), email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();
        }

        let resp = router(users, realms)
            .oneshot(Request::builder().method("GET").uri("/admin/realms/testrealm/users?search=joh").body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let names: Vec<&str> = arr.iter().map(|u| u["username"].as_str().unwrap()).collect();
        assert!(names.contains(&"john"));
        assert!(names.contains(&"johanna"));
        assert!(!names.contains(&"alice"));
    }

    // upstream: keycloak/keycloak UserSearchTest.java:testSearchByEmail
    #[tokio::test]
    async fn test_search_users_by_email() {
        let (realms, users) = setup().await;
        users.create("testrealm", CreateUserRequest { username: "user1".to_string(), email: Some("user1@example.com".to_string()), email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();
        users.create("testrealm", CreateUserRequest { username: "user2".to_string(), email: Some("user2@example.com".to_string()), email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();
        users.create("testrealm", CreateUserRequest { username: "user3".to_string(), email: Some("user3@other.org".to_string()), email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();

        let resp = router(users, realms)
            .oneshot(Request::builder().method("GET").uri("/admin/realms/testrealm/users?search=@example").body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body.as_array().unwrap().len(), 2);
    }

    // upstream: keycloak/keycloak UserSearchTest.java:testSearchEmptyQuery
    #[tokio::test]
    async fn test_search_users_empty_query() {
        let (realms, users) = setup().await;
        for name in &["u1", "u2", "u3"] {
            users.create("testrealm", CreateUserRequest { username: name.to_string(), email: None, email_verified: None, first_name: None, last_name: None, enabled: None, attributes: None, password: None }).await.unwrap();
        }

        let resp = router(users, realms)
            .oneshot(Request::builder().method("GET").uri("/admin/realms/testrealm/users?search=").body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body.as_array().unwrap().len(), 3);
    }
}
