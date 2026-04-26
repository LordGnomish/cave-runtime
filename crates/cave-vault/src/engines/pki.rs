use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::VaultState;
use axum::{
    extract::{Json, Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{delete, get, post},
    Router,
};
use chrono::{Duration, Utc};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

fn chrono_to_time(dt: chrono::DateTime<Utc>) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(dt.timestamp())
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaInfo {
    pub cert_pem: String,
    pub private_key_pem: String,
    pub serial: String,
    pub not_before: String,
    pub not_after: String,
    pub is_intermediate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkiRole {
    pub name: String,
    pub allowed_domains: Vec<String>,
    pub allow_subdomains: bool,
    pub allow_glob_domains: bool,
    pub allow_any_name: bool,
    pub enforce_hostnames: bool,
    pub allow_ip_sans: bool,
    pub key_type: String,
    pub key_bits: u32,
    pub max_ttl: i64,
    pub ttl: i64,
    pub not_before_duration: i64,
    pub generate_lease: bool,
    pub no_store: bool,
    pub require_cn: bool,
    pub server_flag: bool,
    pub client_flag: bool,
    pub code_signing_flag: bool,
    pub email_protection_flag: bool,
    pub allowed_extensions: Vec<String>,
    pub ou: Vec<String>,
    pub organization: Vec<String>,
    pub country: Vec<String>,
}

impl Default for PkiRole {
    fn default() -> Self {
        Self {
            name: String::new(),
            allowed_domains: Vec::new(),
            allow_subdomains: false,
            allow_glob_domains: false,
            allow_any_name: false,
            enforce_hostnames: true,
            allow_ip_sans: true,
            key_type: "rsa".to_string(),
            key_bits: 2048,
            max_ttl: 0,
            ttl: 0,
            not_before_duration: 30,
            generate_lease: false,
            no_store: false,
            require_cn: true,
            server_flag: true,
            client_flag: true,
            code_signing_flag: false,
            email_protection_flag: false,
            allowed_extensions: Vec::new(),
            ou: Vec::new(),
            organization: Vec::new(),
            country: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuedCert {
    pub serial: String,
    pub cert_pem: String,
    pub ca_chain: Vec<String>,
    pub issued_at: String,
    pub expiry: String,
    pub revoked: bool,
    pub revoked_at: Option<String>,
}

#[derive(Default)]
pub struct PkiStore {
    pub ca: Option<CaInfo>,
    pub ca_key_pem: Option<String>, // kept separately for signing
    pub roles: HashMap<String, PkiRole>,
    pub certs: HashMap<String, IssuedCert>,
    pub crl_pem: String,
    pub urls_config: HashMap<String, String>,
}

pub async fn generate_root(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, gen_type)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;

    let common_name = body.get("common_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Vault CA");
    let ttl_secs = body.get("ttl")
        .and_then(|v| v.as_str())
        .map(crate::token::parse_duration)
        .unwrap_or(315_360_000);

    let ca_key = KeyPair::generate()
        .map_err(|e| VaultError::Pki(format!("key gen failed: {}", e)))?;

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, common_name);
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = chrono_to_time(Utc::now() + Duration::seconds(ttl_secs));

    let ca_cert = params.self_signed(&ca_key)
        .map_err(|e| VaultError::Pki(format!("cert gen failed: {}", e)))?;

    let cert_pem = ca_cert.pem();
    let key_pem = ca_key.serialize_pem();
    let serial = format!("{:x}", uuid::Uuid::new_v4().as_u128());
    let not_after_dt = Utc::now() + Duration::seconds(ttl_secs);

    let ca_info = CaInfo {
        cert_pem: cert_pem.clone(),
        private_key_pem: key_pem.clone(),
        serial: serial.clone(),
        not_before: Utc::now().to_rfc3339(),
        not_after: not_after_dt.to_rfc3339(),
        is_intermediate: false,
    };

    let mut store = state.pki_store.write().await;
    store.ca_key_pem = Some(key_pem.clone());
    store.ca = Some(ca_info);

    let mut response = json!({
        "serial_number": serial,
        "certificate": cert_pem,
        "issuing_ca": cert_pem,
        "expiration": not_after_dt.timestamp(),
    });

    if gen_type == "exported" {
        response["private_key"] = json!(key_pem);
        response["private_key_type"] = json!("ec");
    }

    Ok(VaultResponse::new().with_data(response))
}

pub async fn generate_intermediate(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((_mount, gen_type)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let common_name = body.get("common_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Vault Intermediate CA");

    let key = KeyPair::generate()
        .map_err(|e| VaultError::Pki(format!("key gen failed: {}", e)))?;

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, common_name);

    let csr = params.serialize_request(&key)
        .map_err(|e| VaultError::Pki(format!("CSR gen failed: {}", e)))?;
    let csr_pem = csr.pem()
        .map_err(|e| VaultError::Pki(format!("CSR PEM failed: {}", e)))?;

    let key_pem = key.serialize_pem();
    Ok(VaultResponse::new().with_data(json!({
        "csr": csr_pem,
        "private_key": if gen_type == "exported" { key_pem } else { String::new() },
        "private_key_type": "ec",
    })))
}

pub async fn sign_intermediate(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
    Json(_body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.pki_store.read().await;
    let ca = store.ca.as_ref().ok_or_else(|| VaultError::Pki("no CA configured".into()))?;
    Ok(VaultResponse::new().with_data(json!({
        "certificate": "signed-intermediate-cert-placeholder",
        "issuing_ca": ca.cert_pem.clone(),
        "serial_number": format!("{:x}", uuid::Uuid::new_v4().as_u128()),
    })))
}

pub async fn set_signed(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let cert_pem = body.get("certificate")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("certificate required".into()))?
        .to_string();
    let mut store = state.pki_store.write().await;
    let ca = store.ca.get_or_insert(CaInfo {
        cert_pem: cert_pem.clone(),
        private_key_pem: String::new(),
        serial: format!("{:x}", uuid::Uuid::new_v4().as_u128()),
        not_before: Utc::now().to_rfc3339(),
        not_after: (Utc::now() + Duration::days(365)).to_rfc3339(),
        is_intermediate: true,
    });
    ca.cert_pem = cert_pem;
    ca.is_intermediate = true;
    Ok(VaultResponse::new())
}

pub async fn get_ca_pem(
    State(state): State<Arc<VaultState>>,
    Path(_mount): Path<String>,
) -> Result<impl IntoResponse, VaultError> {
    let store = state.pki_store.read().await;
    let ca = store.ca.as_ref().ok_or_else(|| VaultError::Pki("no CA configured".into()))?;
    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/x-pem-file")],
        ca.cert_pem.clone(),
    ))
}

pub async fn get_ca_der(
    State(state): State<Arc<VaultState>>,
    Path(_mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let store = state.pki_store.read().await;
    let ca = store.ca.as_ref().ok_or_else(|| VaultError::Pki("no CA configured".into()))?;
    Ok(VaultResponse::new().with_data(json!({ "certificate": ca.cert_pem })))
}

pub async fn get_crl_pem(
    State(state): State<Arc<VaultState>>,
    Path(_mount): Path<String>,
) -> Result<impl IntoResponse, VaultError> {
    let store = state.pki_store.read().await;
    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/x-pem-file")],
        store.crl_pem.clone(),
    ))
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((_mount, role_name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.pki_store.write().await;
    let role = store.roles.entry(role_name.clone()).or_insert_with(|| PkiRole {
        name: role_name.clone(),
        ..Default::default()
    });
    if let Some(domains) = body.get("allowed_domains").and_then(|v| v.as_array()) {
        role.allowed_domains = domains.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(v) = body.get("allow_subdomains").and_then(|v| v.as_bool()) { role.allow_subdomains = v; }
    if let Some(v) = body.get("allow_any_name").and_then(|v| v.as_bool()) { role.allow_any_name = v; }
    if let Some(v) = body.get("key_type").and_then(|v| v.as_str()) { role.key_type = v.to_string(); }
    if let Some(v) = body.get("key_bits").and_then(|v| v.as_u64()) { role.key_bits = v as u32; }
    if let Some(ttl) = body.get("ttl").and_then(|v| v.as_str()) { role.ttl = crate::token::parse_duration(ttl); }
    if let Some(ttl) = body.get("max_ttl").and_then(|v| v.as_str()) { role.max_ttl = crate::token::parse_duration(ttl); }
    Ok(VaultResponse::new())
}

pub async fn read_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((_mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.pki_store.read().await;
    let role = store.roles.get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(role).unwrap_or_default()))
}

pub async fn delete_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((_mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.pki_store.write().await;
    store.roles.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.pki_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn issue_cert(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((_mount, role_name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;

    let common_name = body.get("common_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("common_name required".into()))?;

    let store = state.pki_store.read().await;
    let ca = store.ca.as_ref().ok_or_else(|| VaultError::Pki("no CA configured".into()))?.clone();
    let ca_key_pem = store.ca_key_pem.as_ref().ok_or_else(|| VaultError::Pki("no CA key available".into()))?.clone();
    let role = store.roles.get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name.clone()))?.clone();
    drop(store);

    if !role.allow_any_name {
        let domain_ok = role.allowed_domains.iter().any(|d| {
            if role.allow_subdomains && common_name.ends_with(&format!(".{}", d)) {
                return true;
            }
            common_name == d.as_str()
        });
        if !domain_ok {
            return Err(VaultError::Pki(format!("common_name {} not allowed by role", common_name)));
        }
    }

    let ttl_secs = body.get("ttl")
        .and_then(|v| v.as_str())
        .map(crate::token::parse_duration)
        .filter(|&t| t > 0)
        .unwrap_or_else(|| if role.ttl > 0 { role.ttl } else { 86400 });

    // Generate end-entity cert using rcgen
    let ee_key = KeyPair::generate()
        .map_err(|e| VaultError::Pki(format!("key gen failed: {}", e)))?;

    let mut san_names = vec![common_name.to_string()];
    if let Some(alt_names) = body.get("alt_names").and_then(|v| v.as_str()) {
        san_names.extend(alt_names.split(',').map(|s| s.trim().to_string()));
    }

    let mut ee_params = CertificateParams::new(san_names)
        .map_err(|e| VaultError::Pki(format!("cert params failed: {}", e)))?;
    ee_params.distinguished_name.push(DnType::CommonName, common_name);
    ee_params.not_before = OffsetDateTime::now_utc();
    ee_params.not_after = chrono_to_time(Utc::now() + Duration::seconds(ttl_secs));

    let not_after_dt = Utc::now() + Duration::seconds(ttl_secs);

    // Parse CA key for signing
    let ca_key = KeyPair::from_pem(&ca_key_pem)
        .map_err(|e| VaultError::Pki(format!("CA key parse failed: {}", e)))?;

    // Reconstruct CA cert params for signing
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.distinguished_name.push(DnType::CommonName, "Vault CA");
    let ca_cert_for_signing = ca_params.self_signed(&ca_key)
        .map_err(|e| VaultError::Pki(format!("CA cert reconstitution failed: {}", e)))?;

    let ee_cert = ee_params.signed_by(&ee_key, &ca_cert_for_signing, &ca_key)
        .map_err(|e| VaultError::Pki(format!("cert signing failed: {}", e)))?;

    let cert_pem = ee_cert.pem();
    let private_key_pem = ee_key.serialize_pem();
    let serial = format!("{:x}", uuid::Uuid::new_v4().as_u128());

    let issued = IssuedCert {
        serial: serial.clone(),
        cert_pem: cert_pem.clone(),
        ca_chain: vec![ca.cert_pem.clone()],
        issued_at: Utc::now().to_rfc3339(),
        expiry: not_after_dt.to_rfc3339(),
        revoked: false,
        revoked_at: None,
    };

    if !role.no_store {
        let mut store = state.pki_store.write().await;
        store.certs.insert(serial.clone(), issued);
    }

    Ok(VaultResponse::new().with_data(json!({
        "certificate": cert_pem,
        "issuing_ca": ca.cert_pem,
        "ca_chain": [ca.cert_pem],
        "private_key": private_key_pem,
        "private_key_type": "ec",
        "serial_number": serial,
        "expiration": not_after_dt.timestamp(),
    })))
}

pub async fn sign_cert(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(paths): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    issue_cert(State(state), headers, Path(paths), Json(body)).await
}

pub async fn revoke_cert(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let serial = body.get("serial_number")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("serial_number required".into()))?
        .to_string();

    let mut store = state.pki_store.write().await;
    if let Some(cert) = store.certs.get_mut(&serial) {
        cert.revoked = true;
        cert.revoked_at = Some(Utc::now().to_rfc3339());
        Ok(VaultResponse::new().with_data(json!({
            "revocation_time": Utc::now().timestamp(),
        })))
    } else {
        Err(VaultError::NotFound(format!("cert {} not found", serial)))
    }
}

pub async fn read_cert(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((_mount, serial)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.pki_store.read().await;
    let cert = store.certs.get(&serial)
        .ok_or_else(|| VaultError::NotFound(format!("cert {} not found", serial)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(cert).unwrap_or_default()))
}

pub async fn list_certs(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.pki_store.read().await;
    let keys: Vec<String> = store.certs.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn configure_urls(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
    Json(body): Json<HashMap<String, String>>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.pki_store.write().await;
    store.urls_config = body;
    Ok(VaultResponse::new())
}

pub async fn rotate_crl(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(_mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.pki_store.write().await;
    store.crl_pem = "-----BEGIN X509 CRL-----\n-----END X509 CRL-----\n".to_string();
    Ok(VaultResponse::new().with_data(json!({ "success": true })))
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    Router::new()
        .route(&format!("/v1/{}/root/generate/{{type}}", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(gen_type): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { generate_root(State(state), headers, Path((mount, gen_type)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/intermediate/generate/{{type}}", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(gen_type): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { generate_intermediate(State(state), headers, Path((mount, gen_type)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/intermediate/set-signed", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { set_signed(State(state), headers, Path(mount), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/root/sign-intermediate", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { sign_intermediate(State(state), headers, Path(mount), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/ca/pem", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move || {
                let state = s.clone();
                let mount = mount.clone();
                async move { get_ca_pem(State(state), Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/ca", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move || {
                let state = s.clone();
                let mount = mount.clone();
                async move { get_ca_der(State(state), Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/crl/pem", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move || {
                let state = s.clone();
                let mount = mount.clone();
                async move { get_crl_pem(State(state), Path(mount)).await }
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
        .route(&format!("/v1/{}/roles/{{role_name}}", mount), post({
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
        .route(&format!("/v1/{}/issue/{{role_name}}", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { issue_cert(State(state), headers, Path((mount, role_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/sign/{{role_name}}", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { sign_cert(State(state), headers, Path((mount, role_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/revoke", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { revoke_cert(State(state), headers, Path(mount), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/cert/{{serial}}", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(serial): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { read_cert(State(state), headers, Path((mount, serial))).await }
            }
        }))
        .route(&format!("/v1/{}/certs", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { list_certs(State(state), headers, Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/config/urls", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Json(body): Json<HashMap<String, String>>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { configure_urls(State(state), headers, Path(mount), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/crl/rotate", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { rotate_crl(State(state), headers, Path(mount)).await }
            }
        }))
        .with_state(state)
}
