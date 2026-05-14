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
use chrono::{DateTime, Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproleRole {
    pub role_name: String,
    pub role_id: String,
    pub bind_secret_id: bool,
    pub secret_id_ttl: i64,
    pub secret_id_num_uses: i64,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
    pub token_policies: Vec<String>,
    pub token_bound_cidrs: Vec<String>,
    pub period: i64,
    pub enable_local_secret_ids: bool,
}

impl Default for ApproleRole {
    fn default() -> Self {
        Self {
            role_name: String::new(),
            role_id: Uuid::new_v4().to_string(),
            bind_secret_id: true,
            secret_id_ttl: 0,
            secret_id_num_uses: 0,
            token_ttl: 3600,
            token_max_ttl: 0,
            token_policies: vec!["default".to_string()],
            token_bound_cidrs: Vec::new(),
            period: 0,
            enable_local_secret_ids: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretIdEntry {
    pub secret_id: String,
    pub accessor: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub num_uses: i64,
    pub uses_remaining: i64,
    pub metadata: HashMap<String, String>,
    pub cidr_list: Vec<String>,
}

#[derive(Default)]
pub struct ApproleStore {
    pub roles: HashMap<String, ApproleRole>,
    pub secret_ids: HashMap<String, HashMap<String, SecretIdEntry>>, // role_name -> accessor -> entry
    pub secret_id_by_id: HashMap<String, (String, String)>, // secret_id -> (role_name, accessor)
}

#[derive(Deserialize)]
pub struct CreateRoleRequest {
    pub bind_secret_id: Option<bool>,
    pub secret_id_ttl: Option<String>,
    pub secret_id_num_uses: Option<i64>,
    pub token_ttl: Option<String>,
    pub token_max_ttl: Option<String>,
    pub token_policies: Option<Vec<String>>,
    pub token_bound_cidrs: Option<Vec<String>>,
    pub period: Option<String>,
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<CreateRoleRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.approle_store.write().await;
    let role = store.roles.entry(role_name.clone()).or_insert_with(|| ApproleRole {
        role_name: role_name.clone(),
        ..Default::default()
    });
    if let Some(b) = body.bind_secret_id { role.bind_secret_id = b; }
    if let Some(ttl) = body.secret_id_ttl.as_deref() { role.secret_id_ttl = crate::token::parse_duration(ttl); }
    if let Some(n) = body.secret_id_num_uses { role.secret_id_num_uses = n; }
    if let Some(ttl) = body.token_ttl.as_deref() { role.token_ttl = crate::token::parse_duration(ttl); }
    if let Some(ttl) = body.token_max_ttl.as_deref() { role.token_max_ttl = crate::token::parse_duration(ttl); }
    if let Some(p) = body.token_policies { role.token_policies = p; }
    if let Some(c) = body.token_bound_cidrs { role.token_bound_cidrs = c; }
    if let Some(period) = body.period.as_deref() { role.period = crate::token::parse_duration(period); }
    Ok(VaultResponse::new())
}

pub async fn read_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.approle_store.read().await;
    let role = store.roles.get(&role_name).ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(role).unwrap_or_default()))
}

pub async fn delete_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.approle_store.write().await;
    store.roles.remove(&role_name);
    store.secret_ids.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.approle_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn read_role_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.approle_store.read().await;
    let role = store.roles.get(&role_name).ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(json!({ "role_id": role.role_id })))
}

#[derive(Deserialize)]
pub struct UpdateRoleIdRequest {
    pub role_id: String,
}

pub async fn update_role_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<UpdateRoleIdRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.approle_store.write().await;
    let role = store.roles.get_mut(&role_name).ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    role.role_id = body.role_id;
    Ok(VaultResponse::new())
}

#[derive(Deserialize, Default)]
pub struct GenerateSecretIdRequest {
    pub metadata: Option<HashMap<String, String>>,
    pub cidr_list: Option<Vec<String>>,
    pub ttl: Option<String>,
    pub num_uses: Option<i64>,
}

pub async fn generate_secret_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<GenerateSecretIdRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.approle_store.write().await;
    let role = store.roles.get(&role_name).ok_or_else(|| VaultError::RoleNotFound(role_name.clone()))?.clone();

    let rng = SystemRandom::new();
    let mut sid_bytes = vec![0u8; 16];
    rng.fill(&mut sid_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    let secret_id = hex::encode(&sid_bytes);
    let accessor = Uuid::new_v4().to_string();

    let ttl_secs = body.ttl.as_deref()
        .map(crate::token::parse_duration)
        .filter(|&t| t > 0)
        .unwrap_or(role.secret_id_ttl);

    let num_uses = body.num_uses.unwrap_or(role.secret_id_num_uses);

    let entry = SecretIdEntry {
        secret_id: secret_id.clone(),
        accessor: accessor.clone(),
        created_at: Utc::now(),
        expires_at: if ttl_secs > 0 { Some(Utc::now() + Duration::seconds(ttl_secs)) } else { None },
        num_uses,
        uses_remaining: num_uses,
        metadata: body.metadata.unwrap_or_default(),
        cidr_list: body.cidr_list.unwrap_or_default(),
    };

    store.secret_id_by_id.insert(secret_id.clone(), (role_name.clone(), accessor.clone()));
    store.secret_ids.entry(role_name).or_default().insert(accessor.clone(), entry);

    Ok(VaultResponse::new().with_data(json!({
        "secret_id": secret_id,
        "secret_id_accessor": accessor,
        "secret_id_ttl": ttl_secs,
        "secret_id_num_uses": num_uses,
    })))
}

pub async fn lookup_secret_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let secret_id = body.get("secret_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("secret_id required".into()))?
        .to_string();

    let store = state.approle_store.read().await;
    let (rn, accessor) = store.secret_id_by_id.get(&secret_id)
        .ok_or(VaultError::NotFound("secret id not found".into()))?
        .clone();
    if rn != role_name {
        return Err(VaultError::NotFound("secret id not found".into()));
    }
    let entry = store.secret_ids.get(&rn)
        .and_then(|m| m.get(&accessor))
        .ok_or(VaultError::NotFound("secret id not found".into()))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(entry).unwrap_or_default()))
}

pub async fn destroy_secret_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let secret_id = body.get("secret_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("secret_id required".into()))?
        .to_string();

    let mut store = state.approle_store.write().await;
    if let Some((rn, accessor)) = store.secret_id_by_id.remove(&secret_id) {
        if rn == role_name {
            store.secret_ids.entry(rn).or_default().remove(&accessor);
        }
    }
    Ok(VaultResponse::new())
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub role_id: String,
    pub secret_id: Option<String>,
}

pub async fn login(
    State(state): State<Arc<VaultState>>,
    Json(body): Json<LoginRequest>,
) -> Result<VaultResponse, VaultError> {
    let mut store = state.approle_store.write().await;
    // Find role by role_id
    let role = store.roles.values()
        .find(|r| r.role_id == body.role_id)
        .cloned()
        .ok_or_else(|| VaultError::Auth("invalid role_id".into()))?;

    if role.bind_secret_id {
        let secret_id = body.secret_id.ok_or_else(|| VaultError::Auth("secret_id required".into()))?;
        let (rn, accessor) = store.secret_id_by_id.get(&secret_id)
            .ok_or_else(|| VaultError::Auth("invalid secret_id".into()))?.clone();

        if rn != role.role_name {
            return Err(VaultError::Auth("invalid secret_id".into()));
        }

        let entry = store.secret_ids.get_mut(&rn)
            .and_then(|m| m.get_mut(&accessor))
            .ok_or_else(|| VaultError::Auth("invalid secret_id".into()))?;

        if let Some(exp) = entry.expires_at {
            if Utc::now() > exp {
                return Err(VaultError::Auth("secret_id expired".into()));
            }
        }
        if entry.num_uses > 0 {
            if entry.uses_remaining <= 0 {
                return Err(VaultError::Auth("secret_id use limit exceeded".into()));
            }
            entry.uses_remaining -= 1;
        }
    }

    drop(store);

    let params = CreateTokenParams {
        policies: Some(role.token_policies.clone()),
        ttl: Some(format!("{}s", role.token_ttl)),
        renewable: Some(true),
        no_parent: Some(true),
        metadata: Some({
            let mut m = HashMap::new();
            m.insert("role_name".to_string(), role.role_name.clone());
            m
        }),
        ..Default::default()
    };
    let mut ts = state.token_store.write().await;
    let token = ts.create(&params, None)?;
    Ok(VaultResponse::new().with_auth(token.to_auth_info()))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route("/v1/auth/approle/role", get(list_roles))
        .route("/v1/auth/approle/role/{role_name}", post(create_role).get(read_role).delete(delete_role))
        .route("/v1/auth/approle/role/{role_name}/role-id", get(read_role_id).post(update_role_id))
        .route("/v1/auth/approle/role/{role_name}/secret-id", post(generate_secret_id))
        .route("/v1/auth/approle/role/{role_name}/secret-id/lookup", post(lookup_secret_id))
        .route("/v1/auth/approle/role/{role_name}/secret-id/destroy", post(destroy_secret_id))
        .route("/v1/auth/approle/login", post(login))
        .with_state(state)
}
