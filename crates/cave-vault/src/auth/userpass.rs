// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::token::CreateTokenParams;
use crate::VaultState;
use axum::{
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
    Router,
};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

fn hash_password(password: &str) -> String {
    let digest = digest::digest(&digest::SHA256, password.as_bytes());
    hex::encode(digest.as_ref())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserpassEntry {
    pub username: String,
    pub password_hash: String,
    pub policies: Vec<String>,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
    pub token_bound_cidrs: Vec<String>,
}

#[derive(Default)]
pub struct UserpassStore {
    pub users: HashMap<String, UserpassEntry>,
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub password: String,
    pub policies: Option<Vec<String>>,
    pub token_ttl: Option<String>,
    pub token_max_ttl: Option<String>,
    pub token_bound_cidrs: Option<Vec<String>>,
}

pub async fn create_user(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(username): Path<String>,
    Json(body): Json<CreateUserRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.userpass_store.write().await;
    let entry = UserpassEntry {
        username: username.clone(),
        password_hash: hash_password(&body.password),
        policies: body.policies.unwrap_or_else(|| vec!["default".to_string()]),
        token_ttl: body.token_ttl.as_deref()
            .map(crate::token::parse_duration).unwrap_or(3600),
        token_max_ttl: body.token_max_ttl.as_deref()
            .map(crate::token::parse_duration).unwrap_or(0),
        token_bound_cidrs: body.token_bound_cidrs.unwrap_or_default(),
    };
    store.users.insert(username, entry);
    Ok(VaultResponse::new())
}

pub async fn read_user(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.userpass_store.read().await;
    let user = store.users.get(&username)
        .ok_or_else(|| VaultError::NotFound(format!("user {} not found", username)))?;
    Ok(VaultResponse::new().with_data(json!({
        "username": user.username,
        "policies": user.policies,
        "token_ttl": user.token_ttl,
        "token_max_ttl": user.token_max_ttl,
    })))
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub password: Option<String>,
    pub policies: Option<Vec<String>>,
    pub token_ttl: Option<String>,
}

pub async fn update_user(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(username): Path<String>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.userpass_store.write().await;
    let user = store.users.get_mut(&username)
        .ok_or_else(|| VaultError::NotFound(format!("user {} not found", username)))?;
    if let Some(pw) = body.password { user.password_hash = hash_password(&pw); }
    if let Some(p) = body.policies { user.policies = p; }
    if let Some(ttl) = body.token_ttl { user.token_ttl = crate::token::parse_duration(&ttl); }
    Ok(VaultResponse::new())
}

pub async fn delete_user(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.userpass_store.write().await;
    store.users.remove(&username);
    Ok(VaultResponse::new())
}

pub async fn list_users(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.userpass_store.read().await;
    let keys: Vec<String> = store.users.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<VaultState>>,
    Path(username): Path<String>,
    Json(body): Json<LoginRequest>,
) -> Result<VaultResponse, VaultError> {
    let store = state.userpass_store.read().await;
    let user = store.users.get(&username)
        .ok_or_else(|| VaultError::Auth("invalid credentials".into()))?;

    let provided_hash = hash_password(&body.password);
    if provided_hash != user.password_hash {
        return Err(VaultError::Auth("invalid credentials".into()));
    }
    let policies = user.policies.clone();
    let ttl = user.token_ttl;
    drop(store);

    let params = CreateTokenParams {
        policies: Some(policies),
        ttl: Some(format!("{}s", ttl)),
        renewable: Some(true),
        no_parent: Some(true),
        metadata: Some({
            let mut m = HashMap::new();
            m.insert("username".to_string(), username.clone());
            m
        }),
        display_name: Some(format!("userpass-{}", username)),
        ..Default::default()
    };
    let mut ts = state.token_store.write().await;
    let token = ts.create(&params, None)?;
    Ok(VaultResponse::new().with_auth(token.to_auth_info()))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route("/v1/auth/userpass/users", get(list_users))
        .route("/v1/auth/userpass/users/{username}", post(create_user).get(read_user).delete(delete_user))
        .route("/v1/auth/userpass/users/{username}/password", post(update_user))
        .route("/v1/auth/userpass/login/{username}", post(login))
        .with_state(state)
}
