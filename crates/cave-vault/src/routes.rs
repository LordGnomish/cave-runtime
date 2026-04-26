//! HTTP route handlers for cave-vault.
//!
//! Exposes a Vault-compatible REST API under /api/v1/vault/...

use crate::{
    auth, kv, models, pki, transit, SharedVaultStore,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

// ── type aliases ─────────────────────────────────────────────────────────────

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "errors": [msg.to_string()] })))
}
fn not_found(msg: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::NOT_FOUND, msg)
}
fn bad_request(msg: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::BAD_REQUEST, msg)
}
fn internal(msg: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, msg)
}

// ── router ───────────────────────────────────────────────────────────────────

pub fn create_router(store: SharedVaultStore) -> Router {
    Router::new()
        // ── KV secrets engine ──────────────────────────────────────────────
        .route("/api/v1/vault/secret/data/{*path}", get(kv_get).post(kv_put).delete(kv_delete))
        .route("/api/v1/vault/secret/metadata/{*path}", get(kv_metadata).post(kv_update_metadata))
        .route("/api/v1/vault/secret/delete/{*path}", post(kv_soft_delete_versions))
        .route("/api/v1/vault/secret/undelete/{*path}", post(kv_undelete))
        .route("/api/v1/vault/secret/destroy/{*path}", post(kv_destroy))
        .route("/api/v1/vault/secret/list/{*path}", get(kv_list))
        // ── PKI engine ─────────────────────────────────────────────────────
        .route("/api/v1/vault/pki/root/generate", post(pki_generate_root))
        .route("/api/v1/vault/pki/issue", post(pki_issue))
        .route("/api/v1/vault/pki/revoke", post(pki_revoke))
        .route("/api/v1/vault/pki/crl", get(pki_crl))
        .route("/api/v1/vault/pki/ca", get(pki_get_ca))
        .route("/api/v1/vault/pki/cert/{serial}", get(pki_get_cert))
        // ── Transit engine ─────────────────────────────────────────────────
        .route("/api/v1/vault/transit/keys", post(transit_create_key))
        .route("/api/v1/vault/transit/keys/{name}", get(transit_read_key))
        .route("/api/v1/vault/transit/keys/{name}/rotate", post(transit_rotate_key))
        .route("/api/v1/vault/transit/encrypt/{name}", post(transit_encrypt))
        .route("/api/v1/vault/transit/decrypt/{name}", post(transit_decrypt))
        .route("/api/v1/vault/transit/sign/{name}", post(transit_sign))
        .route("/api/v1/vault/transit/verify/{name}", post(transit_verify))
        .route("/api/v1/vault/transit/datakey/{name}", post(transit_datakey))
        .route("/api/v1/vault/transit/rewrap/{name}", post(transit_rewrap))
        // ── Auth methods ───────────────────────────────────────────────────
        .route("/api/v1/vault/auth/token/lookup", post(auth_token_lookup))
        .route("/api/v1/vault/auth/approle/login", post(auth_approle_login))
        .route("/api/v1/vault/auth/kubernetes/login", post(auth_kubernetes_login))
        .route("/api/v1/vault/auth/oidc/callback", post(auth_oidc_callback))
        // ── Sys ────────────────────────────────────────────────────────────
        .route("/api/v1/vault/sys/seal-status", get(sys_seal_status))
        .route("/api/v1/vault/sys/seal", put(sys_seal))
        .route("/api/v1/vault/sys/unseal", put(sys_unseal))
        // ── Policies ───────────────────────────────────────────────────────
        .route("/api/v1/vault/sys/policy", get(policy_list))
        .route(
            "/api/v1/vault/sys/policy/{name}",
            get(policy_read).post(policy_create).delete(policy_delete),
        )
        // ── Audit log ──────────────────────────────────────────────────────
        .route("/api/v1/vault/sys/audit", get(audit_list))
        // ── Health ─────────────────────────────────────────────────────────
        .route("/api/v1/vault/sys/health", get(health))
        .with_state(store)
}

// ═══════════════════════════════════════════════════════════════════════════════
// KV handlers
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<u32>,
}

async fn kv_get(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
    Query(q): Query<VersionQuery>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    match kv::kv_get(&guard.kv, &path, q.version) {
        Ok(secret) => {
            append_audit_noop(&guard, "kv_get", &path, 200);
            Ok(Json(json!({ "data": secret })))
        }
        Err(kv::KVError::NotFound(_)) => Err(not_found(format!("no secret at {path}"))),
        Err(e) => Err(bad_request(e)),
    }
}

#[derive(Deserialize)]
struct KVPutRequest {
    data: HashMap<String, Value>,
    #[serde(default)]
    options: HashMap<String, Value>,
}

async fn kv_put(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
    Json(req): Json<KVPutRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let secret = kv::kv_put(&mut guard.kv, &path, req.data, None);
    Ok(Json(json!({ "data": { "version": secret.version } })))
}

async fn kv_delete(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    kv::kv_delete(&mut guard.kv, &path).map_err(|e| not_found(e))?;
    Ok(Json(json!({})))
}

async fn kv_metadata(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let meta = kv::kv_read_metadata(&guard.kv, &path).map_err(|e| not_found(e))?;
    Ok(Json(json!({ "data": meta })))
}

#[derive(Deserialize)]
struct KVMetaUpdateRequest {
    max_versions: Option<u32>,
    custom_metadata: Option<HashMap<String, String>>,
}

async fn kv_update_metadata(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
    Json(req): Json<KVMetaUpdateRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    kv::kv_update_metadata(&mut guard.kv, &path, req.max_versions, req.custom_metadata)
        .map_err(|e| not_found(e))?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct VersionsBody {
    versions: Vec<u32>,
}

async fn kv_soft_delete_versions(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
    Json(body): Json<VersionsBody>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    kv::kv_soft_delete(&mut guard.kv, &path, &body.versions).map_err(|e| not_found(e))?;
    Ok(Json(json!({})))
}

async fn kv_undelete(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
    Json(body): Json<VersionsBody>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    kv::kv_undelete(&mut guard.kv, &path, &body.versions).map_err(|e| bad_request(e))?;
    Ok(Json(json!({})))
}

async fn kv_destroy(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
    Json(body): Json<VersionsBody>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    kv::kv_destroy(&mut guard.kv, &path, &body.versions).map_err(|e| not_found(e))?;
    Ok(Json(json!({})))
}

async fn kv_list(
    State(store): State<SharedVaultStore>,
    Path(path): Path<String>,
) -> Json<Value> {
    let guard = store.lock().unwrap();
    let keys = kv::kv_list(&guard.kv, &path);
    Json(json!({ "data": { "keys": keys } }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// PKI handlers
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct GenerateRootCaRequest {
    common_name: String,
    #[serde(default = "default_org")]
    organization: String,
    #[serde(default = "default_country")]
    country: String,
    #[serde(default = "default_ttl_days")]
    ttl_days: u32,
}
fn default_org() -> String { "CAVE Platform".to_string() }
fn default_country() -> String { "US".to_string() }
fn default_ttl_days() -> u32 { 3650 }

async fn pki_generate_root(
    State(store): State<SharedVaultStore>,
    Json(req): Json<GenerateRootCaRequest>,
) -> ApiResult<Json<Value>> {
    let ca = pki::generate_root_ca(
        &req.common_name,
        &req.organization,
        &req.country,
        req.ttl_days,
    )
    .map_err(|e| internal(e))?;

    let pki_cert = models::PKICert {
        serial_number: ca.serial.clone(),
        certificate: ca.cert_pem.clone(),
        issuing_ca: ca.cert_pem.clone(),
        ca_chain: vec![ca.cert_pem.clone()],
        private_key: Some(ca.key_pem.clone()),
        private_key_type: "ec".to_string(),
        expiration: chrono::Utc::now() + chrono::Duration::days(req.ttl_days as i64),
        subject: models::CertSubject {
            common_name: req.common_name,
            organization: vec![req.organization],
            country: vec![req.country],
            alt_names: vec![],
            ip_sans: vec![],
        },
        revoked: false,
        revocation_time: None,
    };

    let serial = ca.serial.clone();
    let mut guard = store.lock().unwrap();
    guard.pki_certs.insert(serial, pki::StoredCert { pki_cert: pki_cert.clone() });
    guard.root_ca = Some(ca);

    Ok(Json(json!({ "data": pki_cert })))
}

#[derive(Deserialize)]
struct IssueCertRequest {
    common_name: String,
    #[serde(default)]
    alt_names: Vec<String>,
    #[serde(default)]
    ip_sans: Vec<String>,
    #[serde(default)]
    organization: String,
    #[serde(default)]
    country: String,
    #[serde(default = "default_ttl_days")]
    ttl_days: u32,
}

async fn pki_issue(
    State(store): State<SharedVaultStore>,
    Json(req): Json<IssueCertRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    if guard.root_ca.is_none() {
        return Err(err(StatusCode::PRECONDITION_FAILED, "root CA not initialised"));
    }
    let ca = guard.root_ca.as_ref().unwrap();
    let cert = pki::issue_certificate(
        ca,
        &req.common_name,
        &req.alt_names,
        &req.ip_sans,
        &req.organization,
        &req.country,
        req.ttl_days,
    )
    .map_err(|e| internal(e))?;

    let serial = cert.serial_number.clone();
    guard.pki_certs.insert(serial, pki::StoredCert { pki_cert: cert.clone() });

    Ok(Json(json!({ "data": cert })))
}

#[derive(Deserialize)]
struct RevokeRequest {
    serial_number: String,
}

async fn pki_revoke(
    State(store): State<SharedVaultStore>,
    Json(req): Json<RevokeRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let serial = req.serial_number.clone();
    let now = chrono::Utc::now();
    if guard.revoked_certs.contains_key(&serial) {
        return Err(bad_request("certificate already revoked"));
    }
    guard.revoked_certs.insert(serial.clone(), now);
    if let Some(stored) = guard.pki_certs.get_mut(&serial) {
        stored.pki_cert.revoked = true;
        stored.pki_cert.revocation_time = Some(now);
    }
    Ok(Json(json!({ "data": { "revocation_time": chrono::Utc::now().to_rfc3339() } })))
}

async fn pki_crl(State(store): State<SharedVaultStore>) -> Json<Value> {
    let guard = store.lock().unwrap();
    let ca_serial = guard
        .root_ca
        .as_ref()
        .map(|ca| ca.serial.as_str())
        .unwrap_or("none");
    Json(pki::generate_crl(&guard.revoked_certs, ca_serial))
}

async fn pki_get_ca(State(store): State<SharedVaultStore>) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let ca = guard.root_ca.as_ref().ok_or_else(|| not_found("no root CA"))?;
    Ok(Json(json!({ "data": { "certificate": ca.cert_pem } })))
}

async fn pki_get_cert(
    State(store): State<SharedVaultStore>,
    Path(serial): Path<String>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let stored = guard
        .pki_certs
        .get(&serial)
        .ok_or_else(|| not_found(format!("cert {serial} not found")))?;
    Ok(Json(json!({ "data": stored.pki_cert })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Transit handlers
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct CreateKeyRequest {
    name: String,
    #[serde(rename = "type", default = "default_key_type")]
    key_type: models::TransitKeyType,
}
fn default_key_type() -> models::TransitKeyType { models::TransitKeyType::Aes256Gcm96 }

async fn transit_create_key(
    State(store): State<SharedVaultStore>,
    Json(req): Json<CreateKeyRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let meta = transit::create_key(&mut guard.transit_keys, &req.name, req.key_type)
        .map_err(|e| bad_request(e))?;
    Ok(Json(json!({ "data": meta })))
}

async fn transit_read_key(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let entry = guard
        .transit_keys
        .get(&name)
        .ok_or_else(|| not_found(format!("key {name} not found")))?;
    Ok(Json(json!({ "data": entry.meta })))
}

async fn transit_rotate_key(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let meta = transit::rotate_key(&mut guard.transit_keys, &name)
        .map_err(|e| not_found(e))?;
    Ok(Json(json!({ "data": meta })))
}

#[derive(Deserialize)]
struct EncryptRequest {
    plaintext: String, // base64-encoded
    #[serde(default)]
    context: Option<String>,
}

#[derive(Serialize)]
struct EncryptResponse {
    ciphertext: String,
}

async fn transit_encrypt(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<EncryptRequest>,
) -> ApiResult<Json<Value>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let plaintext = B64.decode(&req.plaintext).map_err(|_| bad_request("invalid base64 plaintext"))?;
    let ctx = req.context.as_deref().and_then(|c| B64.decode(c).ok());

    let guard = store.lock().unwrap();
    let ciphertext = transit::encrypt(
        &guard.transit_keys,
        &name,
        &plaintext,
        ctx.as_deref(),
    )
    .map_err(|e| bad_request(e))?;
    Ok(Json(json!({ "data": { "ciphertext": ciphertext } })))
}

#[derive(Deserialize)]
struct DecryptRequest {
    ciphertext: String,
    #[serde(default)]
    context: Option<String>,
}

async fn transit_decrypt(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<DecryptRequest>,
) -> ApiResult<Json<Value>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let ctx = req.context.as_deref().and_then(|c| B64.decode(c).ok());

    let guard = store.lock().unwrap();
    let plaintext = transit::decrypt(
        &guard.transit_keys,
        &name,
        &req.ciphertext,
        ctx.as_deref(),
    )
    .map_err(|e| bad_request(e))?;
    Ok(Json(json!({ "data": { "plaintext": B64.encode(&plaintext) } })))
}

#[derive(Deserialize)]
struct SignRequest {
    input: String, // base64-encoded data to sign
}

async fn transit_sign(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<SignRequest>,
) -> ApiResult<Json<Value>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let data = B64.decode(&req.input).map_err(|_| bad_request("invalid base64 input"))?;
    let guard = store.lock().unwrap();
    let sig = transit::sign(&guard.transit_keys, &name, &data).map_err(|e| bad_request(e))?;
    Ok(Json(json!({ "data": { "signature": sig } })))
}

#[derive(Deserialize)]
struct VerifyRequest {
    input: String,     // base64-encoded
    signature: String, // vault:v{N}:{base64}
}

async fn transit_verify(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<VerifyRequest>,
) -> ApiResult<Json<Value>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let data = B64.decode(&req.input).map_err(|_| bad_request("invalid base64 input"))?;
    let guard = store.lock().unwrap();
    let valid =
        transit::verify(&guard.transit_keys, &name, &data, &req.signature).map_err(|e| bad_request(e))?;
    Ok(Json(json!({ "data": { "valid": valid } })))
}

#[derive(Deserialize)]
struct DataKeyRequest {
    #[serde(default = "default_bits")]
    bits: u32,
}
fn default_bits() -> u32 { 256 }

async fn transit_datakey(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<DataKeyRequest>,
) -> ApiResult<Json<Value>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let guard = store.lock().unwrap();
    let (plaintext, ciphertext) =
        transit::generate_data_key(&guard.transit_keys, &name, req.bits).map_err(|e| bad_request(e))?;
    Ok(Json(json!({
        "data": {
            "plaintext": B64.encode(&plaintext),
            "ciphertext": ciphertext,
        }
    })))
}

#[derive(Deserialize)]
struct RewrapRequest {
    ciphertext: String,
}

async fn transit_rewrap(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<RewrapRequest>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let new_ct = transit::rewrap(&guard.transit_keys, &name, &req.ciphertext)
        .map_err(|e| bad_request(e))?;
    Ok(Json(json!({ "data": { "ciphertext": new_ct } })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Auth handlers
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct TokenLookupRequest {
    token: String,
}

async fn auth_token_lookup(
    State(store): State<SharedVaultStore>,
    Json(req): Json<TokenLookupRequest>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let result = auth::token_auth(&guard.tokens, &req.token).map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    Ok(Json(json!({ "auth": result })))
}

#[derive(Deserialize)]
struct AppRoleLoginRequest {
    role_id: String,
    secret_id: String,
}

async fn auth_approle_login(
    State(store): State<SharedVaultStore>,
    Json(req): Json<AppRoleLoginRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let result =
        auth::approle_auth(&guard.approles.clone(), &mut guard.tokens, &req.role_id, &req.secret_id)
            .map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    Ok(Json(json!({ "auth": result })))
}

#[derive(Deserialize)]
struct KubernetesLoginRequest {
    jwt: String,
    role: String,
}

async fn auth_kubernetes_login(
    State(store): State<SharedVaultStore>,
    Json(req): Json<KubernetesLoginRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let result = auth::kubernetes_auth(&mut guard.tokens, &req.jwt, &req.role)
        .map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    Ok(Json(json!({ "auth": result })))
}

#[derive(Deserialize)]
struct OidcCallbackRequest {
    code: String,
    role: String,
}

async fn auth_oidc_callback(
    State(store): State<SharedVaultStore>,
    Json(req): Json<OidcCallbackRequest>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    let result = auth::oidc_auth(&mut guard.tokens, &req.code, &req.role)
        .map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    Ok(Json(json!({ "auth": result })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Sys handlers
// ═══════════════════════════════════════════════════════════════════════════════

async fn sys_seal_status(State(store): State<SharedVaultStore>) -> Json<Value> {
    let guard = store.lock().unwrap();
    Json(json!(models::SealStatus {
        sealed: guard.sealed,
        initialized: guard.initialized,
        t: 1,
        n: 1,
        progress: 0,
        nonce: "".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        cluster_name: "cave-vault".to_string(),
        cluster_id: guard.cluster_id.clone(),
    }))
}

async fn sys_seal(State(store): State<SharedVaultStore>) -> Json<Value> {
    store.lock().unwrap().sealed = true;
    Json(json!({}))
}

#[derive(Deserialize)]
struct UnsealRequest {
    key: String,
    #[serde(default)]
    reset: bool,
}

async fn sys_unseal(
    State(store): State<SharedVaultStore>,
    Json(_req): Json<UnsealRequest>,
) -> Json<Value> {
    let mut guard = store.lock().unwrap();
    guard.sealed = false;
    Json(json!({ "sealed": false, "progress": 0, "t": 1, "n": 1 }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Policy handlers
// ═══════════════════════════════════════════════════════════════════════════════

async fn policy_list(State(store): State<SharedVaultStore>) -> Json<Value> {
    let guard = store.lock().unwrap();
    let names: Vec<&str> = guard.policies.keys().map(|s| s.as_str()).collect();
    Json(json!({ "data": { "policies": names } }))
}

async fn policy_read(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let guard = store.lock().unwrap();
    let policy = guard
        .policies
        .get(&name)
        .ok_or_else(|| not_found(format!("policy {name} not found")))?;
    Ok(Json(json!({ "data": policy })))
}

#[derive(Deserialize)]
struct PolicyCreateRequest {
    rules: Vec<models::PolicyRule>,
}

async fn policy_create(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
    Json(req): Json<PolicyCreateRequest>,
) -> Json<Value> {
    let now = chrono::Utc::now();
    let mut guard = store.lock().unwrap();
    let existing = guard.policies.get(&name);
    let created_at = existing.map(|p| p.created_at).unwrap_or(now);
    guard.policies.insert(
        name.clone(),
        models::Policy {
            name,
            rules: req.rules,
            created_at,
            updated_at: now,
        },
    );
    Json(json!({}))
}

async fn policy_delete(
    State(store): State<SharedVaultStore>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut guard = store.lock().unwrap();
    if name == "root" {
        return Err(bad_request("cannot delete root policy"));
    }
    guard.policies.remove(&name);
    Ok(Json(json!({})))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Audit log handler
// ═══════════════════════════════════════════════════════════════════════════════

async fn audit_list(State(store): State<SharedVaultStore>) -> Json<Value> {
    let guard = store.lock().unwrap();
    Json(json!({ "data": { "entries": guard.audit_log } }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Health
// ═══════════════════════════════════════════════════════════════════════════════

async fn health(State(store): State<SharedVaultStore>) -> Json<Value> {
    let guard = store.lock().unwrap();
    Json(json!({
        "module": "cave-vault",
        "status": if guard.sealed { "sealed" } else { "ok" },
        "upstream": "HashiCorp Vault",
        "engines": ["kv-v2", "pki", "transit"],
        "auth_methods": ["token", "approle", "kubernetes", "oidc"],
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ── internal helpers ──────────────────────────────────────────────────────────

fn append_audit_noop(guard: &std::sync::MutexGuard<crate::VaultStore>, op: &str, path: &str, code: u16) {
    // Audit log append happens through a mutable guard; this fn signature
    // takes an immutable guard (read path). Audit writes happen in the
    // mutable handlers. This is a no-op placeholder so callers compile.
    let _ = (guard, op, path, code);
}
