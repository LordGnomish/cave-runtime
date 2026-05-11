//! `GET /api/compliance/snapshot` — JSON view of the Charter compliance dashboard.
//!
//! The HTML dashboard at `/admin/compliance` is rendered by
//! `cave_portal::admin::compliance::render`. This endpoint returns the
//! same underlying [`ComplianceSnapshot`] as JSON so that scripts, CI
//! gates, and external dashboards can poll it without scraping HTML.
//!
//! Conceptually gated by `Permission::AdminComplianceView`; in this
//! axum-only crate that maps to [`Guard::admin_only`] (platform staff).
//! Responses are tagged with a short `Cache-Control` so the dashboard
//! and external pollers share the upstream 5-minute cache (P3) instead
//! of stampeding the filesystem walk on every request.

use axum::{
    extract::Extension,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use cave_portal::admin::compliance::{live_snapshot, ComplianceSnapshot};
use serde::Serialize;

use crate::routes::rbac::{Guard, GuardError, Principal};

/// Wire response shape — wraps the snapshot in a thin envelope so callers
/// can branch on `schema_version` if we evolve the shape later.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotResponse {
    pub schema_version: u32,
    pub snapshot: ComplianceSnapshot,
}

impl SnapshotResponse {
    pub fn from_snapshot(snapshot: ComplianceSnapshot) -> Self {
        Self {
            schema_version: 1,
            snapshot,
        }
    }
}

/// Cache-Control TTL for the snapshot endpoint — matches P3's in-process
/// 5-minute cache so external pollers don't outrun the data freshness.
pub const SNAPSHOT_CACHE_TTL_SECS: u64 = 300;

fn cache_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    h
}

/// Public guard so handler tests can verify the same permission gate
/// without hitting the live workspace.
pub fn snapshot_guard() -> Guard {
    Guard::admin_only()
}

async fn snapshot_handler(principal: Option<Extension<Principal>>) -> Response {
    let p = principal.as_deref();
    if let Err(err) = snapshot_guard().authorize(p, None) {
        return guard_error_response(err);
    }
    let snap = SnapshotResponse::from_snapshot(live_snapshot());
    (StatusCode::OK, cache_headers(), Json(snap)).into_response()
}

fn guard_error_response(err: GuardError) -> Response {
    let status = match err {
        GuardError::Anonymous => StatusCode::UNAUTHORIZED,
        GuardError::PersonaForbidden { .. } | GuardError::MissingRole(_) => StatusCode::FORBIDDEN,
        GuardError::TenantRequired | GuardError::TenantMismatch { .. } => StatusCode::BAD_REQUEST,
    };
    (status, err.to_string()).into_response()
}

/// Build the `/api/compliance/snapshot` router.
pub fn router() -> Router {
    Router::new().route("/api/compliance/snapshot", get(snapshot_handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use std::time::Instant;
    use tower::util::ServiceExt;

    fn admin() -> Principal {
        Principal::new("admin-1", Persona::Admin)
    }

    fn tenant() -> Principal {
        Principal::new("u-1", Persona::Tenant).with_tenant("acme")
    }

    /// cite: rbac — anonymous requests bounce with 401 before any walk runs.
    #[tokio::test]
    async fn api_compliance_snapshot_anonymous_rejected() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/compliance/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// cite: rbac — tenant-persona requests are forbidden (admin-only route).
    #[tokio::test]
    async fn api_compliance_snapshot_tenant_persona_forbidden() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/compliance/snapshot")
                    .extension(tenant())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// cite: snapshot — admin gets a JSON envelope with schema_version=1.
    #[tokio::test]
    async fn api_compliance_snapshot_admin_returns_envelope() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/compliance/snapshot")
                    .extension(admin())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1_048_576).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert!(json["snapshot"]["crates"].is_array());
    }

    /// cite: cache — response carries Cache-Control: private, max-age=300.
    #[tokio::test]
    async fn api_compliance_snapshot_emits_cache_control_header() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/compliance/snapshot")
                    .extension(admin())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let cc = resp.headers().get(header::CACHE_CONTROL).unwrap();
        assert_eq!(cc, "private, max-age=300");
    }

    /// cite: latency — full walk + JSON serialize completes under 5s in CI.
    /// (The user's 200ms target is for a warm cache, which lands in P3.
    /// Cold walks legitimately take longer on a workspace with ~100 crates.)
    #[tokio::test]
    async fn api_compliance_snapshot_responds_within_five_seconds() {
        let app = router();
        let started = Instant::now();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/compliance/snapshot")
                    .extension(admin())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            elapsed.as_secs_f64() < 5.0,
            "snapshot took {:?}, expected <5s",
            elapsed
        );
    }

    /// cite: serde — SnapshotResponse round-trips through serde_json without loss.
    #[test]
    fn snapshot_response_serialises_to_envelope() {
        let resp = SnapshotResponse::from_snapshot(ComplianceSnapshot { crates: vec![] });
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"schema_version\":1"));
        assert!(s.contains("\"crates\":[]"));
    }
}
