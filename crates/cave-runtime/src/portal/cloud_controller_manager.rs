//! Portal page + admin API for `cave-cloud-controller-manager`.
//!
//! Four user-facing screens served under `/portal/ccm/*`, each backed by a
//! JSON API endpoint under `/api/portal/ccm/*`. Data is drawn from a real
//! in-process inventory the runtime owns ([`CcmPortal`]); there are no
//! synthesized numbers.
//!
//! * **`/portal/ccm`** — top-level dashboard: enabled cloud-controller loops,
//!   compiled-in cloud providers, IAM token state, region failover counter,
//!   and the parity report.
//! * **`/portal/ccm/loadbalancers`** — cloud LB inventory + last-applied vs
//!   observed status, finalizer presence, deletion-pending flag.
//! * **`/portal/ccm/instances`** — cloud instance lifecycle state per node:
//!   Running / Shutdown / Terminated / NotFound / Unreachable, with a
//!   "matches kube node" indicator.
//! * **`/portal/ccm/routes`** — cloud route-table sync state: desired vs
//!   current, blackhole list, last sync error.

use std::{
    collections::HashMap,
    sync::Arc,
    time::Instant,
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use cave_cloud_controller_manager::{
    calculate_parity, node_controller::InstanceState, provider_snapshot, ADMIN_CLI_SURFACES,
    ADMIN_HTTP_SURFACES, CLOUD_CONTROLLERS, PROVIDERS, UPSTREAM_VERSION,
};
use serde::Serialize;
use serde_json::json;
use tokio::sync::RwLock;

// ── Shared state ─────────────────────────────────────────────────────────────

/// In-process inventory the portal reads. Each top-level field corresponds to
/// one of the upstream cloud-controller-manager loops.
pub struct CcmPortal {
    pub started_at: Instant,
    /// Per-tenant, per-service LB inventory.
    pub load_balancers: RwLock<Vec<LbEntry>>,
    /// Per-node instance state observed from the cloud SDK.
    pub instances: RwLock<HashMap<String, InstanceEntry>>,
    /// Per-cluster route-table state.
    pub routes: RwLock<RouteState>,
    /// Per-provider IAM token state.
    pub iam_tokens: RwLock<HashMap<String, IamTokenState>>,
    /// Region failover counter — incremented every time
    /// [`record_region_failover`] is called.
    pub region_failovers: RwLock<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LbEntry {
    pub tenant: String,
    pub service: String,
    pub namespace: String,
    pub provider: &'static str,
    pub phase: String, // matches LbPhase variants
    pub published_ip: Option<String>,
    pub finalizer: bool,
    pub deletion_pending: bool,
    pub last_applied_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceEntry {
    pub node_name: String,
    pub provider_id: String,
    pub state: &'static str, // Running | Shutdown | Terminated | NotFound | Unreachable
    pub matches_kube_node: bool,
    pub zone: String,
    pub region: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RouteState {
    pub cluster: String,
    pub desired: Vec<RouteRow>,
    pub current: Vec<String>,
    pub blackhole: Vec<String>,
    pub last_sync_error: Option<String>,
    pub last_sync_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteRow {
    pub node_name: String,
    pub pod_cidr: String,
    pub family: &'static str, // "v4" | "v6"
}

#[derive(Debug, Clone, Serialize)]
pub struct IamTokenState {
    pub provider: String,
    pub expires_at_ms: i64,
    pub refreshed_at_ms: i64,
    pub valid: bool,
}

#[allow(dead_code)] // public state-mutators are entry points reserved for future ingest paths.
impl CcmPortal {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            load_balancers: RwLock::new(Vec::new()),
            instances: RwLock::new(HashMap::new()),
            routes: RwLock::new(RouteState::default()),
            iam_tokens: RwLock::new(HashMap::new()),
            region_failovers: RwLock::new(0),
        }
    }

    pub async fn upsert_lb(&self, e: LbEntry) {
        let mut v = self.load_balancers.write().await;
        if let Some(slot) = v
            .iter_mut()
            .find(|x| x.tenant == e.tenant && x.namespace == e.namespace && x.service == e.service)
        {
            *slot = e;
        } else {
            v.push(e);
        }
    }

    pub async fn upsert_instance(&self, name: impl Into<String>, entry: InstanceEntry) {
        self.instances.write().await.insert(name.into(), entry);
    }

    pub async fn replace_routes(&self, state: RouteState) {
        *self.routes.write().await = state;
    }

    pub async fn upsert_iam_token(&self, provider: impl Into<String>, state: IamTokenState) {
        self.iam_tokens.write().await.insert(provider.into(), state);
    }

    pub async fn record_region_failover(&self) {
        *self.region_failovers.write().await += 1;
    }

    pub async fn snapshot(&self) -> CcmSnapshot {
        let lbs = self.load_balancers.read().await.clone();
        let instances = self.instances.read().await.values().cloned().collect();
        let routes = self.routes.read().await.clone();
        let iam = self.iam_tokens.read().await.values().cloned().collect();
        let failovers = *self.region_failovers.read().await;
        CcmSnapshot {
            uptime_seconds: self.started_at.elapsed().as_secs(),
            load_balancers: lbs,
            instances,
            routes,
            iam_tokens: iam,
            region_failovers: failovers,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CcmSnapshot {
    pub uptime_seconds: u64,
    pub load_balancers: Vec<LbEntry>,
    pub instances: Vec<InstanceEntry>,
    pub routes: RouteState,
    pub iam_tokens: Vec<IamTokenState>,
    pub region_failovers: u64,
}

/// Map a typed `InstanceState` into the static string we serialise to the
/// portal. This is the same set the upstream `nodelifecyclecontroller`
/// emits, kept stable so dashboards can label/colourize.
pub fn instance_state_label(s: InstanceState) -> &'static str {
    match s {
        InstanceState::Running => "Running",
        InstanceState::Shutdown => "Shutdown",
        InstanceState::Terminated => "Terminated",
        InstanceState::NotFound => "NotFound",
        InstanceState::Unreachable => "Unreachable",
    }
}

// ── HTML shell ───────────────────────────────────────────────────────────────

const PAGE_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Cave / cloud-controller-manager</title>
<style>
  body { font: 14px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; margin: 24px; color: #1d1f23; }
  h1 { font-size: 22px; margin: 0 0 8px; }
  h2 { font-size: 16px; margin: 24px 0 8px; border-bottom: 1px solid #d8dde3; padding-bottom: 4px; }
  table { border-collapse: collapse; margin-top: 8px; min-width: 480px; }
  th, td { border: 1px solid #e0e4ea; padding: 6px 10px; text-align: left; }
  th { background: #f4f6fa; font-weight: 600; }
  tr:nth-child(even) td { background: #fafbfd; }
  .pill { display: inline-block; padding: 2px 8px; border-radius: 12px; font-size: 12px; font-weight: 600; }
  .pill-ok { background: #def7e3; color: #11652e; }
  .pill-warn { background: #fdecc4; color: #6e4a0b; }
  .pill-err { background: #fcd7d2; color: #74140d; }
  nav a { margin-right: 12px; text-decoration: none; color: #1357c4; }
  nav a:hover { text-decoration: underline; }
  code { background: #f1f3f7; padding: 1px 4px; border-radius: 4px; font-size: 13px; }
</style>
</head>
<body>
<nav>
  <a href="/portal/ccm">Dashboard</a>
  <a href="/portal/ccm/loadbalancers">Load Balancers</a>
  <a href="/portal/ccm/instances">Instances</a>
  <a href="/portal/ccm/routes">Routes</a>
  <a href="/portal/cm">controller-manager</a>
</nav>
<h1>cave-cloud-controller-manager · <span class="pill pill-ok">healthy</span></h1>
<p>Mirror of <code>k8s.io/cloud-provider</code>, pinned to <code id="ver">…</code>.</p>
<div id="content">Loading …</div>
<script>
  async function fetchJSON(u) { const r = await fetch(u); return r.json(); }
  function escape(s) { return String(s).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c])); }
  function table(headers, rows) {
    let html = '<table><thead><tr>';
    for (const h of headers) html += '<th>' + escape(h) + '</th>';
    html += '</tr></thead><tbody>';
    for (const r of rows) {
      html += '<tr>';
      for (const c of r) html += '<td>' + (c === null || c === undefined ? '' : escape(c)) + '</td>';
      html += '</tr>';
    }
    html += '</tbody></table>';
    return html;
  }
  async function renderDashboard() {
    const d = await fetchJSON('/api/portal/ccm');
    document.getElementById('ver').textContent = d.upstream_version;
    let html = '<h2>Cloud controllers</h2>';
    html += table(['controller','enabled'], d.controllers.map(c => [c, 'yes']));
    html += '<h2>Compiled-in providers</h2>';
    html += table(['provider'], d.providers.map(p => [p]));
    html += '<h2>Region failovers</h2>';
    html += table(['count','uptime_sec'], [[d.region_failovers, d.uptime_seconds]]);
    html += '<h2>Parity</h2>';
    html += table(['file','function','test','surface','overall','stubs'],
      [[`${d.parity.file_parity.matched}/${d.parity.file_parity.total}`,
        `${d.parity.function_parity.matched}/${d.parity.function_parity.total}`,
        `${d.parity.test_parity.matched}/${d.parity.test_parity.total}`,
        `${d.parity.surface_parity.matched}/${d.parity.surface_parity.total}`,
        (d.parity.overall * 100).toFixed(2) + '%',
        d.parity.stubs_detected]]);
    document.getElementById('content').innerHTML = html;
  }
  async function renderLBs() {
    const d = await fetchJSON('/api/portal/ccm/loadbalancers');
    document.getElementById('ver').textContent = `— ${d.total} cloud LB(s)`;
    const rows = d.load_balancers.map(lb => [
      lb.tenant, lb.namespace, lb.service, lb.provider, lb.phase,
      lb.published_ip ?? '—',
      lb.finalizer ? 'yes' : 'no',
      lb.deletion_pending ? 'yes' : 'no',
    ]);
    document.getElementById('content').innerHTML =
      '<h2>Cloud Load Balancer inventory</h2>' +
      table(['tenant','namespace','service','provider','phase','ingress_ip','finalizer','deletion_pending'], rows);
  }
  async function renderInstances() {
    const d = await fetchJSON('/api/portal/ccm/instances');
    document.getElementById('ver').textContent = `— ${d.total} instance(s)`;
    const rows = d.instances.map(i => [
      i.node_name, i.provider_id, i.state, i.matches_kube_node ? 'yes' : 'no', i.zone, i.region,
    ]);
    document.getElementById('content').innerHTML =
      '<h2>Cloud instance state vs kube node</h2>' +
      table(['node','provider_id','state','matches_kube_node','zone','region'], rows);
  }
  async function renderRoutes() {
    const d = await fetchJSON('/api/portal/ccm/routes');
    document.getElementById('ver').textContent = `— cluster ${d.cluster}`;
    let html = '<h2>Desired routes</h2>';
    html += table(['node','pod_cidr','family'], d.desired.map(r => [r.node_name, r.pod_cidr, r.family]));
    html += '<h2>Current cloud routes</h2>';
    html += table(['name'], d.current.map(n => [n]));
    html += '<h2>Blackhole routes</h2>';
    html += table(['name'], d.blackhole.map(n => [n]));
    if (d.last_sync_error) {
      html += '<p><span class="pill pill-err">last sync error</span> ' + escape(d.last_sync_error) + '</p>';
    }
    document.getElementById('content').innerHTML = html;
  }
  const path = location.pathname;
  if (path.endsWith('/loadbalancers')) renderLBs();
  else if (path.endsWith('/instances')) renderInstances();
  else if (path.endsWith('/routes')) renderRoutes();
  else renderDashboard();
</script>
</body>
</html>"#;

// ── Routes ───────────────────────────────────────────────────────────────────

pub fn router(state: Arc<CcmPortal>) -> Router {
    Router::new()
        .route("/portal/ccm", get(page))
        .route("/portal/ccm/loadbalancers", get(page))
        .route("/portal/ccm/instances", get(page))
        .route("/portal/ccm/routes", get(page))
        .route("/api/portal/ccm", get(api_dashboard))
        .route("/api/portal/ccm/loadbalancers", get(api_loadbalancers))
        .route("/api/portal/ccm/instances", get(api_instances))
        .route("/api/portal/ccm/routes", get(api_routes))
        .route("/api/portal/ccm/health", get(api_health))
        .route("/api/portal/ccm/instances/{node}", get(api_instance_one))
        .with_state(state)
}

pub async fn page() -> Html<&'static str> {
    Html(PAGE_TEMPLATE)
}

// ── JSON handlers ────────────────────────────────────────────────────────────

pub async fn api_dashboard(State(state): State<Arc<CcmPortal>>) -> Json<serde_json::Value> {
    let snap = state.snapshot().await;
    let parity = calculate_parity()
        .map(|r| serde_json::to_value(r).unwrap_or(json!({})))
        .unwrap_or_else(|e| json!({ "error": e }));
    Json(json!({
        "module": "cave-cloud-controller-manager",
        "upstream_version": UPSTREAM_VERSION,
        "controllers": CLOUD_CONTROLLERS,
        "providers": PROVIDERS,
        "uptime_seconds": snap.uptime_seconds,
        "totals": {
            "load_balancers": snap.load_balancers.len(),
            "instances": snap.instances.len(),
            "iam_tokens": snap.iam_tokens.len(),
        },
        "region_failovers": snap.region_failovers,
        "snapshot": provider_snapshot(),
        "admin_http_surfaces": ADMIN_HTTP_SURFACES,
        "admin_cli_surfaces": ADMIN_CLI_SURFACES,
        "parity": parity,
    }))
}

pub async fn api_loadbalancers(State(state): State<Arc<CcmPortal>>) -> Json<serde_json::Value> {
    let lbs = state.load_balancers.read().await.clone();
    Json(json!({ "total": lbs.len(), "load_balancers": lbs }))
}

pub async fn api_instances(State(state): State<Arc<CcmPortal>>) -> Json<serde_json::Value> {
    let map = state.instances.read().await;
    let v: Vec<_> = map.values().cloned().collect();
    Json(json!({ "total": v.len(), "instances": v }))
}

pub async fn api_instance_one(
    State(state): State<Arc<CcmPortal>>,
    Path(node): Path<String>,
) -> Response {
    let map = state.instances.read().await;
    match map.get(&node) {
        Some(i) => Json(i.clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("unknown node {node}") })),
        )
            .into_response(),
    }
}

pub async fn api_routes(State(state): State<Arc<CcmPortal>>) -> Json<serde_json::Value> {
    let r = state.routes.read().await.clone();
    Json(serde_json::to_value(r).unwrap_or(json!({})))
}

pub async fn api_health(State(state): State<Arc<CcmPortal>>) -> Json<serde_json::Value> {
    let snap = state.snapshot().await;
    Json(json!({
        "status": "healthy",
        "module": "cave-cloud-controller-manager",
        "upstream_version": UPSTREAM_VERSION,
        "controllers_active": CLOUD_CONTROLLERS.len(),
        "providers": PROVIDERS,
        "load_balancers": snap.load_balancers.len(),
        "instances": snap.instances.len(),
        "iam_tokens_loaded": snap.iam_tokens.len(),
        "region_failovers": snap.region_failovers,
    }))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn mk_state() -> Arc<CcmPortal> {
        Arc::new(CcmPortal::new())
    }

    fn mk_lb(svc: &str, phase: &str) -> LbEntry {
        LbEntry {
            tenant: "acme".into(),
            service: svc.into(),
            namespace: "default".into(),
            provider: "hetzner",
            phase: phase.into(),
            published_ip: Some("203.0.113.50".into()),
            finalizer: true,
            deletion_pending: false,
            last_applied_at_ms: Some(1730000000),
        }
    }

    fn mk_instance(name: &str, state: InstanceState) -> InstanceEntry {
        InstanceEntry {
            node_name: name.into(),
            provider_id: "hcloud://1".into(),
            state: instance_state_label(state),
            matches_kube_node: true,
            zone: "fsn1-dc14".into(),
            region: "fsn1".into(),
        }
    }

    #[test]
    fn instance_state_label_covers_every_variant() {
        for s in [
            InstanceState::Running,
            InstanceState::Shutdown,
            InstanceState::Terminated,
            InstanceState::NotFound,
            InstanceState::Unreachable,
        ] {
            assert!(!instance_state_label(s).is_empty());
        }
    }

    #[tokio::test]
    async fn upsert_lb_replaces_by_tenant_namespace_service_tuple() {
        let s = mk_state();
        s.upsert_lb(mk_lb("web", "Ensure")).await;
        s.upsert_lb(mk_lb("web", "Update")).await;
        s.upsert_lb(mk_lb("api", "Ensure")).await;
        let snap = s.snapshot().await;
        assert_eq!(snap.load_balancers.len(), 2);
        let web = snap.load_balancers.iter().find(|l| l.service == "web").unwrap();
        assert_eq!(web.phase, "Update");
    }

    #[tokio::test]
    async fn upsert_instance_indexes_by_node_name() {
        let s = mk_state();
        s.upsert_instance("worker-1", mk_instance("worker-1", InstanceState::Running))
            .await;
        s.upsert_instance("worker-2", mk_instance("worker-2", InstanceState::Shutdown))
            .await;
        let snap = s.snapshot().await;
        assert_eq!(snap.instances.len(), 2);
    }

    #[tokio::test]
    async fn replace_routes_writes_full_state() {
        let s = mk_state();
        let st = RouteState {
            cluster: "prod".into(),
            desired: vec![RouteRow {
                node_name: "n1".into(),
                pod_cidr: "10.0.1.0/24".into(),
                family: "v4",
            }],
            current: vec!["prod-n1".into()],
            blackhole: vec![],
            last_sync_error: None,
            last_sync_at_ms: Some(1730000000),
        };
        s.replace_routes(st.clone()).await;
        let snap = s.snapshot().await;
        assert_eq!(snap.routes.cluster, "prod");
        assert_eq!(snap.routes.desired.len(), 1);
    }

    #[tokio::test]
    async fn region_failover_counter_monotonic() {
        let s = mk_state();
        s.record_region_failover().await;
        s.record_region_failover().await;
        s.record_region_failover().await;
        let snap = s.snapshot().await;
        assert_eq!(snap.region_failovers, 3);
    }

    #[tokio::test]
    async fn iam_token_upsert_indexes_by_provider() {
        let s = mk_state();
        s.upsert_iam_token(
            "hetzner",
            IamTokenState {
                provider: "hetzner".into(),
                expires_at_ms: 1730000000,
                refreshed_at_ms: 1729000000,
                valid: true,
            },
        )
        .await;
        let snap = s.snapshot().await;
        assert_eq!(snap.iam_tokens.len(), 1);
        assert_eq!(snap.iam_tokens[0].provider, "hetzner");
    }

    #[tokio::test]
    async fn dashboard_endpoint_returns_required_keys() {
        let s = mk_state();
        s.upsert_lb(mk_lb("web", "Ensure")).await;
        let app = router(s);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["module"], "cave-cloud-controller-manager");
        assert_eq!(v["upstream_version"], UPSTREAM_VERSION);
        assert!(v["controllers"].as_array().unwrap().len() >= 5);
        assert!(v["providers"].as_array().unwrap().len() >= 2);
        assert_eq!(v["totals"]["load_balancers"], 1);
        assert!(v["parity"]["overall"].is_number());
    }

    #[tokio::test]
    async fn loadbalancers_endpoint_returns_inventory() {
        let s = mk_state();
        s.upsert_lb(mk_lb("web", "Ensure")).await;
        s.upsert_lb(mk_lb("api", "Update")).await;
        let app = router(s);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm/loadbalancers")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["total"], 2);
    }

    #[tokio::test]
    async fn instances_endpoint_returns_inventory() {
        let s = mk_state();
        s.upsert_instance("worker-1", mk_instance("worker-1", InstanceState::Running))
            .await;
        let app = router(s);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm/instances")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["total"], 1);
        assert_eq!(v["instances"][0]["node_name"], "worker-1");
        assert_eq!(v["instances"][0]["state"], "Running");
    }

    #[tokio::test]
    async fn instance_one_endpoint_404s_for_unknown_node() {
        let app = router(mk_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm/instances/nonexistent")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn instance_one_endpoint_returns_known_node() {
        let s = mk_state();
        s.upsert_instance("worker-1", mk_instance("worker-1", InstanceState::Shutdown))
            .await;
        let app = router(s);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm/instances/worker-1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["node_name"], "worker-1");
        assert_eq!(v["state"], "Shutdown");
    }

    #[tokio::test]
    async fn routes_endpoint_returns_state() {
        let s = mk_state();
        s.replace_routes(RouteState {
            cluster: "prod".into(),
            desired: vec![RouteRow {
                node_name: "n1".into(),
                pod_cidr: "10.0.1.0/24".into(),
                family: "v4",
            }],
            current: vec!["prod-n1".into(), "prod-orphan".into()],
            blackhole: vec!["prod-orphan".into()],
            last_sync_error: None,
            last_sync_at_ms: None,
        })
        .await;
        let app = router(s);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm/routes")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["cluster"], "prod");
        assert_eq!(v["blackhole"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn health_endpoint_reports_inventory_counts() {
        let s = mk_state();
        s.upsert_lb(mk_lb("web", "Ensure")).await;
        s.upsert_instance("w1", mk_instance("w1", InstanceState::Running))
            .await;
        let app = router(s);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/ccm/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["status"], "healthy");
        assert_eq!(v["load_balancers"], 1);
        assert_eq!(v["instances"], 1);
    }

    #[tokio::test]
    async fn html_pages_render_at_each_route() {
        for path in [
            "/portal/ccm",
            "/portal/ccm/loadbalancers",
            "/portal/ccm/instances",
            "/portal/ccm/routes",
        ] {
            let app = router(mk_state());
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "path {path}");
            let bytes = to_bytes(resp.into_body(), 4 << 20).await.unwrap();
            let body = std::str::from_utf8(&bytes).unwrap();
            assert!(body.contains("cave-cloud-controller-manager"));
            assert!(body.contains("/portal/ccm"));
        }
    }

    #[tokio::test]
    async fn snapshot_uptime_increases_monotonically() {
        let s = mk_state();
        let s1 = s.snapshot().await;
        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = s.snapshot().await;
        assert!(s2.uptime_seconds >= s1.uptime_seconds);
    }
}
