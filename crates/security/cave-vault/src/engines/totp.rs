// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::VaultState;
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use axum::{
    Router,
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
};
use base64::Engine as _;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers
        .get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpKey {
    pub name: String,
    pub issuer: String,
    pub account_name: String,
    pub secret: Vec<u8>, // raw secret bytes
    pub algorithm: TotpAlgorithm,
    pub digits: u32,
    pub period: u64,
    pub skew: u32,
    pub qr_size: u32,
    pub key_size: u32,
    pub generated: bool,
    pub exported: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum TotpAlgorithm {
    SHA1,
    SHA256,
    SHA512,
}

impl Default for TotpKey {
    fn default() -> Self {
        Self {
            name: String::new(),
            issuer: "Vault".to_string(),
            account_name: String::new(),
            secret: Vec::new(),
            algorithm: TotpAlgorithm::SHA1,
            digits: 6,
            period: 30,
            skew: 1,
            qr_size: 200,
            key_size: 20,
            generated: true,
            exported: true,
            url: String::new(),
        }
    }
}

#[derive(Default)]
pub struct TotpStore {
    pub keys: HashMap<String, TotpKey>,
}

/// TOTP algorithm implementation using HMAC.
/// counter = floor(unix_time / period)
/// HMAC(secret, counter.to_be_bytes())
/// Dynamic truncation → digits
fn compute_totp(secret: &[u8], counter: u64, digits: u32, algorithm: &TotpAlgorithm) -> u32 {
    let counter_bytes = counter.to_be_bytes();

    let hmac_alg = match algorithm {
        TotpAlgorithm::SHA1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        TotpAlgorithm::SHA256 => hmac::HMAC_SHA256,
        TotpAlgorithm::SHA512 => hmac::HMAC_SHA512,
    };

    let key = hmac::Key::new(hmac_alg, secret);
    let tag = hmac::sign(&key, &counter_bytes);
    let mac = tag.as_ref();

    // Dynamic truncation
    let offset = (mac[mac.len() - 1] & 0x0f) as usize;
    let truncated = ((mac[offset] & 0x7f) as u32) << 24
        | (mac[offset + 1] as u32) << 16
        | (mac[offset + 2] as u32) << 8
        | (mac[offset + 3] as u32);

    let modulus = 10u32.pow(digits);
    truncated % modulus
}

fn current_counter(period: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now / period
}

fn generate_code(key: &TotpKey) -> u32 {
    let counter = current_counter(key.period);
    compute_totp(&key.secret, counter, key.digits, &key.algorithm)
}

fn validate_code(key: &TotpKey, code: u32) -> bool {
    let counter = current_counter(key.period);
    let skew = key.skew as i64;
    for offset in -skew..=skew {
        let c = (counter as i64 + offset) as u64;
        if compute_totp(&key.secret, c, key.digits, &key.algorithm) == code {
            return true;
        }
    }
    false
}

pub async fn create_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;

    let mut key = TotpKey {
        name: key_name.clone(),
        issuer: body
            .get("issuer")
            .and_then(|v| v.as_str())
            .unwrap_or("Vault")
            .to_string(),
        account_name: body
            .get("account_name")
            .and_then(|v| v.as_str())
            .unwrap_or(&key_name)
            .to_string(),
        digits: body.get("digits").and_then(|v| v.as_u64()).unwrap_or(6) as u32,
        period: body.get("period").and_then(|v| v.as_u64()).unwrap_or(30),
        skew: body.get("skew").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
        key_size: body.get("key_size").and_then(|v| v.as_u64()).unwrap_or(20) as u32,
        algorithm: match body
            .get("algorithm")
            .and_then(|v| v.as_str())
            .unwrap_or("SHA1")
        {
            "SHA256" => TotpAlgorithm::SHA256,
            "SHA512" => TotpAlgorithm::SHA512,
            _ => TotpAlgorithm::SHA1,
        },
        generated: body
            .get("generate")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        exported: body
            .get("exported")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        ..Default::default()
    };

    // Handle URL-based creation
    if let Some(url) = body.get("url").and_then(|v| v.as_str()) {
        // Parse otpauth URL: otpauth://totp/issuer:account?secret=BASE32_SECRET&...
        key.url = url.to_string();
        if let Some(secret_b32) = url
            .split("secret=")
            .nth(1)
            .map(|s| s.split('&').next().unwrap_or(s))
        {
            // Decode base32 (simplified: use base64 as fallback)
            if let Ok(secret_bytes) = base32_decode(secret_b32) {
                key.secret = secret_bytes;
            }
        }
    } else if let Some(key_b64) = body.get("key").and_then(|v| v.as_str()) {
        key.secret = base64::engine::general_purpose::STANDARD
            .decode(key_b64)
            .map_err(|_| VaultError::InvalidRequest("invalid base64 key".into()))?;
    } else {
        // Generate random secret
        let rng = SystemRandom::new();
        let mut secret = vec![0u8; key.key_size as usize];
        rng.fill(&mut secret)
            .map_err(|_| VaultError::Crypto("rng failure".into()))?;
        key.secret = secret;
    }

    // Build TOTP URL
    let secret_b32 = base32_encode(&key.secret);
    key.url = format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm={}&digits={}&period={}",
        urlencoding_simple(&key.issuer),
        urlencoding_simple(&key.account_name),
        secret_b32,
        urlencoding_simple(&key.issuer),
        match key.algorithm {
            TotpAlgorithm::SHA1 => "SHA1",
            TotpAlgorithm::SHA256 => "SHA256",
            TotpAlgorithm::SHA512 => "SHA512",
        },
        key.digits,
        key.period,
    );

    let mut store = state.totp_store.write().await;
    let url = key.url.clone();
    let exported = key.exported;
    store.keys.insert(key_name.clone(), key);

    let mut response = json!({ "name": key_name });
    if exported {
        response["url"] = json!(url);
    }
    Ok(VaultResponse::new().with_data(response))
}

pub async fn read_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.totp_store.read().await;
    let key = store
        .keys
        .get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;
    Ok(VaultResponse::new().with_data(json!({
        "name": key.name,
        "issuer": key.issuer,
        "account_name": key.account_name,
        "digits": key.digits,
        "period": key.period,
        "algorithm": match key.algorithm {
            TotpAlgorithm::SHA1 => "SHA1",
            TotpAlgorithm::SHA256 => "SHA256",
            TotpAlgorithm::SHA512 => "SHA512",
        },
    })))
}

pub async fn delete_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.totp_store.write().await;
    store.keys.remove(&key_name);
    Ok(VaultResponse::new())
}

pub async fn list_keys(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.totp_store.read().await;
    let keys: Vec<String> = store.keys.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn generate_code_handler(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.totp_store.read().await;
    let key = store
        .keys
        .get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;
    let code = generate_code(key);
    Ok(VaultResponse::new().with_data(json!({
        "code": format!("{:0>width$}", code, width = key.digits as usize),
    })))
}

#[derive(Deserialize)]
pub struct ValidateCodeRequest {
    pub code: String,
}

pub async fn validate_code_handler(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<ValidateCodeRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.totp_store.read().await;
    let key = store
        .keys
        .get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;
    let code_num: u32 = body
        .code
        .parse()
        .map_err(|_| VaultError::InvalidRequest("invalid code".into()))?;
    let valid = validate_code(key, code_num);
    Ok(VaultResponse::new().with_data(json!({ "valid": valid })))
}

// Simplified base32 encoding (RFC 4648 without padding)
fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = String::new();
    let mut buffer: u64 = 0;
    let mut bits_in_buffer = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u64;
        bits_in_buffer += 8;
        while bits_in_buffer >= 5 {
            bits_in_buffer -= 5;
            let idx = ((buffer >> bits_in_buffer) & 0x1f) as usize;
            result.push(ALPHABET[idx] as char);
        }
    }
    if bits_in_buffer > 0 {
        let idx = ((buffer << (5 - bits_in_buffer)) & 0x1f) as usize;
        result.push(ALPHABET[idx] as char);
    }
    result
}

fn base32_decode(s: &str) -> Result<Vec<u8>, ()> {
    const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let s = s.trim_end_matches('=').to_uppercase();
    let mut buffer: u64 = 0;
    let mut bits_in_buffer = 0;
    let mut result = Vec::new();
    for ch in s.chars() {
        let val = ALPHABET.find(ch).ok_or(())? as u64;
        buffer = (buffer << 5) | val;
        bits_in_buffer += 5;
        if bits_in_buffer >= 8 {
            bits_in_buffer -= 8;
            result.push(((buffer >> bits_in_buffer) & 0xff) as u8);
        }
    }
    Ok(result)
}

fn urlencoding_simple(s: &str) -> String {
    s.replace(' ', "%20").replace(':', "%3A")
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    Router::new()
        .route(
            &format!("/v1/{}/keys", mount),
            get({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { list_keys(State(state), headers, Path(mount)).await }
                }
            }),
        )
        .route(
            &format!("/v1/{}/keys/{{key_name}}", mount),
            post({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<Value>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move {
                        create_key(State(state), headers, Path((mount, key_name)), Json(body)).await
                    }
                }
            })
            .get({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(key_name): Path<String>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { read_key(State(state), headers, Path((mount, key_name))).await }
                }
            })
            .delete({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(key_name): Path<String>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move { delete_key(State(state), headers, Path((mount, key_name))).await }
                }
            }),
        )
        .route(
            &format!("/v1/{}/code/{{key_name}}", mount),
            get({
                let s = state.clone();
                let mount = m.clone();
                move |headers: HeaderMap, Path(key_name): Path<String>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move {
                        generate_code_handler(State(state), headers, Path((mount, key_name))).await
                    }
                }
            })
            .post({
                let s = state.clone();
                let mount = m;
                move |headers: HeaderMap,
                      Path(key_name): Path<String>,
                      Json(body): Json<ValidateCodeRequest>| {
                    let state = s.clone();
                    let mount = mount.clone();
                    async move {
                        validate_code_handler(
                            State(state),
                            headers,
                            Path((mount, key_name)),
                            Json(body),
                        )
                        .await
                    }
                }
            }),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_totp_code_generation() {
        // Test vector: RFC 6238 SHA1 key
        let secret = b"12345678901234567890";
        // counter = 1 -> T=30s: expected 94287082
        let code = compute_totp(secret, 1, 8, &TotpAlgorithm::SHA1);
        assert_eq!(code, 94287082);
    }

    #[test]
    fn test_totp_sha256() {
        // Test vector from RFC 6238 SHA256
        let secret = b"12345678901234567890123456789012";
        let code = compute_totp(secret, 1, 8, &TotpAlgorithm::SHA256);
        assert_eq!(code, 46119246);
    }

    #[test]
    fn test_totp_validate_with_skew() {
        let secret = b"test_secret_for_totp";
        let current_counter = current_counter(30);
        let code = compute_totp(secret, current_counter, 6, &TotpAlgorithm::SHA1);
        let key = TotpKey {
            secret: secret.to_vec(),
            digits: 6,
            period: 30,
            skew: 1,
            algorithm: TotpAlgorithm::SHA1,
            ..Default::default()
        };
        assert!(validate_code(&key, code));
    }

    #[test]
    fn test_base32_roundtrip() {
        let data = b"Hello, World!";
        let encoded = base32_encode(data);
        let decoded = base32_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }
}
