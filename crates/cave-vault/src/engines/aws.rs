use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::VaultState;
use axum::{
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
    Router,
};
use chrono::Utc;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AwsRootConfig {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub iam_endpoint: String,
    pub sts_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsRole {
    pub name: String,
    pub credential_type: String, // "iam_user", "assumed_role", "federation_token"
    pub policy_arns: Vec<String>,
    pub policy_document: String,
    pub role_arns: Vec<String>,
    pub iam_groups: Vec<String>,
    pub iam_tags: HashMap<String, String>,
    pub default_sts_ttl: i64,
    pub max_sts_ttl: i64,
    pub user_path: String,
    pub permissions_boundary_arn: String,
}

impl Default for AwsRole {
    fn default() -> Self {
        Self {
            name: String::new(),
            credential_type: "iam_user".to_string(),
            policy_arns: Vec::new(),
            policy_document: String::new(),
            role_arns: Vec::new(),
            iam_groups: Vec::new(),
            iam_tags: HashMap::new(),
            default_sts_ttl: 3600,
            max_sts_ttl: 0,
            user_path: "/".to_string(),
            permissions_boundary_arn: String::new(),
        }
    }
}

#[derive(Default)]
pub struct AwsStore {
    pub config: AwsRootConfig,
    pub roles: HashMap<String, AwsRole>,
}

fn random_aws_key_id() -> VaultResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 10];
    rng.fill(&mut bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    let chars: String = bytes.iter().map(|b| {
        let idx = (b % 26) as usize;
        (b'A' + idx as u8) as char
    }).collect();
    Ok(format!("ASIA{}", chars))
}

fn random_secret_key() -> VaultResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = vec![0u8; 20];
    rng.fill(&mut bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

fn random_session_token() -> VaultResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = vec![0u8; 32];
    rng.fill(&mut bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    Ok(format!("FQoGZXIvYXdzENH//////////wEaDNT{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

pub async fn configure_root(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.aws_store.write().await;
    if let Some(v) = body.get("access_key").and_then(|v| v.as_str()) { store.config.access_key = v.to_string(); }
    if let Some(v) = body.get("secret_key").and_then(|v| v.as_str()) { store.config.secret_key = v.to_string(); }
    if let Some(v) = body.get("region").and_then(|v| v.as_str()) { store.config.region = v.to_string(); }
    if let Some(v) = body.get("iam_endpoint").and_then(|v| v.as_str()) { store.config.iam_endpoint = v.to_string(); }
    if let Some(v) = body.get("sts_endpoint").and_then(|v| v.as_str()) { store.config.sts_endpoint = v.to_string(); }
    Ok(VaultResponse::new())
}

pub async fn read_config(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.aws_store.read().await;
    Ok(VaultResponse::new().with_data(json!({
        "access_key": store.config.access_key,
        "region": store.config.region,
        "iam_endpoint": store.config.iam_endpoint,
        "sts_endpoint": store.config.sts_endpoint,
    })))
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.aws_store.write().await;
    let mut role = store.roles.entry(role_name.clone()).or_insert_with(|| AwsRole {
        name: role_name.clone(),
        ..Default::default()
    });
    if let Some(v) = body.get("credential_type").and_then(|v| v.as_str()) { role.credential_type = v.to_string(); }
    if let Some(arns) = body.get("policy_arns").and_then(|v| v.as_array()) {
        role.policy_arns = arns.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(v) = body.get("policy_document").and_then(|v| v.as_str()) { role.policy_document = v.to_string(); }
    if let Some(arns) = body.get("role_arns").and_then(|v| v.as_array()) {
        role.role_arns = arns.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(ttl) = body.get("default_sts_ttl").and_then(|v| v.as_str()) { role.default_sts_ttl = crate::token::parse_duration(ttl); }
    if let Some(ttl) = body.get("max_sts_ttl").and_then(|v| v.as_str()) { role.max_sts_ttl = crate::token::parse_duration(ttl); }
    Ok(VaultResponse::new())
}

pub async fn read_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.aws_store.read().await;
    let role = store.roles.get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(role).unwrap_or_default()))
}

pub async fn delete_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.aws_store.write().await;
    store.roles.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.aws_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn generate_credentials(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.aws_store.read().await;
    let role = store.roles.get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name.clone()))?.clone();
    drop(store);

    let access_key_id = random_aws_key_id()?;
    let secret_access_key = random_secret_key()?;
    let security_token = random_session_token()?;

    let expiry = Utc::now() + chrono::Duration::seconds(role.default_sts_ttl);

    let data = match role.credential_type.as_str() {
        "assumed_role" => json!({
            "access_key": access_key_id,
            "secret_key": secret_access_key,
            "security_token": security_token,
            "assumed_role_arn": role.role_arns.first().cloned().unwrap_or_default(),
            "expiration": expiry.to_rfc3339(),
        }),
        "federation_token" => json!({
            "access_key": access_key_id,
            "secret_key": secret_access_key,
            "security_token": security_token,
            "expiration": expiry.to_rfc3339(),
        }),
        _ => json!({
            "access_key": access_key_id,
            "secret_key": secret_access_key,
            "security_token": null,
        }),
    };

    Ok(VaultResponse::new().with_data(data))
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    Router::new()
        .route(&format!("/v1/{}/config/root", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { configure_root(State(state), headers, Path(mount), Json(body)).await }
            }
        }).get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { read_config(State(state), headers, Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/roles", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { list_roles(State(state), headers, Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/roles/:role_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { create_role(State(state), headers, Path((mount, role_name)), Json(body)).await }
            }
        }).get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { read_role(State(state), headers, Path((mount, role_name))).await }
            }
        }).delete({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { delete_role(State(state), headers, Path((mount, role_name))).await }
            }
        }))
        .route(&format!("/v1/{}/creds/:role_name", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { generate_credentials(State(state), headers, Path((mount, role_name))).await }
            }
        }))
        .with_state(state)
}

use base64::Engine as _;
