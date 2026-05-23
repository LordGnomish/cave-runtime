// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! HTTP API surface — endpoints under `/api/cert/*`.
//!
//! cert-manager exposes its workflow through CRDs + the Kubernetes API
//! server; cave-cert-manager mirrors the same operations as JSON-shaped
//! REST endpoints so cavectl + cave-portal-api can drive the control
//! plane directly.

use crate::controller::{CertControlPlane, ReconcileResult};
use crate::error::CertManagerError;
use crate::metrics::CertManagerMetrics;
use crate::models::{Certificate, CertificateRequest, ClusterIssuer, IssuerResource};
use crate::revocation::{RevocationLedger, RevocationReason, RevocationRecord};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Application state for the cert-manager HTTP surface.
///
/// Wraps the in-memory control plane, a revocation ledger, and a
/// metrics registry behind a single Mutex so axum's `State` extractor
/// can hand a clone to every handler. Production wiring threads the
/// same `Arc<Mutex<RuntimeState>>` through the reconcile loop +
/// background renewal scheduler so concurrent operators see a
/// coherent snapshot.
pub struct RuntimeState {
    pub plane: CertControlPlane,
    pub revocations: RevocationLedger,
    pub metrics: CertManagerMetrics,
}

impl RuntimeState {
    pub fn new() -> Self {
        Self {
            plane: CertControlPlane::new(),
            revocations: RevocationLedger::new(),
            metrics: CertManagerMetrics::new(),
        }
    }
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

pub type CertState = Arc<Mutex<RuntimeState>>;

pub fn create_router(state: CertState) -> Router {
    Router::new()
        .route("/api/cert/health", get(health))
        .route("/metrics", get(metrics_exposition))
        .route(
            "/api/cert/{tenant}/certificates",
            get(list_certificates).post(create_certificate),
        )
        .route(
            "/api/cert/{tenant}/certificates/{id}",
            get(get_certificate),
        )
        .route(
            "/api/cert/{tenant}/certificates/{id}/issue",
            post(issue_certificate),
        )
        .route(
            "/api/cert/{tenant}/certificates/{id}/renew",
            post(renew_certificate),
        )
        .route(
            "/api/cert/{tenant}/certificates/{id}/verify",
            get(verify_certificate),
        )
        .route(
            "/api/cert/{tenant}/certificates/{id}/revoke",
            post(revoke_certificate),
        )
        .route(
            "/api/cert/{tenant}/certificate-requests",
            get(list_requests),
        )
        .route(
            "/api/cert/{tenant}/issuers",
            get(list_issuers).post(create_issuer),
        )
        .route(
            "/api/cert/{tenant}/cluster-issuers",
            get(list_cluster_issuers).post(create_cluster_issuer),
        )
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cert-manager",
        "status": "ok",
        "upstream": "cert-manager/cert-manager",
        "upstream_version": crate::UPSTREAM_VERSION,
        "upstream_source_sha": crate::UPSTREAM_SOURCE_SHA,
    }))
}

async fn list_certificates(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let rs = state.lock().unwrap();
    let cp = &rs.plane;
    let certs: Vec<Certificate> = cp
        .store
        .list_certificates(&tenant)
        .iter()
        .map(|c| (*c).clone())
        .collect();
    (StatusCode::OK, Json(certs))
}

async fn create_certificate(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
    Json(mut cert): Json<Certificate>,
) -> impl IntoResponse {
    cert.tenant_id = tenant;
    cert.id = Uuid::new_v4();
    cert.created_at = Utc::now();
    cert.updated_at = Utc::now();
    cert.status = None;
    if let Err(e) = cert.spec.validate() {
        return (StatusCode::BAD_REQUEST, Json(error_body(&e))).into_response();
    }
    let mut rs = state.lock().unwrap();
    let cp = &mut rs.plane;
    let id = cp.store.put_certificate(cert);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id}))).into_response()
}

async fn get_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let rs = state.lock().unwrap();
    let cp = &rs.plane;
    match cp.store.certificate(&tenant, id) {
        Ok(c) => (StatusCode::OK, Json(c.clone())).into_response(),
        Err(e) => (status_for(&e), Json(error_body(&e))).into_response(),
    }
}

async fn issue_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let mut rs = state.lock().unwrap();
    let cp = &mut rs.plane;
    match cp.controller().reconcile(&tenant, id) {
        Ok(r) => (StatusCode::OK, Json(serialise_result(&r))).into_response(),
        Err(e) => (status_for(&e), Json(error_body(&e))).into_response(),
    }
}

async fn renew_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    // Renewal is just another reconcile — Cert.spec is unchanged.
    issue_certificate(Path((tenant, id)), State(state)).await
}

/// Verify a Certificate against:
///   * the RevocationLedger (any revocation record blocks the read)
///   * the materialised Secret's notAfter (must be in the future)
///   * the Ready condition (must be `True`)
///
/// Returns a structured report. Idempotent + read-only — safe to poll
/// from `cavectl cert verify` or a cron probe.
async fn verify_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let rs = state.lock().unwrap();
    let cp = &rs.plane;
    let cert = match cp.store.certificate(&tenant, id) {
        Ok(c) => c,
        Err(e) => return (status_for(&e), Json(error_body(&e))).into_response(),
    };
    let revision = cert.status.as_ref().map(|s| s.revision).unwrap_or(0);
    let revoked = rs
        .revocations
        .get(&tenant, id, revision)
        .ok()
        .flatten()
        .cloned();
    let not_after = cert.status.as_ref().and_then(|s| s.not_after);
    let ready = cert
        .status
        .as_ref()
        .and_then(|s| {
            s.conditions
                .iter()
                .find(|c| {
                    c.kind == crate::models::CertificateConditionType::Ready
                })
                .map(|c| c.status)
        });
    let now = Utc::now();
    let expired = not_after.map(|na| na <= now).unwrap_or(false);
    let valid = !expired
        && revoked.is_none()
        && matches!(ready, Some(crate::models::ConditionStatus::True));
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tenant": tenant,
            "certificate_id": id,
            "revision": revision,
            "ready": ready.map(|s| format!("{:?}", s)),
            "not_after": not_after,
            "expired": expired,
            "revoked": revoked.is_some(),
            "revocation_reason": revoked.as_ref().map(|r| format!("{:?}", r.reason)),
            "valid": valid,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RevokeBody {
    /// One of: Unspecified|KeyCompromise|CaCompromise|AffiliationChanged|
    /// Superseded|CessationOfOperation|CertificateHold|RemoveFromCrl|
    /// PrivilegeWithdrawn|AaCompromise — anything else is rejected with 400.
    reason: RevocationReason,
    revoked_by: String,
    #[serde(default)]
    note: Option<String>,
}

async fn revoke_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
    Json(body): Json<RevokeBody>,
) -> impl IntoResponse {
    let mut rs = state.lock().unwrap();
    let cert = match rs.plane.store.certificate(&tenant, id) {
        Ok(c) => c.clone(),
        Err(e) => return (status_for(&e), Json(error_body(&e))).into_response(),
    };
    let revision = cert.status.as_ref().map(|s| s.revision).unwrap_or(0);
    let serial = cert
        .status
        .as_ref()
        .and_then(|s| s.serial.clone())
        .unwrap_or_else(|| "pending".into());
    let rec = RevocationRecord {
        tenant_id: tenant.clone(),
        certificate_id: id,
        revision,
        serial,
        reason: body.reason,
        revoked_at: Utc::now(),
        revoked_by: body.revoked_by,
        note: body.note,
    };
    let RuntimeState {
        revocations,
        metrics,
        ..
    } = &mut *rs;
    match revocations.revoke_with_metrics(rec, metrics) {
        Ok(r) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "tenant": r.tenant_id,
                "certificate_id": r.certificate_id,
                "revision": r.revision,
                "reason_code": r.reason.reason_code(),
                "reason": format!("{:?}", r.reason),
                "revoked_at": r.revoked_at,
            })),
        )
            .into_response(),
        Err(e) => (status_for(&e), Json(error_body(&e))).into_response(),
    }
}

async fn metrics_exposition(State(state): State<CertState>) -> impl IntoResponse {
    let rs = state.lock().unwrap();
    let body = rs.metrics.render_prometheus();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

async fn list_requests(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let rs = state.lock().unwrap();
    let cp = &rs.plane;
    let mut all: Vec<CertificateRequest> = Vec::new();
    for cert in cp.store.list_certificates(&tenant) {
        for r in cp.store.list_requests_for(&tenant, cert.id) {
            all.push(r.clone());
        }
    }
    (StatusCode::OK, Json(all))
}

async fn list_issuers(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    // No direct list method on store — derive from the inner map via
    // count; tests stay focused on the controller.
    let rs = state.lock().unwrap();
    let cp = &rs.plane;
    (StatusCode::OK, Json(serde_json::json!({"count": cp.store.issuer_count(), "tenant": tenant})))
}

async fn create_issuer(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
    Json(mut issuer): Json<IssuerResource>,
) -> impl IntoResponse {
    issuer.tenant_id = tenant;
    issuer.id = Uuid::new_v4();
    issuer.created_at = Utc::now();
    let mut rs = state.lock().unwrap();
    let cp = &mut rs.plane;
    let id = cp.store.put_issuer(issuer);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id})))
}

async fn list_cluster_issuers(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let rs = state.lock().unwrap();
    let cp = &rs.plane;
    (
        StatusCode::OK,
        Json(serde_json::json!({"count": cp.store.cluster_issuer_count(), "tenant": tenant})),
    )
}

async fn create_cluster_issuer(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
    Json(mut issuer): Json<ClusterIssuer>,
) -> impl IntoResponse {
    issuer.tenant_id = tenant;
    issuer.id = Uuid::new_v4();
    issuer.created_at = Utc::now();
    let mut rs = state.lock().unwrap();
    let cp = &mut rs.plane;
    let id = cp.store.put_cluster_issuer(issuer);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id})))
}

fn serialise_result(r: &ReconcileResult) -> serde_json::Value {
    serde_json::json!({
        "request_id": r.request_id,
        "new_revision": r.new_revision,
        "previous_revision": r.previous_revision,
        "secret_name": r.secret.name,
        "secret_namespace": r.secret.namespace,
        "events": r.events.iter().map(|e| match e {
            crate::controller::ReconcileEvent::Issued { certificate_id, serial } =>
                serde_json::json!({"kind": "Issued", "certificate_id": certificate_id, "serial": serial}),
            crate::controller::ReconcileEvent::Renewed { certificate_id, serial } =>
                serde_json::json!({"kind": "Renewed", "certificate_id": certificate_id, "serial": serial}),
            crate::controller::ReconcileEvent::Failed { certificate_id, message } =>
                serde_json::json!({"kind": "Failed", "certificate_id": certificate_id, "message": message}),
        }).collect::<Vec<_>>(),
    })
}

fn error_body(e: &CertManagerError) -> serde_json::Value {
    serde_json::json!({"error": e.to_string()})
}

fn status_for(e: &CertManagerError) -> StatusCode {
    match e {
        CertManagerError::CertificateNotFound(_)
        | CertManagerError::CertificateRequestNotFound(_)
        | CertManagerError::IssuerNotFound(_)
        | CertManagerError::ClusterIssuerNotFound(_)
        | CertManagerError::SecretNotFound(_) => StatusCode::NOT_FOUND,
        CertManagerError::CrossTenantDenied { .. } => StatusCode::FORBIDDEN,
        CertManagerError::InvalidSpec(_)
        | CertManagerError::EmptyDnsNames
        | CertManagerError::InvalidDnsName { .. }
        | CertManagerError::RenewBeforeExceedsDuration { .. }
        | CertManagerError::VaultKeychainScheme(_) => StatusCode::BAD_REQUEST,
        CertManagerError::NotReady(_) => StatusCode::ACCEPTED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateSpec, IssuerRef, IssuerRefKind, IssuerSpec, PrivateKeyPolicy, Usage,
    };
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::collections::BTreeMap;
    use tower::ServiceExt;

    fn fresh_state() -> CertState {
        Arc::new(Mutex::new(RuntimeState::new()))
    }

    fn cert_body() -> Certificate {
        Certificate {
            id: Uuid::nil(),
            name: "demo".into(),
            namespace: "default".into(),
            tenant_id: "tbd".into(),
            spec: CertificateSpec {
                secret_name: "tls".into(),
                issuer_ref: IssuerRef {
                    name: "selfsigned".into(),
                    kind: IssuerRefKind::ClusterIssuer,
                    group: "cert-manager.io".into(),
                },
                dns_names: vec!["api.example.com".into()],
                ip_addresses: vec![],
                uris: vec![],
                email_addresses: vec![],
                common_name: None,
                duration_seconds: 90 * 24 * 3600,
                renew_before_seconds: 30 * 24 * 3600,
                usages: vec![Usage::ServerAuth],
                private_key: PrivateKeyPolicy::default(),
                is_ca: false,
                subject: None,
                secret_template_labels: BTreeMap::new(),
                secret_template_annotations: BTreeMap::new(),
            },
            status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        }
    }

    fn cluster_issuer_body() -> ClusterIssuer {
        ClusterIssuer {
            id: Uuid::nil(),
            name: "selfsigned".into(),
            tenant_id: "tbd".into(),
            spec: IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn health_endpoint_reports_upstream_pin() {
        let app = create_router(fresh_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/cert/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["upstream_version"], crate::UPSTREAM_VERSION);
        assert_eq!(v["upstream_source_sha"], crate::UPSTREAM_SOURCE_SHA);
    }

    #[tokio::test]
    async fn issuance_round_trip_via_http() {
        let state = fresh_state();
        let app = create_router(state.clone());

        // 1. Seed a cluster-issuer.
        let body = serde_json::to_vec(&cluster_issuer_body()).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/cert/t-1/cluster-issuers")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // 2. Create a certificate.
        let body = serde_json::to_vec(&cert_body()).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/cert/t-1/certificates")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let id: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let cert_id: Uuid = serde_json::from_value(id["id"].clone()).unwrap();

        // 3. Issue.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/cert/t-1/certificates/{}/issue", cert_id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["new_revision"], 1);
        assert_eq!(v["events"][0]["kind"], "Issued");
    }

    #[tokio::test]
    async fn invalid_certificate_returns_400() {
        let app = create_router(fresh_state());
        let mut c = cert_body();
        c.spec.dns_names.clear();
        let body = serde_json::to_vec(&c).unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/cert/t-1/certificates")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_unknown_certificate_returns_404() {
        let app = create_router(fresh_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/cert/t-1/certificates/{}", Uuid::new_v4()))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn metrics_endpoint_exposes_prometheus_content_type() {
        let app = create_router(fresh_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.starts_with("text/plain"), "got content-type {ct}");
    }

    #[tokio::test]
    async fn metrics_endpoint_includes_upstream_families_after_traffic() {
        let state = fresh_state();
        let app = create_router(state.clone());
        // Trigger a sync event by issuing nothing — register a sync counter manually.
        {
            let mut rs = state.lock().unwrap();
            rs.metrics.record_sync("certificates");
        }
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 16384).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("certmanager_controller_sync_call_count"));
    }

    async fn seed_issued_cert(app: &Router, state: &CertState, tenant: &str) -> Uuid {
        let mut ci = cluster_issuer_body();
        ci.tenant_id = tenant.into();
        let body = serde_json::to_vec(&ci).unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/cert/{}/cluster-issuers", tenant))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = serde_json::to_vec(&cert_body()).unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/cert/{}/certificates", tenant))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id: Uuid = serde_json::from_value(v["id"].clone()).unwrap();
        // Reconcile to materialise the Ready=True condition.
        let _ = state.lock().unwrap().plane.controller().reconcile(tenant, id);
        id
    }

    #[tokio::test]
    async fn verify_endpoint_returns_valid_for_freshly_issued_cert() {
        let state = fresh_state();
        let app = create_router(state.clone());
        let id = seed_issued_cert(&app, &state, "t-1").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/cert/t-1/certificates/{}/verify", id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["valid"], true);
        assert_eq!(v["revoked"], false);
        assert_eq!(v["expired"], false);
    }

    #[tokio::test]
    async fn revoke_endpoint_records_revocation_then_verify_flips_invalid() {
        let state = fresh_state();
        let app = create_router(state.clone());
        let id = seed_issued_cert(&app, &state, "t-1").await;

        // Revoke.
        let body = serde_json::json!({
            "reason": "KeyCompromise",
            "revoked_by": "ops@example.com",
            "note": "lost laptop"
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/cert/t-1/certificates/{}/revoke", id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["reason_code"], 1); // RFC 5280 keyCompromise

        // Verify must now report revoked + invalid.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/cert/t-1/certificates/{}/verify", id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["revoked"], true);
        assert_eq!(v["valid"], false);
        assert_eq!(v["revocation_reason"], "KeyCompromise");
    }

    #[tokio::test]
    async fn revoke_endpoint_records_metric_counter() {
        let state = fresh_state();
        let app = create_router(state.clone());
        let id = seed_issued_cert(&app, &state, "t-1").await;

        let body = serde_json::json!({
            "reason": "Superseded",
            "revoked_by": "ops"
        });
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/cert/t-1/certificates/{}/revoke", id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Metrics exposition must now carry the revocation counter row.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 16384).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("certmanager_certificate_revocation_total"));
        assert!(text.contains("reason=\"superseded\""));
    }

    #[tokio::test]
    async fn verify_unknown_certificate_returns_404() {
        let app = create_router(fresh_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/cert/t-1/certificates/{}/verify",
                        Uuid::new_v4()
                    ))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn revoke_unknown_certificate_returns_404() {
        let app = create_router(fresh_state());
        let body = serde_json::json!({"reason": "Unspecified", "revoked_by": "ops"});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/cert/t-1/certificates/{}/revoke",
                        Uuid::new_v4()
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
