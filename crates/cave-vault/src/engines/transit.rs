use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::VaultState;
use axum::{
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
    Router,
};
use base64::Engine as _;
use ring::aead;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KeyType {
    #[serde(rename = "aes256-gcm96")]
    Aes256Gcm96,
    #[serde(rename = "chacha20-poly1305")]
    Chacha20Poly1305,
    Ed25519,
    #[serde(rename = "ecdsa-p256")]
    EcdsaP256,
    #[serde(rename = "rsa-2048")]
    Rsa2048,
    #[serde(rename = "rsa-4096")]
    Rsa4096,
}

impl KeyType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "aes256-gcm96" | "aes256_gcm96" => Some(Self::Aes256Gcm96),
            "chacha20-poly1305" | "chacha20_poly1305" => Some(Self::Chacha20Poly1305),
            "ed25519" => Some(Self::Ed25519),
            "ecdsa-p256" | "ecdsa_p256" => Some(Self::EcdsaP256),
            "rsa-2048" | "rsa_2048" => Some(Self::Rsa2048),
            "rsa-4096" | "rsa_4096" => Some(Self::Rsa4096),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyVersion {
    pub version: u64,
    pub key_bytes: Vec<u8>, // raw key material for symmetric, or private key bytes for asymmetric
    pub creation_time: chrono::DateTime<chrono::Utc>,
    pub public_key: Option<String>, // PEM for asymmetric keys
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitKey {
    pub name: String,
    pub key_type: KeyType,
    pub versions: HashMap<u64, KeyVersion>,
    pub latest_version: u64,
    pub min_decryption_version: u64,
    pub min_encryption_version: u64,
    pub deletion_allowed: bool,
    pub exportable: bool,
    pub allow_plaintext_backup: bool,
    pub derived: bool,
    pub convergent_encryption: bool,
}

#[derive(Default)]
pub struct TransitStore {
    pub keys: HashMap<String, TransitKey>,
}

fn generate_aes256_key() -> VaultResult<Vec<u8>> {
    let rng = SystemRandom::new();
    let mut key = vec![0u8; 32];
    rng.fill(&mut key).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    Ok(key)
}

fn generate_ed25519_key() -> VaultResult<(Vec<u8>, String)> {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| VaultError::Crypto("key generation failed".into()))?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| VaultError::Crypto("key decode failed".into()))?;
    let public_key_b64 = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
    Ok((pkcs8.as_ref().to_vec(), public_key_b64))
}

fn create_transit_key(name: &str, key_type: KeyType) -> VaultResult<TransitKey> {
    let (key_bytes, public_key) = match &key_type {
        KeyType::Aes256Gcm96 | KeyType::Chacha20Poly1305 => {
            (generate_aes256_key()?, None)
        }
        KeyType::Ed25519 => {
            let (kb, pk) = generate_ed25519_key()?;
            (kb, Some(pk))
        }
        KeyType::EcdsaP256 | KeyType::Rsa2048 | KeyType::Rsa4096 => {
            // For these, use a placeholder key (real impl would use ring/rsa crates)
            let rng = SystemRandom::new();
            let mut key = vec![0u8; 32];
            rng.fill(&mut key).map_err(|_| VaultError::Crypto("rng failure".into()))?;
            (key, Some("placeholder-public-key".to_string()))
        }
    };

    let version = KeyVersion {
        version: 1,
        key_bytes,
        creation_time: chrono::Utc::now(),
        public_key: public_key.clone(),
    };
    let mut versions = HashMap::new();
    versions.insert(1u64, version);

    Ok(TransitKey {
        name: name.to_string(),
        key_type,
        versions,
        latest_version: 1,
        min_decryption_version: 1,
        min_encryption_version: 0,
        deletion_allowed: false,
        exportable: false,
        allow_plaintext_backup: false,
        derived: false,
        convergent_encryption: false,
    })
}

fn aes256_gcm_encrypt(key: &[u8], plaintext: &[u8]) -> VaultResult<Vec<u8>> {
    let rng = SystemRandom::new();
    let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| VaultError::Crypto("key creation failed".into()))?;
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
    let key = aead::LessSafeKey::new(unbound_key);
    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| VaultError::Crypto("encryption failed".into()))?;
    let mut result = nonce_bytes.to_vec();
    result.extend_from_slice(&in_out);
    Ok(result)
}

fn aes256_gcm_decrypt(key: &[u8], ciphertext: &[u8]) -> VaultResult<Vec<u8>> {
    if ciphertext.len() < 12 {
        return Err(VaultError::Crypto("ciphertext too short".into()));
    }
    let (nonce_bytes, encrypted) = ciphertext.split_at(12);
    let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| VaultError::Crypto("key creation failed".into()))?;
    let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| VaultError::Crypto("nonce error".into()))?;
    let key = aead::LessSafeKey::new(unbound_key);
    let mut in_out = encrypted.to_vec();
    let plaintext = key.open_in_place(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| VaultError::Crypto("decryption failed".into()))?;
    Ok(plaintext.to_vec())
}

fn chacha20_encrypt(key_bytes: &[u8], plaintext: &[u8]) -> VaultResult<Vec<u8>> {
    let rng = SystemRandom::new();
    let unbound_key = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key_bytes)
        .map_err(|_| VaultError::Crypto("key creation failed".into()))?;
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
    let key = aead::LessSafeKey::new(unbound_key);
    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| VaultError::Crypto("encryption failed".into()))?;
    let mut result = nonce_bytes.to_vec();
    result.extend_from_slice(&in_out);
    Ok(result)
}

fn chacha20_decrypt(key_bytes: &[u8], ciphertext: &[u8]) -> VaultResult<Vec<u8>> {
    if ciphertext.len() < 12 {
        return Err(VaultError::Crypto("ciphertext too short".into()));
    }
    let (nonce_bytes, encrypted) = ciphertext.split_at(12);
    let unbound_key = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key_bytes)
        .map_err(|_| VaultError::Crypto("key creation failed".into()))?;
    let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| VaultError::Crypto("nonce error".into()))?;
    let key = aead::LessSafeKey::new(unbound_key);
    let mut in_out = encrypted.to_vec();
    let plaintext = key.open_in_place(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| VaultError::Crypto("decryption failed".into()))?;
    Ok(plaintext.to_vec())
}

pub async fn create_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let key_type_str = body.get("type").and_then(|v| v.as_str()).unwrap_or("aes256-gcm96");
    let key_type = KeyType::from_str(key_type_str)
        .ok_or_else(|| VaultError::InvalidRequest(format!("unknown key type: {}", key_type_str)))?;

    let mut store = state.transit_store.write().await;
    if store.keys.contains_key(&key_name) {
        return Ok(VaultResponse::new()); // idempotent
    }
    let mut key = create_transit_key(&key_name, key_type)?;
    if let Some(exportable) = body.get("exportable").and_then(|v| v.as_bool()) {
        key.exportable = exportable;
    }
    if let Some(deletion_allowed) = body.get("deletion_allowed").and_then(|v| v.as_bool()) {
        key.deletion_allowed = deletion_allowed;
    }
    store.keys.insert(key_name, key);
    Ok(VaultResponse::new())
}

pub async fn read_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;
    let keys_info: HashMap<String, Value> = key.versions.iter().map(|(v, kv)| {
        (v.to_string(), json!({
            "name": key.name,
            "creation_time": kv.creation_time.to_rfc3339(),
            "public_key": kv.public_key,
        }))
    }).collect();
    Ok(VaultResponse::new().with_data(json!({
        "name": key.name,
        "type": format!("{:?}", key.key_type).to_lowercase(),
        "latest_version": key.latest_version,
        "min_decryption_version": key.min_decryption_version,
        "min_encryption_version": key.min_encryption_version,
        "deletion_allowed": key.deletion_allowed,
        "exportable": key.exportable,
        "keys": keys_info,
    })))
}

pub async fn delete_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.transit_store.write().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name.clone()))?;
    if !key.deletion_allowed {
        return Err(VaultError::InvalidRequest("key deletion not allowed".into()));
    }
    store.keys.remove(&key_name);
    Ok(VaultResponse::new())
}

pub async fn list_keys(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.transit_store.read().await;
    let keys: Vec<String> = store.keys.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn rotate_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.transit_store.write().await;
    let key = store.keys.get_mut(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;
    let new_version = key.latest_version + 1;
    let (key_bytes, public_key) = match &key.key_type {
        KeyType::Aes256Gcm96 | KeyType::Chacha20Poly1305 => (generate_aes256_key()?, None),
        KeyType::Ed25519 => {
            let (kb, pk) = generate_ed25519_key()?;
            (kb, Some(pk))
        }
        _ => {
            let rng = SystemRandom::new();
            let mut k = vec![0u8; 32];
            rng.fill(&mut k).map_err(|_| VaultError::Crypto("rng failure".into()))?;
            (k, Some("placeholder-public-key".to_string()))
        }
    };
    key.versions.insert(new_version, KeyVersion {
        version: new_version,
        key_bytes,
        creation_time: chrono::Utc::now(),
        public_key,
    });
    key.latest_version = new_version;
    Ok(VaultResponse::new())
}

#[derive(Deserialize)]
pub struct EncryptRequest {
    pub plaintext: String, // base64-encoded
    pub context: Option<String>,
    pub key_version: Option<u64>,
    pub nonce: Option<String>,
}

pub async fn encrypt(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<EncryptRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;

    let version = body.key_version.unwrap_or(key.latest_version);
    let kv = key.versions.get(&version)
        .ok_or_else(|| VaultError::Crypto("key version not found".into()))?;

    let plaintext = base64::engine::general_purpose::STANDARD.decode(&body.plaintext)
        .map_err(|_| VaultError::InvalidRequest("invalid base64 plaintext".into()))?;

    let ciphertext_bytes = match &key.key_type {
        KeyType::Aes256Gcm96 => aes256_gcm_encrypt(&kv.key_bytes, &plaintext)?,
        KeyType::Chacha20Poly1305 => chacha20_encrypt(&kv.key_bytes, &plaintext)?,
        _ => return Err(VaultError::InvalidRequest("encryption not supported for this key type".into())),
    };

    let ciphertext = format!("vault:v{}:{}", version,
        base64::engine::general_purpose::STANDARD.encode(&ciphertext_bytes));

    Ok(VaultResponse::new().with_data(json!({ "ciphertext": ciphertext })))
}

#[derive(Deserialize)]
pub struct DecryptRequest {
    pub ciphertext: String,
    pub context: Option<String>,
    pub nonce: Option<String>,
}

pub async fn decrypt(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<DecryptRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name))?;

    // Parse ciphertext: vault:v{N}:{base64}
    let parts: Vec<&str> = body.ciphertext.splitn(3, ':').collect();
    if parts.len() != 3 || parts[0] != "vault" {
        return Err(VaultError::InvalidRequest("invalid ciphertext format".into()));
    }
    let version: u64 = parts[1].trim_start_matches('v').parse()
        .map_err(|_| VaultError::InvalidRequest("invalid ciphertext version".into()))?;
    let ciphertext_bytes = base64::engine::general_purpose::STANDARD.decode(parts[2])
        .map_err(|_| VaultError::InvalidRequest("invalid base64 in ciphertext".into()))?;

    if version < key.min_decryption_version {
        return Err(VaultError::Crypto("key version too old for decryption".into()));
    }

    let kv = key.versions.get(&version)
        .ok_or_else(|| VaultError::Crypto("key version not found".into()))?;

    let plaintext_bytes = match &key.key_type {
        KeyType::Aes256Gcm96 => aes256_gcm_decrypt(&kv.key_bytes, &ciphertext_bytes)?,
        KeyType::Chacha20Poly1305 => chacha20_decrypt(&kv.key_bytes, &ciphertext_bytes)?,
        _ => return Err(VaultError::InvalidRequest("decryption not supported for this key type".into())),
    };

    let plaintext = base64::engine::general_purpose::STANDARD.encode(&plaintext_bytes);
    Ok(VaultResponse::new().with_data(json!({ "plaintext": plaintext })))
}

pub async fn rewrap(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<DecryptRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    // Decrypt with old version, re-encrypt with latest
    let decrypt_req = DecryptRequest {
        ciphertext: body.ciphertext.clone(),
        context: body.context.clone(),
        nonce: body.nonce.clone(),
    };

    // We need to do both ops inline
    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name.clone()))?;

    let parts: Vec<&str> = body.ciphertext.splitn(3, ':').collect();
    if parts.len() != 3 || parts[0] != "vault" {
        return Err(VaultError::InvalidRequest("invalid ciphertext format".into()));
    }
    let old_version: u64 = parts[1].trim_start_matches('v').parse()
        .map_err(|_| VaultError::InvalidRequest("invalid version".into()))?;
    let ciphertext_bytes = base64::engine::general_purpose::STANDARD.decode(parts[2])
        .map_err(|_| VaultError::InvalidRequest("invalid base64".into()))?;

    let old_kv = key.versions.get(&old_version)
        .ok_or_else(|| VaultError::Crypto("old key version not found".into()))?;

    let plaintext_bytes = match &key.key_type {
        KeyType::Aes256Gcm96 => aes256_gcm_decrypt(&old_kv.key_bytes, &ciphertext_bytes)?,
        KeyType::Chacha20Poly1305 => chacha20_decrypt(&old_kv.key_bytes, &ciphertext_bytes)?,
        _ => return Err(VaultError::InvalidRequest("rewrap not supported for this key type".into())),
    };

    let new_version = key.latest_version;
    let new_kv = key.versions.get(&new_version)
        .ok_or_else(|| VaultError::Crypto("latest key version not found".into()))?;

    let new_ct_bytes = match &key.key_type {
        KeyType::Aes256Gcm96 => aes256_gcm_encrypt(&new_kv.key_bytes, &plaintext_bytes)?,
        KeyType::Chacha20Poly1305 => chacha20_encrypt(&new_kv.key_bytes, &plaintext_bytes)?,
        _ => return Err(VaultError::InvalidRequest("rewrap not supported".into())),
    };

    let new_ciphertext = format!("vault:v{}:{}", new_version,
        base64::engine::general_purpose::STANDARD.encode(&new_ct_bytes));

    Ok(VaultResponse::new().with_data(json!({ "ciphertext": new_ciphertext })))
}

#[derive(Deserialize)]
pub struct SignRequest {
    pub input: String, // base64-encoded
    pub context: Option<String>,
    pub prehashed: Option<bool>,
    pub signature_algorithm: Option<String>,
    pub marshaling_algorithm: Option<String>,
}

pub async fn sign(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<SignRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name.clone()))?;

    let input = base64::engine::general_purpose::STANDARD.decode(&body.input)
        .map_err(|_| VaultError::InvalidRequest("invalid base64 input".into()))?;

    let version = key.latest_version;
    let kv = key.versions.get(&version)
        .ok_or_else(|| VaultError::Crypto("key version not found".into()))?;

    let signature_b64 = match &key.key_type {
        KeyType::Ed25519 => {
            let pair = Ed25519KeyPair::from_pkcs8(&kv.key_bytes)
                .map_err(|_| VaultError::Crypto("key decode failed".into()))?;
            let sig = pair.sign(&input);
            base64::engine::general_purpose::STANDARD.encode(sig.as_ref())
        }
        _ => return Err(VaultError::InvalidRequest("signing not supported for this key type".into())),
    };

    let signature = format!("vault:v{}:{}", version, signature_b64);
    Ok(VaultResponse::new().with_data(json!({ "signature": signature })))
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub input: String,
    pub signature: String,
    pub context: Option<String>,
    pub prehashed: Option<bool>,
    pub signature_algorithm: Option<String>,
}

pub async fn verify(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_name)): Path<(String, String)>,
    Json(body): Json<VerifyRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name.clone()))?;

    let parts: Vec<&str> = body.signature.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Err(VaultError::InvalidRequest("invalid signature format".into()));
    }
    let version: u64 = parts[1].trim_start_matches('v').parse()
        .map_err(|_| VaultError::InvalidRequest("invalid version".into()))?;
    let sig_bytes = base64::engine::general_purpose::STANDARD.decode(parts[2])
        .map_err(|_| VaultError::InvalidRequest("invalid base64 signature".into()))?;

    let kv = key.versions.get(&version)
        .ok_or_else(|| VaultError::Crypto("key version not found".into()))?;

    let input = base64::engine::general_purpose::STANDARD.decode(&body.input)
        .map_err(|_| VaultError::InvalidRequest("invalid base64 input".into()))?;

    let valid = match &key.key_type {
        KeyType::Ed25519 => {
            let pair = Ed25519KeyPair::from_pkcs8(&kv.key_bytes)
                .map_err(|_| VaultError::Crypto("key decode failed".into()))?;
            let public_key = UnparsedPublicKey::new(&ED25519, pair.public_key().as_ref());
            public_key.verify(&input, &sig_bytes).is_ok()
        }
        _ => return Err(VaultError::InvalidRequest("verify not supported for this key type".into())),
    };

    Ok(VaultResponse::new().with_data(json!({ "valid": valid })))
}

#[derive(Deserialize)]
pub struct GenerateDataKeyRequest {
    pub bits: Option<u32>,
    pub context: Option<String>,
}

pub async fn generate_data_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, plaintext_or_wrapped, key_name)): Path<(String, String, String)>,
    Json(body): Json<GenerateDataKeyRequest>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let rng = SystemRandom::new();
    let bits = body.bits.unwrap_or(256);
    let key_len = (bits / 8) as usize;
    let mut data_key = vec![0u8; key_len];
    rng.fill(&mut data_key).map_err(|_| VaultError::Crypto("rng failure".into()))?;

    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name.clone()))?;
    let version = key.latest_version;
    let kv = key.versions.get(&version)
        .ok_or_else(|| VaultError::Crypto("key version not found".into()))?;

    let wrapped_bytes = aes256_gcm_encrypt(&kv.key_bytes, &data_key)?;
    let ciphertext_key = format!("vault:v{}:{}", version,
        base64::engine::general_purpose::STANDARD.encode(&wrapped_bytes));

    let mut response = json!({
        "ciphertext": ciphertext_key,
        "key_version": version,
    });

    if plaintext_or_wrapped == "plaintext" {
        response["plaintext"] = json!(base64::engine::general_purpose::STANDARD.encode(&data_key));
    }

    Ok(VaultResponse::new().with_data(response))
}

pub async fn export_key(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, key_type_path, key_name, version_str)): Path<(String, String, String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.transit_store.read().await;
    let key = store.keys.get(&key_name)
        .ok_or_else(|| VaultError::KeyNotFound(key_name.clone()))?;

    if !key.exportable {
        return Err(VaultError::InvalidRequest("key is not exportable".into()));
    }

    let versions_to_export: Vec<u64> = if version_str == "latest" {
        vec![key.latest_version]
    } else if let Ok(v) = version_str.parse::<u64>() {
        vec![v]
    } else {
        key.versions.keys().cloned().collect()
    };

    let mut keys_map = serde_json::Map::new();
    for v in versions_to_export {
        if let Some(kv) = key.versions.get(&v) {
            keys_map.insert(v.to_string(), json!(
                base64::engine::general_purpose::STANDARD.encode(&kv.key_bytes)
            ));
        }
    }

    Ok(VaultResponse::new().with_data(json!({
        "name": key.name,
        "type": format!("{:?}", key.key_type).to_lowercase(),
        "keys": keys_map,
    })))
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    Router::new()
        .route(&format!("/v1/{}/keys", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { list_keys(State(state), headers, Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/keys/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { create_key(State(state), headers, Path((mount, key_name)), Json(body)).await }
            }
        }).get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { read_key(State(state), headers, Path((mount, key_name))).await }
            }
        }).delete({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { delete_key(State(state), headers, Path((mount, key_name))).await }
            }
        }))
        .route(&format!("/v1/{}/keys/:key_name/rotate", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { rotate_key(State(state), headers, Path((mount, key_name))).await }
            }
        }))
        .route(&format!("/v1/{}/encrypt/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<EncryptRequest>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { encrypt(State(state), headers, Path((mount, key_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/decrypt/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<DecryptRequest>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { decrypt(State(state), headers, Path((mount, key_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/rewrap/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<DecryptRequest>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { rewrap(State(state), headers, Path((mount, key_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/sign/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<SignRequest>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { sign(State(state), headers, Path((mount, key_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/verify/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(key_name): Path<String>, Json(body): Json<VerifyRequest>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { verify(State(state), headers, Path((mount, key_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/datakey/:plaintext_or_wrapped/:key_name", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path((pow, key_name)): Path<(String, String)>, Json(body): Json<GenerateDataKeyRequest>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { generate_data_key(State(state), headers, Path((mount, pow, key_name)), Json(body)).await }
            }
        }))
        .route(&format!("/v1/{}/export/:key_type/:key_name/:version", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path((kt, key_name, version)): Path<(String, String, String)>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { export_key(State(state), headers, Path((mount, kt, key_name, version))).await }
            }
        }))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn test_aes256_gcm_round_trip() {
        let key = generate_aes256_key().unwrap();
        let plaintext = b"Hello, Vault transit engine!";
        let ciphertext = aes256_gcm_encrypt(&key, plaintext).unwrap();
        let decrypted = aes256_gcm_decrypt(&key, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_chacha20_round_trip() {
        let key = generate_aes256_key().unwrap();
        let plaintext = b"chacha20 test data";
        let ciphertext = chacha20_encrypt(&key, plaintext).unwrap();
        let decrypted = chacha20_decrypt(&key, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_transit_key_creation() {
        let key = create_transit_key("mykey", KeyType::Aes256Gcm96).unwrap();
        assert_eq!(key.name, "mykey");
        assert_eq!(key.latest_version, 1);
        assert!(key.versions.contains_key(&1));
    }

    #[test]
    fn test_ed25519_key_sign_verify() {
        use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
        let (pkcs8_bytes, _pub_b64) = generate_ed25519_key().unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(&pkcs8_bytes).unwrap();
        let message = b"test message for signing";
        let sig = pair.sign(message);
        let public_key = UnparsedPublicKey::new(&ED25519, pair.public_key().as_ref());
        assert!(public_key.verify(message, sig.as_ref()).is_ok());
    }
}
