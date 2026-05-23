// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! axum router with the umbrella's HTTP surface:
//!
//!   GET  /healthz         — liveness probe target (cluster phase = Running)
//!   GET  /readyz          — readiness probe target (all components Healthy)
//!   GET  /version         — kubernetes version pin + cave-k8s version
//!   GET  /api             — core /api discovery
//!   GET  /apis            — aggregated /apis discovery
//!   GET  /openapi/v3      — composed OpenAPI v3 schema doc
//!   GET  /metrics         — Prometheus text-format metrics
//!   GET  /api/cluster     — umbrella ClusterStatus summary

use crate::cluster::{ClusterConfig, ControlPlane};
use crate::discovery::Discovery;
use crate::observability_metrics::MetricRegistry;
use crate::openapi::OpenApiAggregator;
use crate::state::State;
use axum::{
    extract::Extension,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub kubernetes: &'static str,
    pub cave_k8s: &'static str,
    pub source_sha: &'static str,
}

pub fn create_router(state: Arc<State>) -> Router {
    let metrics = Arc::new(MetricRegistry::new());
    let cp = Arc::new(ControlPlane::new(ClusterConfig::default()).with_state(state.clone()));
    cp.start();
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/version", get(version))
        .route("/api", get(api_index))
        .route("/apis", get(apis_index))
        .route("/openapi/v3", get(openapi_v3))
        .route("/metrics", get(metrics_scrape))
        .route("/api/cluster", get(cluster_status))
        .layer(Extension(state))
        .layer(Extension(cp))
        .layer(Extension(metrics))
}

async fn healthz(Extension(cp): Extension<Arc<ControlPlane>>) -> &'static str {
    match cp.phase() {
        crate::models::ClusterPhase::Running => "ok",
        _ => "not-ready",
    }
}

async fn readyz(Extension(cp): Extension<Arc<ControlPlane>>) -> &'static str {
    let s = cp.status();
    if s.is_healthy() {
        "ok"
    } else {
        "degraded"
    }
}

async fn version() -> Json<VersionInfo> {
    Json(VersionInfo {
        kubernetes: crate::UPSTREAM_VERSION,
        cave_k8s: env!("CARGO_PKG_VERSION"),
        source_sha: crate::UPSTREAM_SHA,
    })
}

async fn api_index() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "kind": "APIVersions",
        "versions": ["v1"],
        "serverAddressByClientCIDRs": []
    }))
}

async fn apis_index(Extension(state): Extension<Arc<State>>) -> Json<crate::discovery::DiscoveryDoc> {
    let d = Discovery::new(state.crds.clone(), state.aggregator.clone());
    Json(d.doc())
}

async fn openapi_v3(Extension(state): Extension<Arc<State>>) -> Json<crate::openapi::OpenApiDoc> {
    let a = OpenApiAggregator::new(state.crds.clone());
    Json(a.compose())
}

async fn metrics_scrape(Extension(metrics): Extension<Arc<MetricRegistry>>) -> String {
    metrics.scrape_text()
}

async fn cluster_status(
    Extension(cp): Extension<Arc<ControlPlane>>,
) -> Json<crate::cluster::ClusterStatus> {
    Json(cp.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_constructs_without_panic() {
        let _ = create_router(Arc::new(State::default()));
    }

    #[test]
    fn version_info_uses_pinned_values() {
        let v = VersionInfo {
            kubernetes: crate::UPSTREAM_VERSION,
            cave_k8s: env!("CARGO_PKG_VERSION"),
            source_sha: crate::UPSTREAM_SHA,
        };
        assert_eq!(v.kubernetes, "v1.32.0");
        assert_eq!(v.source_sha.len(), 40);
    }

    #[test]
    fn metric_registry_attached_to_router_unique_aware() {
        // Construct twice; each call instantiates its own registry.
        let _r1 = create_router(Arc::new(State::default()));
        let _r2 = create_router(Arc::new(State::default()));
    }

    #[test]
    fn version_serializes_with_camelcase_friendly_layout() {
        let v = VersionInfo {
            kubernetes: "v1.32.0",
            cave_k8s: "0.1.0",
            source_sha: "abc",
        };
        let s = serde_json::to_string(&v).unwrap();
        assert!(s.contains("kubernetes"));
        assert!(s.contains("cave_k8s"));
        assert!(s.contains("source_sha"));
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn body_bytes(resp: axum::http::Response<axum::body::Body>) -> Vec<u8> {
        let limit = 1_048_576;
        to_bytes(resp.into_body(), limit).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn healthz_returns_ok_after_bootstrap() {
        let app = create_router(Arc::new(State::default()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let b = body_bytes(resp).await;
        assert_eq!(b, b"ok");
    }

    #[tokio::test]
    async fn version_returns_pinned_values() {
        let app = create_router(Arc::new(State::default()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/version")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let b = body_bytes(resp).await;
        let s = String::from_utf8(b).unwrap();
        assert!(s.contains("v1.32.0"));
    }

    #[tokio::test]
    async fn apis_index_lists_groups() {
        let app = create_router(Arc::new(State::default()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/apis")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let s = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(s.contains("apps"));
    }

    #[tokio::test]
    async fn openapi_returns_v3_doc() {
        let app = create_router(Arc::new(State::default()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/openapi/v3")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let s = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(s.contains("\"openapi\":\"3.0.0\""));
    }

    #[tokio::test]
    async fn metrics_endpoint_emits_prom_text() {
        let app = create_router(Arc::new(State::default()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let s = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(s.contains("# HELP cave_k8s_pod_count"));
    }
}
