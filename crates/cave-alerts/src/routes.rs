// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP API — Alertmanager v2 surface.
//!
//! - `GET    /api/alerts/health`              health
//! - `GET    /api/v2/status`                  cluster + version
//! - `GET    /api/v2/alerts`                  list alerts (tenant-scoped)
//! - `POST   /api/v2/alerts`                  ingest alerts
//! - `GET    /api/v2/alerts/groups`           grouped firing alerts
//! - `DELETE /api/v2/alerts/:id`              delete a single alert
//! - `GET    /api/v2/silences`                list silences
//! - `POST   /api/v2/silences`                create a silence
//! - `GET    /api/v2/silence/:id`             single silence
//! - `DELETE /api/v2/silence/:id`             expire (delete) a silence
//! - `GET    /api/v2/inhibits`                list inhibit rules
//! - `POST   /api/v2/inhibits`                create an inhibit rule
//! - `DELETE /api/v2/inhibits/:id`            delete an inhibit rule
//! - `GET    /api/v2/receivers`               list receivers
//! - `POST   /api/v2/receivers`               upsert a receiver
//! - `DELETE /api/v2/receivers/:name`         delete a receiver
//! - `GET    /api/v2/route`                   read root route
//! - `PUT    /api/v2/route`                   set root route
//!
//! Tenancy: every list endpoint scopes by `X-Scope-OrgID`; create endpoints
//! stamp the tenant onto the new resource.

use crate::engine::{PipelineInput, run_pipeline};
use crate::models::{Alert, DEFAULT_TENANT, InhibitRule, Receiver, Route, Silence};
use crate::store::AlertStore;
use crate::tenant::tenant_from_headers;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post, put},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ─── State ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub store: AlertStore,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            store: AlertStore::new(),
        }
    }
}

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/alerts/health", get(health))
        .route("/api/v2/status", get(status))
        .route("/api/v2/alerts", get(list_alerts).post(post_alerts))
        .route("/api/v2/alerts/groups", get(list_alert_groups))
        .route("/api/v2/alerts/{id}", delete(delete_alert))
        .route("/api/v2/silences", get(list_silences).post(post_silence))
        .route(
            "/api/v2/silence/{id}",
            get(get_silence).delete(delete_silence),
        )
        .route("/api/v2/inhibits", get(list_inhibits).post(post_inhibit))
        .route("/api/v2/inhibits/{id}", delete(delete_inhibit))
        .route("/api/v2/receivers", get(list_receivers).post(post_receiver))
        .route("/api/v2/receivers/{name}", delete(delete_receiver))
        .route("/api/v2/route", get(get_route).put(put_route))
        .with_state(state)
}

// ─── Handlers ─────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-alerts",
        "status": "ok",
        "upstream": "Alertmanager"
    }))
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    cluster: ClusterStatus,
    version: VersionInfo,
    uptime: String,
}

#[derive(Debug, Serialize)]
struct ClusterStatus {
    status: &'static str,
    peers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct VersionInfo {
    version: &'static str,
    revision: &'static str,
    branch: &'static str,
}

async fn status() -> Json<StatusResponse> {
    Json(StatusResponse {
        cluster: ClusterStatus {
            status: "ready",
            peers: vec![],
        },
        version: VersionInfo {
            version: env!("CARGO_PKG_VERSION"),
            revision: "cave",
            branch: "main",
        },
        uptime: Utc::now().to_rfc3339(),
    })
}

// ─── Alerts ────────────────────────────────────────────────────────────────

async fn list_alerts(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Json<Vec<Alert>> {
    let tenant = tenant_from_headers(&headers);
    Json(state.store.list_alerts(Some(&tenant)))
}

async fn post_alerts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut alerts): Json<Vec<Alert>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let tenant = tenant_from_headers(&headers);
    crate::tenant::inject_tenant(&mut alerts, &tenant);
    for a in alerts.iter() {
        state.store.upsert_alert(a.clone());
    }
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "ingested": alerts.len() })),
    )
}

#[derive(Debug, Serialize, Deserialize)]
struct AlertGroup {
    receiver: String,
    group_key: String,
    alerts_firing: Vec<Alert>,
    alerts_resolved: Vec<Alert>,
}

async fn list_alert_groups(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Vec<AlertGroup>> {
    let tenant = tenant_from_headers(&headers);
    let alerts = state.store.list_alerts(Some(&tenant));
    let root = state
        .store
        .get_root_route()
        .unwrap_or_else(|| Route::root("default"));
    let silences = state.store.list_silences(Some(&tenant));
    let rules = state.store.list_inhibit_rules(Some(&tenant));
    let receivers = state.store.receiver_map();
    let groups = run_pipeline(
        PipelineInput {
            root_route: &root,
            silences: &silences,
            inhibit_rules: &rules,
            receivers: &receivers,
            now: Utc::now(),
        },
        alerts,
    );
    Json(
        groups
            .into_iter()
            .map(|g| AlertGroup {
                receiver: g.decision.receivers.join(","),
                group_key: g.group_key,
                alerts_firing: g.firing,
                alerts_resolved: g.resolved,
            })
            .collect(),
    )
}

async fn delete_alert(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> StatusCode {
    if state.store.delete_alert(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Silences ─────────────────────────────────────────────────────────────

async fn list_silences(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Vec<Silence>> {
    let tenant = tenant_from_headers(&headers);
    Json(state.store.list_silences(Some(&tenant)))
}

async fn post_silence(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut silence): Json<Silence>,
) -> Json<Silence> {
    let tenant = tenant_from_headers(&headers);
    silence.tenant_id = tenant;
    if silence.id.is_nil() {
        silence.id = Uuid::new_v4();
    }
    Json(state.store.create_silence(silence))
}

async fn get_silence(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Silence>, StatusCode> {
    state
        .store
        .get_silence(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_silence(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> StatusCode {
    if state.store.delete_silence(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Inhibit rules ─────────────────────────────────────────────────────────

async fn list_inhibits(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Vec<InhibitRule>> {
    let tenant = tenant_from_headers(&headers);
    Json(state.store.list_inhibit_rules(Some(&tenant)))
}

async fn post_inhibit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut rule): Json<InhibitRule>,
) -> Json<InhibitRule> {
    let tenant = tenant_from_headers(&headers);
    rule.tenant_id = tenant;
    if rule.id.is_nil() {
        rule.id = Uuid::new_v4();
    }
    Json(state.store.create_inhibit_rule(rule))
}

async fn delete_inhibit(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> StatusCode {
    if state.store.delete_inhibit_rule(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Receivers ────────────────────────────────────────────────────────────

async fn list_receivers(State(state): State<Arc<AppState>>) -> Json<Vec<Receiver>> {
    Json(state.store.list_receivers())
}

async fn post_receiver(
    State(state): State<Arc<AppState>>,
    Json(receiver): Json<Receiver>,
) -> Json<Receiver> {
    state.store.upsert_receiver(receiver.clone());
    Json(receiver)
}

async fn delete_receiver(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> StatusCode {
    if state.store.delete_receiver(&name) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Route tree ────────────────────────────────────────────────────────────

async fn get_route(State(state): State<Arc<AppState>>) -> Result<Json<Route>, StatusCode> {
    state
        .store
        .get_root_route()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
struct PutRoute {
    #[serde(flatten)]
    route: Route,
}

async fn put_route(State(state): State<Arc<AppState>>, Json(body): Json<PutRoute>) -> StatusCode {
    state.store.set_root_route(body.route);
    StatusCode::NO_CONTENT
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertSeverity, AlertState, Matcher, ReceiverConfig, WebhookConfig};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Duration;
    use std::collections::HashMap;
    use tower::ServiceExt;

    fn app() -> (Router, Arc<AppState>) {
        let state = Arc::new(AppState::default());
        (create_router(state.clone()), state)
    }

    fn alert(name: &str, fp: &str) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: name.into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: fp.into(),
            tenant_id: DEFAULT_TENANT.into(),
            generator_url: None,
        }
    }

    async fn json<T: for<'de> serde::de::Deserialize<'de>>(
        router: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        tenant: Option<&str>,
    ) -> (StatusCode, T) {
        let mut req = Request::builder().method(method).uri(uri);
        req = req.header("content-type", "application/json");
        if let Some(t) = tenant {
            req = req.header(crate::tenant::X_SCOPE_ORG_ID, t);
        }
        let body = match body {
            Some(v) => Body::from(serde_json::to_vec(&v).unwrap()),
            None => Body::empty(),
        };
        let req = req.body(body).unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed = serde_json::from_slice(&bytes).unwrap();
        (status, parsed)
    }

    #[tokio::test]
    async fn test_health_returns_ok() {
        let (router, _) = app();
        let (status, body): (_, serde_json::Value) =
            json(&router, "GET", "/api/alerts/health", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let (router, _) = app();
        let (status, body): (_, serde_json::Value) =
            json(&router, "GET", "/api/v2/status", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["cluster"]["status"], "ready");
    }

    #[tokio::test]
    async fn test_post_and_list_alerts_tenant_scoped() {
        let (router, _) = app();
        let alerts = vec![alert("HighCPU", "fp1")];
        let (status, _): (_, serde_json::Value) = json(
            &router,
            "POST",
            "/api/v2/alerts",
            Some(serde_json::to_value(&alerts).unwrap()),
            Some("acme"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let (status, list): (_, Vec<Alert>) =
            json(&router, "GET", "/api/v2/alerts", None, Some("acme")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list.len(), 1);

        let (_, list_other): (_, Vec<Alert>) =
            json(&router, "GET", "/api/v2/alerts", None, Some("globex")).await;
        assert_eq!(list_other.len(), 0);
    }

    #[tokio::test]
    async fn test_delete_alert_returns_404_for_missing() {
        let (router, _) = app();
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/v2/alerts/{}", Uuid::new_v4()))
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_silence_create_get_delete() {
        let (router, _) = app();
        let s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            Utc::now(),
            Utc::now() + Duration::hours(1),
            "alice",
            "test",
        );
        let (status, created): (_, Silence) = json(
            &router,
            "POST",
            "/api/v2/silences",
            Some(serde_json::to_value(&s).unwrap()),
            Some("acme"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(created.tenant_id, "acme");

        let (status, _): (_, Silence) = json(
            &router,
            "GET",
            &format!("/api/v2/silence/{}", created.id),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/v2/silence/{}", created.id))
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_inhibit_rule_endpoints() {
        let (router, _) = app();
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("a", "b")],
            vec![Matcher::equal("c", "d")],
            vec!["x".into()],
        );
        let (status, created): (_, InhibitRule) = json(
            &router,
            "POST",
            "/api/v2/inhibits",
            Some(serde_json::to_value(&rule).unwrap()),
            Some("acme"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, listed): (_, Vec<InhibitRule>) =
            json(&router, "GET", "/api/v2/inhibits", None, Some("acme")).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
    }

    #[tokio::test]
    async fn test_receivers_endpoints() {
        let (router, _) = app();
        let r = Receiver::new("rcv").with_config(ReceiverConfig::Webhook(WebhookConfig {
            url: "http://x".into(),
            send_resolved: true,
        }));
        let (status, _): (_, Receiver) = json(
            &router,
            "POST",
            "/api/v2/receivers",
            Some(serde_json::to_value(&r).unwrap()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, listed): (_, Vec<Receiver>) =
            json(&router, "GET", "/api/v2/receivers", None, None).await;
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn test_route_set_and_get() {
        let (router, _) = app();
        let route = Route::root("default");
        let req = Request::builder()
            .method("PUT")
            .uri("/api/v2/route")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&route).unwrap()))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let (status, _): (_, Route) = json(&router, "GET", "/api/v2/route", None, None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_alert_groups_endpoint_returns_groups() {
        let (router, state) = app();

        // Set root route + receiver up so the pipeline produces something.
        state.store.set_root_route(Route::root("default"));
        state.store.upsert_receiver(
            Receiver::new("default").with_config(ReceiverConfig::Webhook(WebhookConfig {
                url: "http://x".into(),
                send_resolved: true,
            })),
        );

        // Two alerts with same alertname (default group_by = alertname).
        let mut a1 = alert("HighCPU", "fp1");
        a1.labels.insert("alertname".into(), "HighCPU".into());
        let mut a2 = alert("HighCPU", "fp2");
        a2.labels.insert("alertname".into(), "HighCPU".into());
        let payload = vec![a1, a2];
        let _: (_, serde_json::Value) = json(
            &router,
            "POST",
            "/api/v2/alerts",
            Some(serde_json::to_value(&payload).unwrap()),
            None,
        )
        .await;

        let (status, groups): (_, Vec<AlertGroup>) =
            json(&router, "GET", "/api/v2/alerts/groups", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].alerts_firing.len(), 2);
    }
}
