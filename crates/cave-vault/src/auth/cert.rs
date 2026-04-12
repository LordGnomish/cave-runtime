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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CertConfig {
    pub disable_binding: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertRole {
    pub name: String,
    pub certificate: String, // PEM-encoded CA or client cert
    pub allowed_names: Vec<String>,
    pub allowed_dns_sans: Vec<String>,
    pub allowed_email_sans: Vec<String>,
    pub allowed_uri_sans: Vec<String>,
    pub required_extensions: Vec<String>,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
    pub token_policies: Vec<String>,
    pub display_name: String,
}

#[derive(Default)]
pub struct CertStore {
    pub config: CertConfig,
    pub roles: HashMap<String, CertRole>,
}

pub async fn configure(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<CertConfig>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.cert_store.write().await;
    store.config = body;
    Ok(VaultResponse::new())
}

#[derive(Deserialize)]
pub struct CreateRoleRequest {
    pub certificate: String,
    pub allowed_names: Option<Vec<String>>,
    pub allowed_dns_sans: Option<Vec<String>>,
    pub allowed_email_sans: Option<Vec<String>>,
    pub allowed_uri_sans: Option<Vec<String>>,
    pub required_extensions: Option<Vec<String>>,
    pub token_ttl: Option<String>,
    pub token_max_ttl: Option<String>,
    pub token_policies: Option<Vec<String>>,
    pub display_name: Option<String>,
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<CreateRoleRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.cert_store.write().await;
    let role = CertRole {
        name: role_name.clone(),
        certificate: body.certificate,
        allowed_names: body.allowed_names.unwrap_or_default(),
        allowed_dns_sans: body.allowed_dns_sans.unwrap_or_default(),
        allowed_email_sans: body.allowed_email_sans.unwrap_or_default(),
        allowed_uri_sans: body.allowed_uri_sans.unwrap_or_default(),
        required_extensions: body.required_extensions.unwrap_or_default(),
        token_ttl: body.token_ttl.as_deref().map(crate::token::parse_duration).unwrap_or(3600),
        token_max_ttl: body.token_max_ttl.as_deref().map(crate::token::parse_duration).unwrap_or(0),
        token_policies: body.token_policies.unwrap_or_else(|| vec!["default".to_string()]),
        display_name: body.display_name.unwrap_or_else(|| role_name.clone()),
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
    let store = state.cert_store.read().await;
    let role = store.roles.get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(role).unwrap_or_default()))
}

pub async fn delete_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.cert_store.write().await;
    store.roles.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.cert_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub name: Option<String>, // role name hint
}

/// TLS cert login. In production, the client cert would be extracted from the TLS layer.
/// Here we accept a role name and issue a token for the matching role (mock).
pub async fn login(
    State(state): State<Arc<VaultState>>,
    Json(body): Json<LoginRequest>,
) -> Result<VaultResponse, VaultError> {
    let store = state.cert_store.read().await;

    let role = if let Some(ref name) = body.name {
        store.roles.get(name)
            .ok_or_else(|| VaultError::RoleNotFound(name.clone()))?
            .clone()
    } else {
        // Pick the first available role
        store.roles.values().next()
            .cloned()
            .ok_or_else(|| VaultError::Auth("no cert roles configured".into()))?
    };
    drop(store);

    let params = CreateTokenParams {
        policies: Some(role.token_policies.clone()),
        ttl: Some(format!("{}s", role.token_ttl)),
        renewable: Some(true),
        no_parent: Some(true),
        display_name: Some(format!("cert-{}", role.display_name)),
        ..Default::default()
    };
    let mut ts = state.token_store.write().await;
    let token = ts.create(&params, None)?;
    Ok(VaultResponse::new().with_auth(token.to_auth_info()))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route("/v1/auth/cert/config", post(configure))
        .route("/v1/auth/cert/certs", get(list_roles))
        .route("/v1/auth/cert/certs/:role_name", post(create_role).get(read_role).delete(delete_role))
        .route("/v1/auth/cert/login", post(login))
        .with_state(state)
}
