//! HTTP routes — Vault v1 API compatible paths.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use base64::Engine as _;

use crate::{
    audit::AuditEntry,
    auth::AuthEngine,
    database::DbRole,
    error::VaultError,
    kv::{KVEntry, KVV1Entry},
    policy::Capability,
    transit::TransitKeyType,
    SharedVaultState,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-vault-token")
        .or_else(|| headers.get("X-Vault-Token"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn token_or_err(headers: &HeaderMap) -> Result<String, VaultError> {
    extract_token(headers).ok_or(VaultError::InvalidToken)
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn create_router(state: SharedVaultState) -> Router {
    Router::new()
        // === Sys ===
        .route("/v1/sys/health",       get(sys_health))
        .route("/v1/sys/seal-status",  get(sys_seal_status))
        .route("/v1/sys/init",         post(sys_init))
        .route("/v1/sys/seal",         put(sys_seal))
        .route("/v1/sys/unseal",       put(sys_unseal))
        .route("/v1/sys/policy",       get(sys_list_policies))
        .route("/v1/sys/policy/:name", get(sys_get_policy).post(sys_put_policy).delete(sys_delete_policy))
        .route("/v1/sys/audit",        get(sys_audit_log))
        // === Auth: token ===
        .route("/v1/auth/token/create",      post(auth_token_create))
        .route("/v1/auth/token/lookup-self",  get(auth_token_lookup_self))
        .route("/v1/auth/token/renew-self",   post(auth_token_renew_self))
        .route("/v1/auth/token/revoke-self",  post(auth_token_revoke_self))
        // === Auth: userpass ===
        .route("/v1/auth/userpass/login/:username",  post(auth_userpass_login))
        .route("/v1/auth/userpass/users/:username",  post(auth_userpass_create).delete(auth_userpass_delete))
        // === Auth: AppRole ===
        .route("/v1/auth/approle/role/:role_name",              post(approle_create_role))
        .route("/v1/auth/approle/role/:role_name/role-id",      get(approle_get_role_id))
        .route("/v1/auth/approle/role/:role_name/secret-id",    post(approle_gen_secret_id))
        .route("/v1/auth/approle/login",                        post(approle_login))
        // === Auth: OIDC ===
        .route("/v1/auth/oidc/callback", post(auth_oidc_callback))
        // === KV v2 ===
        .route("/v1/secret/data/*path",     get(kv_v2_read).post(kv_v2_write).delete(kv_v2_delete_latest))
        .route("/v1/secret/metadata/*path", get(kv_v2_metadata).post(kv_v2_update_metadata).delete(kv_v2_delete_all))
        .route("/v1/secret/delete/*path",   post(kv_v2_soft_delete))
        .route("/v1/secret/undelete/*path", post(kv_v2_undelete))
        .route("/v1/secret/destroy/*path",  post(kv_v2_destroy))
        .route("/v1/secret/list/*path",     get(kv_v2_list))
        // === KV v1 ===
        .route("/v1/kv/*path", get(kv_v1_read).post(kv_v1_write).delete(kv_v1_delete))
        // === Transit ===
        .route("/v1/transit/keys",             post(transit_create_key))
        .route("/v1/transit/keys/:name",       get(transit_get_key).delete(transit_delete_key))
        .route("/v1/transit/keys/:name/rotate", post(transit_rotate_key))
        .route("/v1/transit/encrypt/:name",    post(transit_encrypt))
        .route("/v1/transit/decrypt/:name",    post(transit_decrypt))
        .route("/v1/transit/rewrap/:name",     post(transit_rewrap))
        .route("/v1/transit/sign/:name",       post(transit_sign))
        .route("/v1/transit/verify/:name",     post(transit_verify))
        .route("/v1/transit/datakey/:name",    post(transit_datakey))
        // === PKI ===
        .route("/v1/pki/root/generate",   post(pki_generate_root))
        .route("/v1/pki/intermediate/generate", post(pki_generate_intermediate))
        .route("/v1/pki/issue",           post(pki_issue))
        .route("/v1/pki/revoke",          post(pki_revoke))
        .route("/v1/pki/crl",             get(pki_crl))
        .route("/v1/pki/ca",              get(pki_ca))
        .route("/v1/pki/cert/:serial",    get(pki_cert))
        // === Database ===
        .route("/v1/database/roles/:name",  post(db_create_role).get(db_get_role))
        .route("/v1/database/creds/:name",  get(db_generate_creds))
        .route("/v1/database/revoke/:lease_id", post(db_revoke_creds))
        // === Leases ===
        .route("/v1/sys/leases/renew",  put(lease_renew))
        .route("/v1/sys/leases/revoke", put(lease_revoke))
        .with_state(state)
}

// ── Sys handlers ──────────────────────────────────────────────────────────────

async fn sys_health(State(state): State<SharedVaultState>) -> Json<Value> {
    let s = state.read().await;
    Json(serde_json::json!({
        "initialized": s.initialized,
        "sealed": s.sealed,
        "version": env!("CARGO_PKG_VERSION"),
        "cluster_id": s.cluster_id,
    }))
}

async fn sys_seal_status(State(state): State<SharedVaultState>) -> Json<Value> {
    let s = state.read().await;
    Json(serde_json::to_value(s.seal_status()).unwrap())
}

#[derive(Deserialize)]
struct InitRequest {
    secret_shares: u8,
    secret_threshold: u8,
}

async fn sys_init(
    State(state): State<SharedVaultState>,
    Json(req): Json<InitRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let (shares, root_token) = s.initialize(req.secret_shares, req.secret_threshold)?;
    Ok(Json(serde_json::json!({
        "keys": shares,
        "root_token": root_token,
    })))
}

async fn sys_seal(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?; // must be authenticated to seal
    s.seal();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UnsealRequest {
    key: String,
}

async fn sys_unseal(
    State(state): State<SharedVaultState>,
    Json(req): Json<UnsealRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let done = s.unseal(&req.key)?;
    Ok(Json(serde_json::json!({
        "sealed": s.sealed,
        "progress": s.unseal_buffer.len(),
        "t": s.seal_config.secret_threshold,
        "n": s.seal_config.secret_shares,
        "unsealed": done,
    })))
}

async fn sys_list_policies(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    Ok(Json(serde_json::json!({ "policies": s.policy.list() })))
}

async fn sys_get_policy(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let policy = s.policy.get(&name)
        .ok_or_else(|| VaultError::NotFound(format!("policy '{name}'")))?;
    Ok(Json(serde_json::to_value(policy).unwrap()))
}

#[derive(Deserialize)]
struct PolicyRequest {
    paths: Vec<crate::policy::PolicyPath>,
}

async fn sys_put_policy(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<PolicyRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, "sys/policy", &Capability::Update)
        .map_err(|_| VaultError::PermissionDenied("sys/policy update".into()))?;
    s.policy.put(crate::policy::Policy {
        name: name.clone(),
        paths: req.paths,
    });
    Ok(StatusCode::NO_CONTENT)
}

async fn sys_delete_policy(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, "sys/policy", &Capability::Delete)
        .map_err(|_| VaultError::PermissionDenied("sys/policy delete".into()))?;
    s.policy.delete(&name);
    Ok(StatusCode::NO_CONTENT)
}

async fn sys_audit_log(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, "sys/audit", &Capability::Read)
        .map_err(|_| VaultError::PermissionDenied("sys/audit read".into()))?;
    let entries = s.audit.entries();
    Ok(Json(serde_json::json!({ "entries": entries })))
}

// ── Auth: Token ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenCreateRequest {
    policies: Option<Vec<String>>,
    ttl: Option<u64>,
    renewable: Option<bool>,
    display_name: Option<String>,
    meta: Option<HashMap<String, String>>,
}

async fn auth_token_create(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<TokenCreateRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token_str = token_or_err(&headers)?;
    let caller_policies = s.authenticate(&token_str)?;
    s.policy.check(&caller_policies, "auth/token/create", &Capability::Create)
        .map_err(|_| VaultError::PermissionDenied("auth/token/create".into()))?;

    let new_token = s.auth.mint_token(
        req.display_name.as_deref().unwrap_or("token"),
        req.policies.unwrap_or_else(|| vec!["default".into()]),
        req.ttl.unwrap_or(3600),
        req.renewable.unwrap_or(true),
        req.meta.unwrap_or_default(),
    );
    Ok(Json(serde_json::json!({
        "auth": {
            "client_token": new_token.token_id,
            "accessor": new_token.accessor,
            "policies": new_token.policies,
            "lease_duration": new_token.ttl,
            "renewable": new_token.renewable,
        }
    })))
}

async fn auth_token_lookup_self(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token_str = token_or_err(&headers)?;
    let info = s.auth.lookup_token(&token_str)?;
    Ok(Json(serde_json::to_value(info).unwrap()))
}

#[derive(Deserialize)]
struct RenewRequest {
    increment: Option<u64>,
}

async fn auth_token_renew_self(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<RenewRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token_str = token_or_err(&headers)?;
    let info = s.auth.renew_token(&token_str, req.increment.unwrap_or(0))?;
    Ok(Json(serde_json::json!({
        "auth": {
            "client_token": info.token_id,
            "lease_duration": info.ttl,
            "renewable": info.renewable,
        }
    })))
}

async fn auth_token_revoke_self(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token_str = token_or_err(&headers)?;
    s.auth.revoke_token(&token_str)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Auth: Userpass ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UserpassLoginRequest {
    password: String,
}

async fn auth_userpass_login(
    State(state): State<SharedVaultState>,
    Path(username): Path<String>,
    Json(req): Json<UserpassLoginRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let result = s.auth.userpass_login(&username, &req.password)?;
    Ok(Json(serde_json::json!({ "auth": result })))
}

#[derive(Deserialize)]
struct UserpassCreateRequest {
    password: String,
    policies: Option<Vec<String>>,
    token_ttl: Option<u64>,
}

async fn auth_userpass_create(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(username): Path<String>,
    Json(req): Json<UserpassCreateRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    s.auth.userpass_create(
        &username,
        &req.password,
        req.policies.unwrap_or_else(|| vec!["default".into()]),
        req.token_ttl.unwrap_or(3600),
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn auth_userpass_delete(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    s.auth.userpass.remove(&username);
    Ok(StatusCode::NO_CONTENT)
}

// ── Auth: AppRole ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AppRoleCreateRequest {
    policies: Option<Vec<String>>,
    token_ttl: Option<u64>,
    token_max_ttl: Option<u64>,
    secret_id_ttl: Option<u64>,
    bind_secret_id: Option<bool>,
}

async fn approle_create_role(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
    Json(req): Json<AppRoleCreateRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    s.auth.approle_create(
        &role_name,
        req.policies.unwrap_or_else(|| vec!["default".into()]),
        req.token_ttl.unwrap_or(3600),
        req.token_max_ttl.unwrap_or(86400),
        req.secret_id_ttl.unwrap_or(600),
        req.bind_secret_id.unwrap_or(true),
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn approle_get_role_id(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let role = s.auth.approles.get(&role_name)
        .ok_or_else(|| VaultError::NotFound(format!("approle '{role_name}'")))?;
    Ok(Json(serde_json::json!({ "data": { "role_id": role.role_id } })))
}

async fn approle_gen_secret_id(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(role_name): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let secret_id = s.auth.approle_generate_secret_id(&role_name, HashMap::new())?;
    Ok(Json(serde_json::json!({ "data": { "secret_id": secret_id } })))
}

#[derive(Deserialize)]
struct AppRoleLoginRequest {
    role_id: String,
    secret_id: String,
}

async fn approle_login(
    State(state): State<SharedVaultState>,
    Json(req): Json<AppRoleLoginRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let result = s.auth.approle_login(&req.role_id, &req.secret_id)?;
    Ok(Json(serde_json::json!({ "auth": result })))
}

// ── Auth: OIDC ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct OidcCallbackRequest {
    code: String,
    state: String,
}

async fn auth_oidc_callback(
    State(state): State<SharedVaultState>,
    Json(req): Json<OidcCallbackRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let result = s.auth.oidc_login(&req.code, &req.state, vec!["default".into()])?;
    Ok(Json(serde_json::json!({ "auth": result })))
}

// ── KV v2 ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct VersionQuery {
    version: Option<u32>,
}

async fn kv_v2_read(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    let full_path = format!("secret/data/{path}");
    s.policy.check(&policies, &full_path, &Capability::Read)?;

    let entry = s.kv_v2.get(&path)
        .ok_or_else(|| VaultError::NotFound(path.clone()))?;
    let version_data = entry.get(q.version)?;
    Ok(Json(serde_json::json!({
        "data": {
            "data": version_data.data,
            "metadata": {
                "created_time": version_data.created_time,
                "version": version_data.version,
                "destroyed": version_data.destroyed,
                "deletion_time": version_data.deletion_time,
            }
        }
    })))
}

#[derive(Deserialize)]
struct KVWriteRequest {
    data: HashMap<String, Value>,
    options: Option<KVWriteOptions>,
}

#[derive(Deserialize)]
struct KVWriteOptions {
    cas: Option<u32>,
}

async fn kv_v2_write(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(req): Json<KVWriteRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    let full_path = format!("secret/data/{path}");
    s.policy.check(&policies, &full_path, &Capability::Create)
        .or_else(|_| s.policy.check(&policies, &full_path, &Capability::Update))?;

    let cas = req.options.as_ref().and_then(|o| o.cas);
    let entry = s.kv_v2.entry(path.clone()).or_insert_with(|| KVEntry::new(10));
    let version = entry.put(req.data, cas)?;
    Ok(Json(serde_json::json!({
        "data": { "version": version, "created_time": Utc::now() }
    })))
}

use chrono::Utc;

async fn kv_v2_delete_latest(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/data/{path}"), &Capability::Delete)?;
    if let Some(entry) = s.kv_v2.get_mut(&path) {
        entry.delete_latest();
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_v2_metadata(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/metadata/{path}"), &Capability::Read)?;
    let entry = s.kv_v2.get(&path)
        .ok_or_else(|| VaultError::NotFound(path.clone()))?;
    Ok(Json(serde_json::to_value(entry.metadata()).unwrap()))
}

#[derive(Deserialize)]
struct KVMetaUpdateRequest {
    max_versions: Option<u32>,
    cas_required: Option<bool>,
    custom_metadata: Option<HashMap<String, String>>,
}

async fn kv_v2_update_metadata(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(req): Json<KVMetaUpdateRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/metadata/{path}"), &Capability::Update)?;
    let entry = s.kv_v2.entry(path).or_insert_with(|| KVEntry::new(10));
    if let Some(mv) = req.max_versions { entry.max_versions = mv; }
    if let Some(cas) = req.cas_required { entry.cas_required = cas; }
    if let Some(meta) = req.custom_metadata { entry.custom_metadata = meta; }
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_v2_delete_all(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/metadata/{path}"), &Capability::Delete)?;
    s.kv_v2.remove(&path);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct VersionsRequest {
    versions: Vec<u32>,
}

async fn kv_v2_soft_delete(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(req): Json<VersionsRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/delete/{path}"), &Capability::Delete)?;
    if let Some(entry) = s.kv_v2.get_mut(&path) {
        entry.soft_delete(&req.versions);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_v2_undelete(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(req): Json<VersionsRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/undelete/{path}"), &Capability::Update)?;
    if let Some(entry) = s.kv_v2.get_mut(&path) {
        entry.undelete(&req.versions);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_v2_destroy(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(req): Json<VersionsRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/destroy/{path}"), &Capability::Delete)?;
    if let Some(entry) = s.kv_v2.get_mut(&path) {
        entry.destroy(&req.versions);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_v2_list(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("secret/metadata/{path}"), &Capability::List)?;
    let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
    let mut keys: Vec<String> = s.kv_v2.keys()
        .filter(|k| if prefix.is_empty() { true } else { k.starts_with(&prefix) })
        .map(|k| {
            if prefix.is_empty() { k.clone() }
            else { k.trim_start_matches(&prefix).to_string() }
        })
        .collect();
    keys.sort();
    Ok(Json(serde_json::json!({ "data": { "keys": keys } })))
}

// ── KV v1 ────────────────────────────────────────────────────────────────────

async fn kv_v1_read(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("kv/{path}"), &Capability::Read)?;
    let entry = s.kv_v1.get(&path)
        .ok_or_else(|| VaultError::NotFound(path.clone()))?;
    Ok(Json(serde_json::json!({ "data": entry.data })))
}

async fn kv_v1_write(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(data): Json<HashMap<String, Value>>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("kv/{path}"), &Capability::Create)
        .or_else(|_| s.policy.check(&policies, &format!("kv/{path}"), &Capability::Update))?;
    let entry = s.kv_v1.entry(path).or_insert_with(|| KVV1Entry::new(HashMap::new()));
    entry.update(data);
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_v1_delete(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    let policies = s.authenticate(&token)?;
    s.policy.check(&policies, &format!("kv/{path}"), &Capability::Delete)?;
    s.kv_v1.remove(&path);
    Ok(StatusCode::NO_CONTENT)
}

// ── Transit ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TransitCreateKeyRequest {
    name: String,
    #[serde(rename = "type")]
    key_type: Option<String>,
}

async fn transit_create_key(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<TransitCreateKeyRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let kt = match req.key_type.as_deref().unwrap_or("aes256-gcm96") {
        "aes256-gcm96" | "aes256-gcm" => TransitKeyType::Aes256Gcm,
        "ed25519"                      => TransitKeyType::Ed25519,
        "rsa-2048"                     => TransitKeyType::Rsa2048,
        other => return Err(VaultError::InvalidRequest(format!("unknown key type: {other}"))),
    };
    let entry = crate::transit::TransitKeyEntry::create(&req.name, kt)?;
    s.transit.insert(req.name, entry);
    Ok(StatusCode::NO_CONTENT)
}

async fn transit_get_key(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    Ok(Json(serde_json::to_value(&entry.meta).unwrap()))
}

async fn transit_delete_key(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    if !entry.meta.deletion_allowed {
        return Err(VaultError::InvalidRequest("deletion_allowed is false".into()));
    }
    s.transit.remove(&name);
    Ok(StatusCode::NO_CONTENT)
}

async fn transit_rotate_key(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get_mut(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    entry.rotate()?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TransitEncryptRequest {
    plaintext: String,  // base64-encoded
    context: Option<String>,
}

async fn transit_encrypt(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<TransitEncryptRequest>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    let pt = base64::engine::general_purpose::STANDARD
        .decode(&req.plaintext)
        .map_err(|_| VaultError::InvalidRequest("invalid base64 plaintext".into()))?;
    let ctx = req.context.as_ref()
        .map(|c| base64::engine::general_purpose::STANDARD.decode(c)
            .map_err(|_| VaultError::InvalidRequest("invalid base64 context".into())))
        .transpose()?;
    let ct = entry.encrypt(&pt, ctx.as_deref())?;
    Ok(Json(serde_json::json!({ "data": { "ciphertext": ct } })))
}

#[derive(Deserialize)]
struct TransitDecryptRequest {
    ciphertext: String,
    context: Option<String>,
}

async fn transit_decrypt(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<TransitDecryptRequest>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    let ctx = req.context.as_ref()
        .map(|c| base64::engine::general_purpose::STANDARD.decode(c)
            .map_err(|_| VaultError::InvalidRequest("invalid base64 context".into())))
        .transpose()?;
    let pt = entry.decrypt(&req.ciphertext, ctx.as_deref())?;
    Ok(Json(serde_json::json!({
        "data": { "plaintext": base64::engine::general_purpose::STANDARD.encode(&pt) }
    })))
}

async fn transit_rewrap(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<TransitDecryptRequest>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    let new_ct = entry.rewrap(&req.ciphertext, None)?;
    Ok(Json(serde_json::json!({ "data": { "ciphertext": new_ct } })))
}

#[derive(Deserialize)]
struct TransitSignRequest {
    input: String,   // base64
}

async fn transit_sign(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<TransitSignRequest>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(&req.input)
        .map_err(|_| VaultError::InvalidRequest("invalid base64".into()))?;
    let sig = entry.sign(&data)?;
    Ok(Json(serde_json::json!({ "data": { "signature": sig } })))
}

#[derive(Deserialize)]
struct TransitVerifyRequest {
    input: String,
    signature: String,
}

async fn transit_verify(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<TransitVerifyRequest>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(&req.input)
        .map_err(|_| VaultError::InvalidRequest("invalid base64".into()))?;
    let valid = entry.verify(&data, &req.signature)?;
    Ok(Json(serde_json::json!({ "data": { "valid": valid } })))
}

#[derive(Deserialize)]
struct DataKeyRequest {
    bits: Option<u32>,
}

async fn transit_datakey(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<DataKeyRequest>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.transit.get(&name)
        .ok_or_else(|| VaultError::KeyNotFound(name.clone()))?;
    let (pt, ct) = entry.generate_data_key(req.bits.unwrap_or(256))?;
    Ok(Json(serde_json::json!({
        "data": {
            "plaintext": base64::engine::general_purpose::STANDARD.encode(&pt),
            "ciphertext": ct,
        }
    })))
}

// ── PKI ───────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PkiRootRequest {
    common_name: String,
    organization: Option<String>,
    ttl_days: Option<i64>,
}

async fn pki_generate_root(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<PkiRootRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let pem = s.pki.generate_root_ca(
        &req.common_name,
        req.organization.as_deref().unwrap_or("CAVE"),
        req.ttl_days.unwrap_or(3650),
    )?;
    Ok(Json(serde_json::json!({ "data": { "certificate": pem } })))
}

async fn pki_generate_intermediate(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<PkiRootRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let chain = s.pki.generate_intermediate_ca(
        &req.common_name,
        req.organization.as_deref().unwrap_or("CAVE"),
        req.ttl_days.unwrap_or(1825),
    )?;
    Ok(Json(serde_json::json!({ "data": { "ca_chain": chain } })))
}

#[derive(Deserialize)]
struct PkiIssueRequest {
    common_name: String,
    alt_names: Option<Vec<String>>,
    ttl_days: Option<i64>,
    private_key_format: Option<String>,
}

async fn pki_issue(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<PkiIssueRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let include_key = req.private_key_format.as_deref().unwrap_or("pem") == "pem";
    let cert = s.pki.issue_certificate(
        &req.common_name,
        &req.alt_names.unwrap_or_default(),
        req.ttl_days.unwrap_or(30),
        include_key,
    )?;
    Ok(Json(serde_json::to_value(&cert).unwrap()))
}

#[derive(Deserialize)]
struct PkiRevokeRequest {
    serial_number: String,
}

async fn pki_revoke(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<PkiRevokeRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let ts = s.pki.revoke(&req.serial_number)?;
    Ok(Json(serde_json::json!({ "data": { "revocation_time": ts } })))
}

async fn pki_crl(State(state): State<SharedVaultState>) -> Json<Value> {
    let s = state.read().await;
    Json(s.pki.generate_crl())
}

async fn pki_ca(State(state): State<SharedVaultState>) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let ca = s.pki.root_ca.as_ref()
        .ok_or_else(|| VaultError::NotFound("root CA".into()))?;
    Ok(Json(serde_json::json!({ "data": { "certificate": ca.cert_pem } })))
}

async fn pki_cert(
    State(state): State<SharedVaultState>,
    Path(serial): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let cert = s.pki.certs.get(&serial)
        .ok_or_else(|| VaultError::NotFound(format!("serial {serial}")))?;
    Ok(Json(serde_json::to_value(cert).unwrap()))
}

// ── Database ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DbRoleRequest {
    db_name: String,
    db_type: Option<String>,
    creation_statements: Option<Vec<String>>,
    revocation_statements: Option<Vec<String>>,
    default_ttl: Option<u64>,
    max_ttl: Option<u64>,
}

async fn db_create_role(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<DbRoleRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let db_type = match req.db_type.as_deref().unwrap_or("postgresql") {
        "mysql"      => crate::database::DbType::Mysql,
        _            => crate::database::DbType::Postgresql,
    };
    s.db.configure_role(DbRole {
        name: name.clone(),
        db_name: req.db_name,
        db_type,
        creation_statements: req.creation_statements.unwrap_or_default(),
        revocation_statements: req.revocation_statements.unwrap_or_default(),
        default_ttl: req.default_ttl.unwrap_or(3600),
        max_ttl: req.max_ttl.unwrap_or(86400),
    });
    Ok(StatusCode::NO_CONTENT)
}

async fn db_get_role(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let s = state.read().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let role = s.db.roles.get(&name)
        .ok_or_else(|| VaultError::NotFound(format!("db role '{name}'")))?;
    Ok(Json(serde_json::to_value(role).unwrap()))
}

async fn db_generate_creds(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let creds = s.db.generate_credentials(&name)?;
    Ok(Json(serde_json::json!({
        "data": {
            "username": creds.username,
            "password": creds.password,
        },
        "lease_id": creds.lease.lease_id,
        "lease_duration": creds.lease.lease_duration,
        "renewable": creds.lease.renewable,
    })))
}

async fn db_revoke_creds(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Path(lease_id): Path<String>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    s.db.revoke_credentials(&lease_id)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Leases ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LeaseRenewRequest {
    lease_id: String,
    increment: Option<u64>,
}

async fn lease_renew(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<LeaseRenewRequest>,
) -> Result<Json<Value>, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    let entry = s.leases.renew(&req.lease_id, req.increment.unwrap_or(0))?;
    Ok(Json(serde_json::json!({
        "lease_id": entry.lease_id,
        "renewable": entry.renewable,
        "lease_duration": entry.remaining_secs(),
    })))
}

#[derive(Deserialize)]
struct LeaseRevokeRequest {
    lease_id: String,
}

async fn lease_revoke(
    State(state): State<SharedVaultState>,
    headers: HeaderMap,
    Json(req): Json<LeaseRevokeRequest>,
) -> Result<StatusCode, VaultError> {
    let mut s = state.write().await;
    let token = token_or_err(&headers)?;
    s.authenticate(&token)?;
    s.leases.revoke(&req.lease_id)?;
    Ok(StatusCode::NO_CONTENT)
}
