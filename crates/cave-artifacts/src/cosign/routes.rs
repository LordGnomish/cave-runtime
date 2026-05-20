// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: sigstore/cosign@HEAD cmd/cosign/cli/sign/ + cmd/cosign/cli/verify/
//! HTTP surface for the Cosign module.
//!
//! Endpoint shapes mirror the cosign-style "sigstore policy" REST surface
//! enough that consumers familiar with `cosign sign` and `cosign verify`
//! can reach for the same verbs without learning a new vocabulary.

use super::manifest::{
    SignatureIndex, SignatureRecord, build_payload, manifest_digest, signature_tag,
};
use super::{Alg, CosignError, KeyPair, PublicKeyHandle, Signature, sign, verify};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Combined state for the Cosign subsystem.
pub struct CosignState {
    pub keys: RwLock<HashMap<String, Arc<KeyPair>>>,
    pub publics: RwLock<HashMap<String, PublicKeyHandle>>,
    pub index: SignatureIndex,
    /// Counters for the dashboard / observability panels.
    pub counters: RwLock<Counters>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Counters {
    pub keys_generated_classic: u64,
    pub keys_generated_pqc: u64,
    pub signatures_issued_classic: u64,
    pub signatures_issued_pqc: u64,
    pub verifications_passed_classic: u64,
    pub verifications_passed_pqc: u64,
    pub verifications_failed: u64,
}

impl CosignState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            keys: RwLock::new(HashMap::new()),
            publics: RwLock::new(HashMap::new()),
            index: SignatureIndex::new(),
            counters: RwLock::new(Counters::default()),
        })
    }
}

impl Default for CosignState {
    fn default() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
            publics: RwLock::new(HashMap::new()),
            index: SignatureIndex::new(),
            counters: RwLock::new(Counters::default()),
        }
    }
}

pub fn router(state: Arc<CosignState>) -> Router {
    Router::new()
        .route("/api/cosign/v1/health", get(health))
        .route("/api/cosign/v1/keypair", post(generate_keypair))
        .route("/api/cosign/v1/keypair/{id}", get(get_public_key))
        .route("/api/cosign/v1/sign", post(sign_payload))
        .route("/api/cosign/v1/verify", post(verify_payload))
        .route(
            "/api/cosign/v1/signatures/{digest}",
            get(list_signatures).delete(delete_signatures),
        )
        .route("/api/cosign/v1/counters", get(get_counters))
        .with_state(state)
}

// ── helpers ─────────────────────────────────────────────────────────────

fn map_err(e: CosignError) -> Response {
    let (code, kind) = match &e {
        CosignError::UnknownAlgorithm(_) => (StatusCode::BAD_REQUEST, "unknown_algorithm"),
        CosignError::KeyNotFound(_) | CosignError::SignatureNotFound(_) => {
            (StatusCode::NOT_FOUND, "not_found")
        }
        CosignError::EcdsaVerifyFailed | CosignError::PqcCompositeFailed(_) => {
            (StatusCode::UNAUTHORIZED, "verify_failed")
        }
        CosignError::AlgorithmMismatch { .. } => (StatusCode::BAD_REQUEST, "algorithm_mismatch"),
        CosignError::MalformedKey(_) | CosignError::PayloadInvalid(_) => {
            (StatusCode::BAD_REQUEST, "bad_request")
        }
        CosignError::DigestMismatch { .. } => (StatusCode::CONFLICT, "digest_mismatch"),
    };
    (
        code,
        Json(json!({ "error": kind, "message": e.to_string() })),
    )
        .into_response()
}

// ── handlers ────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<CosignState>>) -> Json<serde_json::Value> {
    let c = state.counters.read().unwrap().clone();
    Json(json!({
        "status": "ok",
        "module": "cosign",
        "supported_algorithms": ["ecdsa-p256", "ml-dsa-65"],
        "pqc_backend": "fixture",
        "pqc_backend_note": "ML-DSA-65 half is a deterministic fixture (cave_certs::pqc::pqc_fixture); real signer wired when pqcrypto-mldsa or oqs-rs lands in workspace deps",
        "counters": c,
    }))
}

#[derive(Debug, Deserialize)]
struct GenerateKeypairRequest {
    pub alg: String,
}

async fn generate_keypair(
    State(state): State<Arc<CosignState>>,
    Json(req): Json<GenerateKeypairRequest>,
) -> Response {
    let alg = match Alg::parse(&req.alg) {
        Some(a) => a,
        None => return map_err(CosignError::UnknownAlgorithm(req.alg)),
    };
    let key = KeyPair::generate(alg);
    let pub_handle = match key.public_handle() {
        Ok(h) => h,
        Err(e) => return map_err(e),
    };
    let id = key.id().to_string();
    state
        .keys
        .write()
        .unwrap()
        .insert(id.clone(), Arc::new(key));
    state
        .publics
        .write()
        .unwrap()
        .insert(id.clone(), pub_handle.clone());
    {
        let mut c = state.counters.write().unwrap();
        match alg {
            Alg::EcdsaP256 => c.keys_generated_classic += 1,
            Alg::MlDsa65 => c.keys_generated_pqc += 1,
        }
    }
    (StatusCode::CREATED, Json(pub_handle)).into_response()
}

async fn get_public_key(State(state): State<Arc<CosignState>>, Path(id): Path<String>) -> Response {
    match state.publics.read().unwrap().get(&id) {
        Some(h) => Json(h.clone()).into_response(),
        None => map_err(CosignError::KeyNotFound(id)),
    }
}

#[derive(Debug, Deserialize)]
struct SignRequest {
    pub key_id: String,
    /// Image reference to pin in the Cosign payload (e.g.
    /// `registry/foo:tag`).
    pub reference: String,
    /// Manifest digest (`sha256:HEX`) the signature attests.
    pub digest: String,
}

#[derive(Debug, Serialize)]
struct SignResponse {
    pub signature: Signature,
    pub payload_b64: String,
    pub signature_tag: String,
}

async fn sign_payload(
    State(state): State<Arc<CosignState>>,
    Json(req): Json<SignRequest>,
) -> Response {
    let key = match state.keys.read().unwrap().get(&req.key_id).cloned() {
        Some(k) => k,
        None => return map_err(CosignError::KeyNotFound(req.key_id)),
    };
    let payload = build_payload(&req.reference, &req.digest);
    let sig = match sign(&key, &payload) {
        Ok(s) => s,
        Err(e) => return map_err(e),
    };
    let tag = match signature_tag(&req.digest) {
        Ok(t) => t,
        Err(e) => return map_err(e),
    };

    state.index.attach(&req.digest, &payload, sig.clone());
    {
        let mut c = state.counters.write().unwrap();
        match sig.alg() {
            Alg::EcdsaP256 => c.signatures_issued_classic += 1,
            Alg::MlDsa65 => c.signatures_issued_pqc += 1,
        }
    }

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    (
        StatusCode::CREATED,
        Json(SignResponse {
            signature: sig,
            payload_b64: STANDARD.encode(&payload),
            signature_tag: tag,
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    pub key_id: String,
    pub signature: Signature,
    pub payload_b64: String,
}

async fn verify_payload(
    State(state): State<Arc<CosignState>>,
    Json(req): Json<VerifyRequest>,
) -> Response {
    let pubkey = match state.publics.read().unwrap().get(&req.key_id).cloned() {
        Some(p) => p,
        None => return map_err(CosignError::KeyNotFound(req.key_id)),
    };
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    let payload = match STANDARD.decode(req.payload_b64) {
        Ok(p) => p,
        Err(e) => return map_err(CosignError::MalformedKey(format!("payload_b64: {e}"))),
    };
    match verify(&pubkey, &payload, &req.signature) {
        Ok(()) => {
            let mut c = state.counters.write().unwrap();
            match req.signature.alg() {
                Alg::EcdsaP256 => c.verifications_passed_classic += 1,
                Alg::MlDsa65 => c.verifications_passed_pqc += 1,
            }
            Json(json!({ "valid": true })).into_response()
        }
        Err(e) => {
            state.counters.write().unwrap().verifications_failed += 1;
            map_err(e)
        }
    }
}

async fn list_signatures(
    State(state): State<Arc<CosignState>>,
    Path(digest): Path<String>,
) -> Json<Vec<SignatureRecord>> {
    Json(state.index.list(&digest))
}

async fn delete_signatures(
    State(state): State<Arc<CosignState>>,
    Path(digest): Path<String>,
) -> Response {
    let removed = state.index.remove(&digest);
    Json(json!({ "removed": removed })).into_response()
}

async fn get_counters(State(state): State<Arc<CosignState>>) -> Json<Counters> {
    Json(state.counters.read().unwrap().clone())
}

// keep manifest_digest reachable from this module
#[allow(dead_code)]
fn _digest_sentinel(b: &[u8]) -> String {
    manifest_digest(b)
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SearchQuery {
    pub digest: Option<String>,
}
#[allow(dead_code)]
fn _query_sentinel(_: SearchQuery, _: Query<SearchQuery>) {}
