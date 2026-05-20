// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::VaultState;
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::token::CreateTokenParams;
use axum::{
    Router,
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers
        .get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KubernetesConfig {
    pub kubernetes_host: String,
    pub kubernetes_ca_cert: String,
    pub token_reviewer_jwt: String,
    pub disable_local_ca_jwt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubernetesRole {
    pub name: String,
    pub bound_service_account_names: Vec<String>,
    pub bound_service_account_namespaces: Vec<String>,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
    pub token_policies: Vec<String>,
}

#[derive(Default)]
pub struct KubernetesStore {
    pub config: KubernetesConfig,
    pub roles: HashMap<String, KubernetesRole>,
}

pub async fn configure(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<KubernetesConfig>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kubernetes_store.write().await;
    store.config = body;
    Ok(VaultResponse::new())
}

pub async fn read_config(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kubernetes_store.read().await;
    Ok(VaultResponse::new().with_data(serde_json::to_value(&store.config).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct CreateRoleRequest {
    pub bound_service_account_names: Vec<String>,
    pub bound_service_account_namespaces: Vec<String>,
    pub token_ttl: Option<String>,
    pub token_max_ttl: Option<String>,
    pub token_policies: Option<Vec<String>>,
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<CreateRoleRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kubernetes_store.write().await;
    let role = KubernetesRole {
        name: role_name.clone(),
        bound_service_account_names: body.bound_service_account_names,
        bound_service_account_namespaces: body.bound_service_account_namespaces,
        token_ttl: body
            .token_ttl
            .as_deref()
            .map(crate::token::parse_duration)
            .unwrap_or(3600),
        token_max_ttl: body
            .token_max_ttl
            .as_deref()
            .map(crate::token::parse_duration)
            .unwrap_or(0),
        token_policies: body
            .token_policies
            .unwrap_or_else(|| vec!["default".to_string()]),
    };
    store.roles.insert(role_name, role);
    Ok(VaultResponse::new())
}

pub async fn read_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kubernetes_store.read().await;
    let role = store
        .roles
        .get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(role).unwrap_or_default()))
}

pub async fn delete_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kubernetes_store.write().await;
    store.roles.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kubernetes_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

/// Decode a JWT without verification (base64 decode the payload).
fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    use base64::Engine as _;
    let parts: Vec<&str> = jwt.splitn(3, '.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    serde_json::from_slice(&payload).ok()
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub role: String,
    pub jwt: String,
}

pub async fn login(
    State(state): State<Arc<VaultState>>,
    Json(body): Json<LoginRequest>,
) -> Result<VaultResponse, VaultError> {
    let store = state.kubernetes_store.read().await;
    let role = store
        .roles
        .get(&body.role)
        .ok_or_else(|| VaultError::RoleNotFound(body.role.clone()))?
        .clone();

    // Decode JWT to extract service account info
    let claims =
        decode_jwt_payload(&body.jwt).ok_or_else(|| VaultError::Auth("invalid JWT".into()))?;

    let sub = claims.get("sub").and_then(|v| v.as_str()).unwrap_or("");

    // sub format: system:serviceaccount:<namespace>:<name>
    let parts: Vec<&str> = sub.splitn(4, ':').collect();
    let (sa_namespace, sa_name) =
        if parts.len() == 4 && parts[0] == "system" && parts[1] == "serviceaccount" {
            (parts[2], parts[3])
        } else {
            return Err(VaultError::Auth("invalid service account subject".into()));
        };

    let name_ok = role
        .bound_service_account_names
        .contains(&sa_name.to_string())
        || role.bound_service_account_names.contains(&"*".to_string());
    let ns_ok = role
        .bound_service_account_namespaces
        .contains(&sa_namespace.to_string())
        || role
            .bound_service_account_namespaces
            .contains(&"*".to_string());

    if !name_ok || !ns_ok {
        return Err(VaultError::Auth("service account not authorized".into()));
    }

    drop(store);

    let params = CreateTokenParams {
        policies: Some(role.token_policies.clone()),
        ttl: Some(format!("{}s", role.token_ttl)),
        renewable: Some(true),
        no_parent: Some(true),
        metadata: Some({
            let mut m = HashMap::new();
            m.insert("service_account_name".to_string(), sa_name.to_string());
            m.insert(
                "service_account_namespace".to_string(),
                sa_namespace.to_string(),
            );
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
        .route(
            "/v1/auth/kubernetes/config",
            post(configure).get(read_config),
        )
        .route("/v1/auth/kubernetes/role", get(list_roles))
        .route(
            "/v1/auth/kubernetes/role/{role_name}",
            post(create_role).get(read_role).delete(delete_role),
        )
        .route("/v1/auth/kubernetes/login", post(login))
        .with_state(state)
}
