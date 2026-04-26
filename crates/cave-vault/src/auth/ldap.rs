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
pub struct LdapConfig {
    pub url: String,
    pub starttls: bool,
    pub tls_min_version: String,
    pub bind_dn: String,
    pub bind_pass: String,
    pub user_dn: String,
    pub user_attr: String,
    pub group_dn: String,
    pub group_attr: String,
    pub group_filter: String,
    pub certificate: String,
    pub token_ttl: i64,
    pub token_max_ttl: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapGroup {
    pub name: String,
    pub policies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapUser {
    pub username: String,
    pub policies: Vec<String>,
    pub groups: Vec<String>,
}

#[derive(Default)]
pub struct LdapStore {
    pub config: LdapConfig,
    pub groups: HashMap<String, LdapGroup>,
    pub users: HashMap<String, LdapUser>,
}

pub async fn configure(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<LdapConfig>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.ldap_store.write().await;
    store.config = body;
    Ok(VaultResponse::new())
}

pub async fn read_config(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.ldap_store.read().await;
    let mut cfg = serde_json::to_value(&store.config).unwrap_or_default();
    // Mask bind_pass
    if let Some(obj) = cfg.as_object_mut() {
        obj.remove("bind_pass");
    }
    Ok(VaultResponse::new().with_data(cfg))
}

#[derive(Deserialize)]
pub struct GroupRequest {
    pub policies: Option<Vec<String>>,
}

pub async fn create_group(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(group_name): Path<String>,
    Json(body): Json<GroupRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.ldap_store.write().await;
    store.groups.insert(group_name.clone(), LdapGroup {
        name: group_name,
        policies: body.policies.unwrap_or_default(),
    });
    Ok(VaultResponse::new())
}

pub async fn read_group(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(group_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.ldap_store.read().await;
    let group = store.groups.get(&group_name)
        .ok_or_else(|| VaultError::NotFound(format!("group {} not found", group_name)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(group).unwrap_or_default()))
}

pub async fn delete_group(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(group_name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.ldap_store.write().await;
    store.groups.remove(&group_name);
    Ok(VaultResponse::new())
}

pub async fn list_groups(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.ldap_store.read().await;
    let keys: Vec<String> = store.groups.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

#[derive(Deserialize)]
pub struct UserPolicyRequest {
    pub policies: Option<Vec<String>>,
    pub groups: Option<Vec<String>>,
}

pub async fn create_user_policy(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(username): Path<String>,
    Json(body): Json<UserPolicyRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.ldap_store.write().await;
    store.users.insert(username.clone(), LdapUser {
        username: username,
        policies: body.policies.unwrap_or_default(),
        groups: body.groups.unwrap_or_default(),
    });
    Ok(VaultResponse::new())
}

pub async fn delete_user_policy(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.ldap_store.write().await;
    store.users.remove(&username);
    Ok(VaultResponse::new())
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<VaultState>>,
    Path(username): Path<String>,
    Json(_body): Json<LoginRequest>,
) -> Result<VaultResponse, VaultError> {
    // In production, this would bind to LDAP and verify credentials.
    // This mock always succeeds (for testing without an actual LDAP server).
    let store = state.ldap_store.read().await;
    let mut policies = vec!["default".to_string()];

    // Add user-specific policies
    if let Some(user) = store.users.get(&username) {
        policies.extend(user.policies.clone());
        // Add group policies
        for group_name in &user.groups {
            if let Some(group) = store.groups.get(group_name) {
                policies.extend(group.policies.clone());
            }
        }
    }

    let ttl = store.config.token_ttl;
    let ttl = if ttl == 0 { 3600 } else { ttl };
    drop(store);

    // Deduplicate
    policies.sort();
    policies.dedup();

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
        display_name: Some(format!("ldap-{}", username)),
        ..Default::default()
    };
    let mut ts = state.token_store.write().await;
    let token = ts.create(&params, None)?;
    Ok(VaultResponse::new().with_auth(token.to_auth_info()))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route("/v1/auth/ldap/config", post(configure).get(read_config))
        .route("/v1/auth/ldap/groups", get(list_groups))
        .route("/v1/auth/ldap/groups/{group_name}", post(create_group).get(read_group).delete(delete_group))
        .route("/v1/auth/ldap/users/{username}", post(create_user_policy).delete(delete_user_policy))
        .route("/v1/auth/ldap/login/{username}", post(login))
        .with_state(state)
}
