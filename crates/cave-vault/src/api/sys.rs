// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::core::{AuditBackend, AuditBackendType};
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::{AuthEntry, MountEntry, MountConfig, VaultState};
use axum::{
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post, put},
    Router,
};
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

pub async fn seal_status(
    State(state): State<Arc<VaultState>>,
) -> Result<VaultResponse, VaultError> {
    let seal = state.seal_state.read().await;
    Ok(VaultResponse::new().with_data(json!({
        "sealed": seal.is_sealed(),
        "initialized": seal.is_initialized(),
        "t": seal.threshold,
        "n": seal.shares,
        "progress": seal.unseal_progress,
        "nonce": seal.unseal_nonce,
        "version": "1.17.0",
        "build_date": "2024-01-01",
        "migration": false,
        "cluster_name": "vault-cluster",
        "cluster_id": uuid::Uuid::new_v4().to_string(),
        "recovery_seal": false,
        "storage_type": "inmem",
    })))
}

pub async fn health(
    State(state): State<Arc<VaultState>>,
) -> Result<VaultResponse, VaultError> {
    let seal = state.seal_state.read().await;
    Ok(VaultResponse::new().with_data(json!({
        "initialized": seal.is_initialized(),
        "sealed": seal.is_sealed(),
        "standby": false,
        "performance_standby": false,
        "replication_performance_mode": "disabled",
        "replication_dr_mode": "disabled",
        "server_time_utc": chrono::Utc::now().timestamp(),
        "version": "1.17.0",
        "cluster_name": "vault-cluster",
        "cluster_id": uuid::Uuid::new_v4().to_string(),
    })))
}

pub async fn init_status(
    State(state): State<Arc<VaultState>>,
) -> Result<VaultResponse, VaultError> {
    let seal = state.seal_state.read().await;
    Ok(VaultResponse::new().with_data(json!({
        "initialized": seal.is_initialized(),
    })))
}

#[derive(Deserialize)]
pub struct InitRequest {
    pub secret_shares: u8,
    pub secret_threshold: u8,
    pub pgp_keys: Option<Vec<String>>,
    pub root_token_pgp_key: Option<String>,
    pub recovery_shares: Option<u8>,
    pub recovery_threshold: Option<u8>,
}

pub async fn initialize(
    State(state): State<Arc<VaultState>>,
    Json(body): Json<InitRequest>,
) -> Result<VaultResponse, VaultError> {
    let (root_token, key_shares) = {
        let mut seal = state.seal_state.write().await;
        seal.initialize(body.secret_shares, body.secret_threshold)?
    };

    // Create the root token in the token store
    let mut ts = state.token_store.write().await;
    ts.create_root(&root_token);

    Ok(VaultResponse::new().with_data(json!({
        "keys": key_shares,
        "keys_base64": key_shares.iter().map(|k| {
            base64::engine::general_purpose::STANDARD.encode(hex::decode(k).unwrap_or_default())
        }).collect::<Vec<_>>(),
        "root_token": root_token,
    })))
}

#[derive(Deserialize)]
pub struct UnsealRequest {
    pub key: Option<String>,
    pub reset: Option<bool>,
    pub migrate: Option<bool>,
}

pub async fn unseal(
    State(state): State<Arc<VaultState>>,
    Json(body): Json<UnsealRequest>,
) -> Result<VaultResponse, VaultError> {
    if body.reset.unwrap_or(false) {
        let mut seal = state.seal_state.write().await;
        seal.pending_shares.clear();
        seal.unseal_progress = 0;
        return seal_status(State(state.clone())).await;
    }

    let key = body.key.ok_or_else(|| VaultError::InvalidRequest("key required".into()))?;
    let unsealed = {
        let mut seal = state.seal_state.write().await;
        seal.unseal(&key)?
    };

    // If just unsealed and we have a root token, register it
    if unsealed {
        let root_token = {
            let seal = state.seal_state.read().await;
            seal.root_token.clone()
        };
        if let Some(rt) = root_token {
            let mut ts = state.token_store.write().await;
            if ts.lookup(&rt).is_none() {
                ts.create_root(&rt);
            }
        }
    }

    seal_status(State(state)).await
}

pub async fn seal(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut seal = state.seal_state.write().await;
    seal.seal();
    Ok(VaultResponse::new())
}

pub async fn list_mounts(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mt = state.mount_table.read().await;
    let mounts: HashMap<String, &MountEntry> = mt.mounts.iter().map(|(k, v)| (k.clone(), v)).collect();
    Ok(VaultResponse::new().with_data(serde_json::to_value(mounts).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct MountRequest {
    #[serde(rename = "type")]
    pub mount_type: String,
    pub description: Option<String>,
    pub config: Option<MountConfig>,
    pub local: Option<bool>,
    pub seal_wrap: Option<bool>,
}

pub async fn enable_mount(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(body): Json<MountRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let path = if path.ends_with('/') { path } else { format!("{}/", path) };
    let entry = MountEntry {
        path: path.clone(),
        mount_type: body.mount_type,
        description: body.description.unwrap_or_default(),
        config: body.config.unwrap_or_default(),
        local: body.local.unwrap_or(false),
        seal_wrap: body.seal_wrap.unwrap_or(false),
        uuid: uuid::Uuid::new_v4().to_string(),
        accessor: uuid::Uuid::new_v4().to_string(),
        namespace_id: String::new(),
    };
    let mut mt = state.mount_table.write().await;
    mt.mounts.insert(path, entry);
    Ok(VaultResponse::new())
}

pub async fn disable_mount(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let path = if path.ends_with('/') { path } else { format!("{}/", path) };
    let mut mt = state.mount_table.write().await;
    mt.mounts.remove(&path);
    Ok(VaultResponse::new())
}

pub async fn tune_mount(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let path = if path.ends_with('/') { path } else { format!("{}/", path) };
    let mut mt = state.mount_table.write().await;
    if let Some(entry) = mt.mounts.get_mut(&path) {
        if let Some(ttl) = body.get("default_lease_ttl").and_then(|v| v.as_i64()) {
            entry.config.default_lease_ttl = ttl;
        }
        if let Some(ttl) = body.get("max_lease_ttl").and_then(|v| v.as_i64()) {
            entry.config.max_lease_ttl = ttl;
        }
    }
    Ok(VaultResponse::new())
}

pub async fn list_auth(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let at = state.auth_table.read().await;
    Ok(VaultResponse::new().with_data(serde_json::to_value(&at.methods).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct AuthRequest {
    #[serde(rename = "type")]
    pub auth_type: String,
    pub description: Option<String>,
    pub config: Option<MountConfig>,
    pub local: Option<bool>,
}

pub async fn enable_auth(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(body): Json<AuthRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let path = if path.ends_with('/') { path } else { format!("{}/", path) };
    let entry = AuthEntry {
        path: path.clone(),
        auth_type: body.auth_type,
        description: body.description.unwrap_or_default(),
        config: body.config.unwrap_or_default(),
        local: body.local.unwrap_or(false),
        seal_wrap: false,
        uuid: uuid::Uuid::new_v4().to_string(),
        accessor: uuid::Uuid::new_v4().to_string(),
    };
    let mut at = state.auth_table.write().await;
    at.methods.insert(path, entry);
    Ok(VaultResponse::new())
}

pub async fn disable_auth(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let path = if path.ends_with('/') { path } else { format!("{}/", path) };
    let mut at = state.auth_table.write().await;
    at.methods.remove(&path);
    Ok(VaultResponse::new())
}

pub async fn list_policies(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let ps = state.policy_store.read().await;
    let policies = ps.list();
    Ok(VaultResponse::new().with_data(json!({ "keys": policies, "policies": policies })))
}

pub async fn read_policy(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let ps = state.policy_store.read().await;
    let policy = ps.get(&name)
        .ok_or_else(|| VaultError::PolicyNotFound(name))?;
    Ok(VaultResponse::new().with_data(json!({
        "name": policy.name,
        "policy": policy.raw,
        "rules": policy.raw,
    })))
}

#[derive(Deserialize)]
pub struct PolicyRequest {
    pub policy: Option<String>,
    pub rules: Option<String>,
}

pub async fn write_policy(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<PolicyRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let hcl = body.policy.or(body.rules)
        .ok_or_else(|| VaultError::InvalidRequest("policy required".into()))?;
    let policy = crate::core::Policy::parse(&name, &hcl)?;
    let mut ps = state.policy_store.write().await;
    ps.put(policy);
    Ok(VaultResponse::new())
}

pub async fn delete_policy(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut ps = state.policy_store.write().await;
    if !ps.delete(&name) {
        return Err(VaultError::InvalidRequest(format!("cannot delete built-in policy: {}", name)));
    }
    Ok(VaultResponse::new())
}

#[derive(Deserialize)]
pub struct LeaseLookupRequest {
    pub lease_id: Option<String>,
    pub prefix: Option<String>,
}

pub async fn renew_lease(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let lease_id = body.get("lease_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("lease_id required".into()))?;
    let increment = body.get("increment").and_then(|v| v.as_i64()).unwrap_or(3600);
    let mut ls = state.lease_store.write().await;
    let lease = ls.renew(lease_id, increment)?;
    Ok(VaultResponse::new().with_data(json!({
        "lease_id": lease.id,
        "renewable": lease.renewable,
        "lease_duration": lease.remaining_secs(),
    })))
}

pub async fn revoke_lease(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let lease_id = body.get("lease_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("lease_id required".into()))?;
    let mut ls = state.lease_store.write().await;
    ls.revoke(lease_id);
    Ok(VaultResponse::new())
}

pub async fn list_leases(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let prefix = body.get("prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ls = state.lease_store.read().await;
    let leases: Vec<String> = ls.list_by_prefix(prefix).iter().map(|l| l.id.clone()).collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": leases })))
}

pub async fn list_audit(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let backends = state.audit_logger.list_backends();
    Ok(VaultResponse::new().with_data(serde_json::to_value(backends).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct AuditEnableRequest {
    #[serde(rename = "type")]
    pub backend_type: String,
    pub description: Option<String>,
    pub options: Option<HashMap<String, String>>,
    pub local: Option<bool>,
}

pub async fn enable_audit(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(body): Json<AuditEnableRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let backend_type = match body.backend_type.as_str() {
        "file" => AuditBackendType::File,
        "syslog" => AuditBackendType::Syslog,
        "socket" => AuditBackendType::Socket,
        t => return Err(VaultError::InvalidRequest(format!("unknown audit backend type: {}", t))),
    };
    let backend = AuditBackend {
        path: path.clone(),
        backend_type,
        description: body.description.unwrap_or_default(),
        options: body.options.unwrap_or_default(),
        local: body.local.unwrap_or(false),
        seal_wrap: false,
    };
    state.audit_logger.enable(&path, backend);
    Ok(VaultResponse::new())
}

pub async fn disable_audit(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    state.audit_logger.disable(&path);
    Ok(VaultResponse::new())
}

pub async fn wrapping_wrap(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let ttl_secs = headers.get("x-vault-wrap-ttl")
        .and_then(|v| v.to_str().ok())
        .map(crate::token::parse_duration)
        .unwrap_or(300);
    let mut ws = state.wrap_store.write().await;
    let wrap_info = ws.wrap(body, ttl_secs, "sys/wrapping/wrap")?;
    let mut resp = VaultResponse::new();
    resp.wrap_info = Some(wrap_info);
    Ok(resp)
}

pub async fn wrapping_unwrap(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let token_str = body.get("token")
        .and_then(|v| v.as_str())
        .or_else(|| headers.get("x-vault-token").and_then(|v| v.to_str().ok()))
        .ok_or(VaultError::BadToken)?
        .to_string();
    let mut ws = state.wrap_store.write().await;
    let data = ws.unwrap(&token_str)?;
    Ok(VaultResponse::new().with_data(data))
}

pub async fn wrapping_lookup(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let wrap_token = body.get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("token required".into()))?;
    let ws = state.wrap_store.read().await;
    let info = ws.lookup(wrap_token)?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(info).unwrap_or_default()))
}

pub async fn wrapping_rewrap(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let wrap_token = body.get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("token required".into()))?;
    let ttl_secs = headers.get("x-vault-wrap-ttl")
        .and_then(|v| v.to_str().ok())
        .map(crate::token::parse_duration)
        .unwrap_or(300);
    let mut ws = state.wrap_store.write().await;
    let info = ws.rewrap(wrap_token, ttl_secs)?;
    let mut resp = VaultResponse::new();
    resp.wrap_info = Some(info);
    Ok(resp)
}

pub async fn capabilities_self(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    let policies = token.policies.clone();
    drop(ts);

    let paths = body.get("paths")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>())
        .unwrap_or_default();

    let ps = state.policy_store.read().await;
    let mut caps: HashMap<String, Vec<String>> = HashMap::new();
    for path in &paths {
        let mut path_caps = Vec::new();
        for cap_name in &["create", "read", "update", "delete", "list", "sudo"] {
            let cap = crate::core::Capability::from_str(cap_name).unwrap();
            if ps.check(&policies, path, &cap) {
                path_caps.push(cap_name.to_string());
            }
        }
        if path_caps.is_empty() {
            path_caps.push("deny".to_string());
        }
        caps.insert(path.clone(), path_caps);
    }

    Ok(VaultResponse::new().with_data(serde_json::to_value(caps).unwrap_or_default()))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route("/v1/sys/seal-status", get(seal_status))
        .route("/v1/sys/health", get(health))
        .route("/v1/sys/init", get(init_status).post(initialize))
        .route("/v1/sys/unseal", post(unseal))
        .route("/v1/sys/seal", post(seal))
        .route("/v1/sys/mounts", get(list_mounts))
        .route("/v1/sys/mounts/{path}", post(enable_mount).delete(disable_mount))
        .route("/v1/sys/mounts/{path}/tune", post(tune_mount))
        .route("/v1/sys/auth", get(list_auth))
        .route("/v1/sys/auth/{path}", post(enable_auth).delete(disable_auth))
        .route("/v1/sys/policies/acl", get(list_policies))
        .route("/v1/sys/policies/acl/{name}", get(read_policy).put(write_policy).post(write_policy).delete(delete_policy))
        .route("/v1/sys/leases/renew", put(renew_lease).post(renew_lease))
        .route("/v1/sys/leases/revoke", put(revoke_lease).post(revoke_lease))
        .route("/v1/sys/leases/lookup", post(list_leases))
        .route("/v1/sys/audit", get(list_audit))
        .route("/v1/sys/audit/{path}", put(enable_audit).delete(disable_audit))
        .route("/v1/sys/wrapping/wrap", post(wrapping_wrap))
        .route("/v1/sys/wrapping/unwrap", post(wrapping_unwrap))
        .route("/v1/sys/wrapping/lookup", post(wrapping_lookup))
        .route("/v1/sys/wrapping/rewrap", post(wrapping_rewrap))
        .route("/v1/sys/capabilities-self", post(capabilities_self))
        .with_state(state)
}

use base64::Engine as _;
