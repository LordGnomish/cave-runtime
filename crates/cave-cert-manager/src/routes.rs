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
use crate::models::{Certificate, CertificateRequest, ClusterIssuer, IssuerResource};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub type CertState = Arc<Mutex<CertControlPlane>>;

pub fn create_router(state: CertState) -> Router {
    Router::new()
        .route("/api/cert/health", get(health))
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
    let cp = state.lock().unwrap();
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
    let mut cp = state.lock().unwrap();
    let id = cp.store.put_certificate(cert);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id}))).into_response()
}

async fn get_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let cp = state.lock().unwrap();
    match cp.store.certificate(&tenant, id) {
        Ok(c) => (StatusCode::OK, Json(c.clone())).into_response(),
        Err(e) => (status_for(&e), Json(error_body(&e))).into_response(),
    }
}

async fn issue_certificate(
    Path((tenant, id)): Path<(String, Uuid)>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let mut cp = state.lock().unwrap();
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

async fn list_requests(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let cp = state.lock().unwrap();
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
    let cp = state.lock().unwrap();
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
    let mut cp = state.lock().unwrap();
    let id = cp.store.put_issuer(issuer);
    (StatusCode::CREATED, Json(serde_json::json!({"id": id})))
}

async fn list_cluster_issuers(
    Path(tenant): Path<String>,
    State(state): State<CertState>,
) -> impl IntoResponse {
    let cp = state.lock().unwrap();
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
    let mut cp = state.lock().unwrap();
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
        Arc::new(Mutex::new(CertControlPlane::new()))
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
}
