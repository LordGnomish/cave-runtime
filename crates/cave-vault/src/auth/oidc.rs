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
use base64::Engine as _;
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
pub struct OidcConfig {
    pub oidc_discovery_url: String,
    pub oidc_client_id: String,
    pub oidc_client_secret: String,
    pub bound_issuer: String,
    pub jwks_url: String,
    pub jwt_supported_algs: Vec<String>,
    pub default_role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcRole {
    pub name: String,
    pub role_type: String, // "jwt" or "oidc"
    pub bound_audiences: Vec<String>,
    pub bound_subject: String,
    pub bound_claims: HashMap<String, String>,
    pub user_claim: String,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
    pub token_policies: Vec<String>,
    pub allowed_redirect_uris: Vec<String>,
    pub groups_claim: String,
    pub claim_mappings: HashMap<String, String>,
}

impl Default for OidcRole {
    fn default() -> Self {
        Self {
            name: String::new(),
            role_type: "jwt".to_string(),
            bound_audiences: Vec::new(),
            bound_subject: String::new(),
            bound_claims: HashMap::new(),
            user_claim: "sub".to_string(),
            token_ttl: 3600,
            token_max_ttl: 0,
            token_policies: vec!["default".to_string()],
            allowed_redirect_uris: Vec::new(),
            groups_claim: String::new(),
            claim_mappings: HashMap::new(),
        }
    }
}

#[derive(Default)]
pub struct OidcStore {
    pub config: OidcConfig,
    pub roles: HashMap<String, OidcRole>,
}

pub async fn configure(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<OidcConfig>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.oidc_store.write().await;
    store.config = body;
    Ok(VaultResponse::new())
}

pub async fn read_config(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.oidc_store.read().await;
    let mut cfg = serde_json::to_value(&store.config).unwrap_or_default();
    if let Some(obj) = cfg.as_object_mut() {
        obj.remove("oidc_client_secret");
    }
    Ok(VaultResponse::new().with_data(cfg))
}

#[derive(Deserialize)]
pub struct CreateRoleRequest {
    pub role_type: Option<String>,
    pub bound_audiences: Option<Vec<String>>,
    pub bound_subject: Option<String>,
    pub user_claim: Option<String>,
    pub token_ttl: Option<String>,
    pub token_max_ttl: Option<String>,
    pub token_policies: Option<Vec<String>>,
    pub allowed_redirect_uris: Option<Vec<String>>,
    pub groups_claim: Option<String>,
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(body): Json<CreateRoleRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.oidc_store.write().await;
    let mut role = store.roles.entry(role_name.clone()).or_insert_with(|| OidcRole {
        name: role_name.clone(),
        ..Default::default()
    });
    if let Some(rt) = body.role_type { role.role_type = rt; }
    if let Some(aud) = body.bound_audiences { role.bound_audiences = aud; }
    if let Some(sub) = body.bound_subject { role.bound_subject = sub; }
    if let Some(uc) = body.user_claim { role.user_claim = uc; }
    if let Some(ttl) = body.token_ttl { role.token_ttl = crate::token::parse_duration(&ttl); }
    if let Some(ttl) = body.token_max_ttl { role.token_max_ttl = crate::token::parse_duration(&ttl); }
    if let Some(p) = body.token_policies { role.token_policies = p; }
    if let Some(uris) = body.allowed_redirect_uris { role.allowed_redirect_uris = uris; }
    if let Some(gc) = body.groups_claim { role.groups_claim = gc; }
    Ok(VaultResponse::new())
}

pub async fn read_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.oidc_store.read().await;
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
    let mut store = state.oidc_store.write().await;
    store.roles.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.oidc_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

/// Decode JWT without signature verification (for testing/mock purposes).
fn decode_jwt_claims(jwt: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = jwt.splitn(3, '.').collect();
    if parts.len() < 2 { return None; }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&payload).ok()
}

#[derive(Deserialize)]
pub struct JwtLoginRequest {
    pub role: Option<String>,
    pub jwt: String,
}

pub async fn jwt_login(
    State(state): State<Arc<VaultState>>,
    Json(body): Json<JwtLoginRequest>,
) -> Result<VaultResponse, VaultError> {
    let store = state.oidc_store.read().await;
    let role_name = body.role.clone()
        .or_else(|| if store.config.default_role.is_empty() { None } else { Some(store.config.default_role.clone()) })
        .ok_or_else(|| VaultError::InvalidRequest("role required".into()))?;
    let role = store.roles.get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name.clone()))?.clone();

    let claims = decode_jwt_claims(&body.jwt)
        .ok_or_else(|| VaultError::Auth("invalid JWT".into()))?;

    // Validate bound_subject
    if !role.bound_subject.is_empty() {
        let sub = claims.get("sub").and_then(|v| v.as_str()).unwrap_or("");
        if sub != role.bound_subject {
            return Err(VaultError::Auth("bound_subject mismatch".into()));
        }
    }

    // Validate bound_audiences
    if !role.bound_audiences.is_empty() {
        let aud = claims.get("aud");
        let matches = match aud {
            Some(serde_json::Value::String(s)) => role.bound_audiences.contains(s),
            Some(serde_json::Value::Array(arr)) => arr.iter()
                .filter_map(|v| v.as_str())
                .any(|a| role.bound_audiences.contains(&a.to_string())),
            _ => false,
        };
        if !matches {
            return Err(VaultError::Auth("bound_audiences mismatch".into()));
        }
    }

    let user_claim_val = claims.get(&role.user_claim)
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    drop(store);

    let params = CreateTokenParams {
        policies: Some(role.token_policies.clone()),
        ttl: Some(format!("{}s", role.token_ttl)),
        renewable: Some(true),
        no_parent: Some(true),
        metadata: Some({
            let mut m = HashMap::new();
            m.insert("role".to_string(), role_name);
            m.insert("user".to_string(), user_claim_val.to_string());
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
        .route("/v1/auth/jwt/config", post(configure).get(read_config))
        .route("/v1/auth/jwt/role", get(list_roles))
        .route("/v1/auth/jwt/role/:role_name", post(create_role).get(read_role).delete(delete_role))
        .route("/v1/auth/jwt/login", post(jwt_login))
        .route("/v1/auth/oidc/config", post(configure).get(read_config))
        .route("/v1/auth/oidc/role", get(list_roles))
        .route("/v1/auth/oidc/role/:role_name", post(create_role).get(read_role).delete(delete_role))
        .route("/v1/auth/oidc/login", post(jwt_login))
        .with_state(state)
}
