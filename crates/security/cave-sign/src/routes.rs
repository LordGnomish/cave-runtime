// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-sign.
//!
//! Surface maps to the cosign CLI commands:
//!   /api/sign/sign           → cosign sign
//!   /api/sign/verify         → cosign verify
//!   /api/sign/attest         → cosign attest
//!   /api/sign/verify-attest  → cosign verify-attest
//!   /api/sign/policy         → policy CRUD
//!   /api/sign/fulcio         → fulcio status
//!   /api/sign/rekor          → rekor lookup
//!   /api/sign/health         → liveness

use crate::State;
use crate::blob::sign_blob_keypair_with_rekor;
use crate::error::SignError;
use crate::models::{ArtifactType, KeyAlgorithm, SignedArtifact};
use crate::signature::Keypair;
use axum::{
    Json, Router,
    extract::{Query, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/sign/health", get(health))
        .route("/api/sign/sign", post(sign_route))
        .route("/api/sign/verify", post(verify_route))
        .route("/api/sign/attest", post(attest_route))
        .route("/api/sign/verify-attest", post(verify_attest_route))
        .route("/api/sign/policy", get(get_policy_route).post(set_policy_route))
        .route("/api/sign/fulcio", get(fulcio_route))
        .route("/api/sign/rekor", get(rekor_route))
        .route("/api/sign/list", get(list_route))
        .route("/api/sign/sigstore-bundle", post(sigstore_bundle_route))
        .route("/api/sign/triangulate", get(triangulate_route))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-sign",
        "status": "ok",
        "upstream": "sigstore/cosign v3.0.6",
    }))
}

#[derive(Debug, Deserialize)]
pub struct SignReq {
    pub artifact: String,
    #[serde(default)]
    pub payload_b64: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SignResp {
    pub artifact_digest: String,
    pub signature_b64: String,
    pub bundle_json: String,
    pub log_index: Option<u64>,
}

async fn sign_route(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<SignReq>,
) -> std::result::Result<Json<SignResp>, ApiError> {
    use base64::Engine;
    let payload = match req.payload_b64.as_deref() {
        Some(b64) => base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| ApiError::bad(format!("payload base64: {}", e)))?,
        None => req.artifact.as_bytes().to_vec(),
    };
    let kp = Keypair::generate(KeyAlgorithm::EcdsaP256).map_err(ApiError::from)?;
    let b = sign_blob_keypair_with_rekor(&payload, &kp, &state.rekor).map_err(ApiError::from)?;
    state
        .store
        .insert(
            b.artifact_digest.clone(),
            ArtifactType::Blob,
            b.signature.sig_b64.clone(),
            req.key_id.unwrap_or_else(|| "ephemeral".into()),
            true,
        )
        .map_err(ApiError::from)?;
    Ok(Json(SignResp {
        artifact_digest: b.artifact_digest,
        signature_b64: b.signature.sig_b64,
        bundle_json: b.bundle.encode_json().map_err(ApiError::from)?,
        log_index: b.signature.log_index,
    }))
}

#[derive(Debug, Deserialize)]
pub struct VerifyReq {
    pub artifact: String,
    pub bundle_json: String,
    #[serde(default)]
    pub payload_b64: Option<String>,
    #[serde(default)]
    pub identity: Option<String>,
}

async fn verify_route(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<VerifyReq>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    use base64::Engine;
    let payload = match req.payload_b64.as_deref() {
        Some(b64) => base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| ApiError::bad(format!("payload base64: {}", e)))?,
        None => req.artifact.as_bytes().to_vec(),
    };
    let bundle = crate::bundle::CosignBundle::decode_json(&req.bundle_json).map_err(ApiError::from)?;
    let policy = req.identity.as_deref().map(|i| {
        crate::policy::Policy::new("inline")
            .require(crate::policy::Rule::CertificateIdentity { glob: i.into() })
    });
    let req = crate::verify::VerifyRequest {
        payload: &payload,
        bundle: &bundle,
        rekor: Some(&state.rekor),
        policy: policy.as_ref(),
    };
    let out = crate::verify::verify(req).map_err(ApiError::from)?;
    Ok(Json(serde_json::to_value(out).unwrap()))
}

#[derive(Debug, Deserialize)]
pub struct AttestReq {
    pub subject_name: String,
    pub subject_digest: String,
    pub predicate_type: String,
    pub predicate: serde_json::Value,
}

async fn attest_route(
    Json(req): Json<AttestReq>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    use crate::attestation::{sign_attestation, subject_sha256};
    use crate::models::{Attestation, PredicateType};
    let att = Attestation {
        media_type: "application/vnd.in-toto+json".into(),
        predicate_type: PredicateType::from_uri(&req.predicate_type),
        subject: vec![subject_sha256(&req.subject_name, &req.subject_digest)],
        predicate: req.predicate,
    };
    let kp = Keypair::generate(KeyAlgorithm::EcdsaP256).map_err(ApiError::from)?;
    let env = sign_attestation(&att, &kp, "cave-sign-ephemeral").map_err(ApiError::from)?;
    Ok(Json(json!({
        "envelope": env,
        "public_key_pem": crate::keypair::encode_public_pem(kp.algorithm, kp.public_key_bytes()),
    })))
}

#[derive(Debug, Deserialize)]
pub struct VerifyAttestReq {
    pub envelope: crate::attestation::DsseEnvelope,
    pub public_key_pem: String,
}

async fn verify_attest_route(
    Json(req): Json<VerifyAttestReq>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let (alg, pk) = crate::keypair::decode_public_pem(&req.public_key_pem).map_err(ApiError::from)?;
    let att = crate::attestation::verify_envelope(&req.envelope, alg, &pk).map_err(ApiError::from)?;
    Ok(Json(json!({
        "valid": true,
        "predicate_type": att.predicate_type.uri(),
        "subjects": att.subject,
    })))
}

async fn get_policy_route(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let p = state.policy.lock().ok().map(|p| p.clone()).unwrap_or_default();
    Json(serde_json::to_value(p).unwrap())
}

async fn set_policy_route(
    AxumState(state): AxumState<Arc<State>>,
    Json(p): Json<crate::policy::Policy>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    if let Ok(mut g) = state.policy.lock() {
        *g = p.clone();
    }
    Ok(Json(serde_json::to_value(p).unwrap()))
}

async fn fulcio_route(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    Json(json!({
        "fulcio_url": state.fulcio.base_url,
        "kind": "mock",
    }))
}

#[derive(Debug, Deserialize)]
pub struct RekorQuery {
    #[serde(default)]
    pub log_index: Option<u64>,
    #[serde(default)]
    pub digest: Option<String>,
}

async fn rekor_route(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<RekorQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    if let Some(i) = q.log_index {
        let e = state.rekor.get_by_index_offline(i).map_err(ApiError::from)?;
        return Ok(Json(serde_json::to_value(e).unwrap()));
    }
    if let Some(d) = q.digest {
        let es = state.rekor.search_by_digest_offline(&d).map_err(ApiError::from)?;
        return Ok(Json(serde_json::to_value(es).unwrap()));
    }
    let (size, root) = state.rekor.tree_state_offline().map_err(ApiError::from)?;
    Ok(Json(json!({"tree_size": size, "root_hash": root})))
}

async fn list_route(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<SignedArtifact>> {
    Json(state.store.all().unwrap_or_default())
}

#[derive(Debug, Deserialize)]
pub struct TriangulateQuery {
    /// `registry/repo@sha256:<hex>` image reference.
    pub image: String,
    /// `signature` (default) | `attestation` | `sbom`.
    #[serde(default)]
    pub r#type: Option<String>,
}

/// `cosign triangulate` — derive the OCI reference of an attached artifact.
async fn triangulate_route(
    Query(q): Query<TriangulateQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let image = crate::oci::ImageRef::parse(&q.image).map_err(ApiError::from)?;
    let kind = crate::oci::CosignArtifactType::parse(q.r#type.as_deref().unwrap_or("signature"))
        .map_err(ApiError::from)?;
    Ok(Json(json!({
        "image": q.image,
        "type": kind,
        "reference": image.triangulate(kind),
    })))
}

#[derive(Debug, Deserialize)]
pub struct SigstoreBundleReq {
    /// Flat cosign bundle JSON to convert to the v0.3 protobuf-JSON envelope.
    pub bundle_json: String,
    /// Optional DSSE envelope — when present the result is an attestation
    /// bundle (dsseEnvelope) instead of a messageSignature bundle.
    #[serde(default)]
    pub dsse_envelope: Option<serde_json::Value>,
}

/// Convert a flat cosign bundle into the modern Sigstore protobuf bundle v0.3
/// envelope (`cosign sign --new-bundle-format`).
async fn sigstore_bundle_route(
    Json(req): Json<SigstoreBundleReq>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let cosign = crate::bundle::CosignBundle::decode_json(&req.bundle_json).map_err(ApiError::from)?;
    let v03 = match req.dsse_envelope {
        Some(env) => crate::sigstore_bundle::SigstoreBundle::from_dsse(&cosign, env),
        None => crate::sigstore_bundle::SigstoreBundle::from_cosign_bundle(&cosign),
    }
    .map_err(ApiError::from)?;
    Ok(Json(serde_json::to_value(v03).unwrap()))
}

// ─── error helper ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ApiError {
    code: StatusCode,
    msg: String,
}

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self {
            code: StatusCode::BAD_REQUEST,
            msg: msg.into(),
        }
    }
}

impl From<SignError> for ApiError {
    fn from(e: SignError) -> Self {
        let code = match &e {
            SignError::NotFound(_) => StatusCode::NOT_FOUND,
            SignError::Policy(_) | SignError::Verify(_) | SignError::Tlog(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            code,
            msg: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.code, Json(json!({"error": self.msg}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    use axum::http::{Request, Method};
    use tower::ServiceExt;

    fn router() -> Router {
        create_router(Arc::new(State::default()))
    }

    #[tokio::test]
    async fn health_ok() {
        let r = router();
        let resp = r
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sign/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sign_then_list() {
        let state = Arc::new(State::default());
        let r = create_router(state.clone());
        let body = serde_json::to_vec(&json!({"artifact":"hello"})).unwrap();
        let resp = r
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sign/sign")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = r
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sign/list")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rekor_query_returns_tree_state() {
        let r = router();
        let resp = r
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sign/rekor")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn fulcio_returns_config() {
        let r = router();
        let resp = r
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sign/fulcio")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sigstore_bundle_converts_cosign_bundle() {
        let r = router();
        let cosign = json!({
            "kind": "keypair",
            "signed_payload_b64": "c2ln",
            "cert_pem": "-----BEGIN PUBLIC KEY-----\nQQ==\n-----END PUBLIC KEY-----",
            "artifact_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        });
        let body = serde_json::to_vec(&json!({
            "bundle_json": serde_json::to_string(&cosign).unwrap()
        }))
        .unwrap();
        let resp = r
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sign/sigstore-bundle")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            v["mediaType"],
            "application/vnd.dev.sigstore.bundle.v0.3+json"
        );
        assert!(v["verificationMaterial"]["publicKey"].is_object());
    }

    #[tokio::test]
    async fn policy_set_then_get_roundtrip() {
        let state = Arc::new(State::default());
        let r = create_router(state.clone());
        let policy = crate::policy::Policy::new("test")
            .require(crate::policy::Rule::CertificateIdentity { glob: "*@cave.io".into() });
        let body = serde_json::to_vec(&policy).unwrap();
        let resp = r
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sign/policy")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = r
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sign/policy")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
