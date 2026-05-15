// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admin views — per-module Backstage-parity panels gated by WebAuthn + RBAC.
//!
//! Each submodule is a self-contained view with three layers:
//!
//! 1. **Data accessors** (`list_*`, `inspect_*`) — pure functions taking
//!    `&AdminState` + `&RequestCtx`, scoped to the caller's tenant.
//! 2. **Mutators** (where allowed) — gated on a stricter permission than
//!    the read accessors.
//! 3. **`render(...)`** — composes the page shell + tables + htmx hooks.
//!
//! A small axum [`router`] glues them together at the standard URLs.
//! The router takes its own `Arc<AdminState>` so it can be merged into the
//! main portal router without touching `PortalState`.

pub mod runtime_client;
pub mod types;

pub mod permission;
pub mod render;
pub mod state;

pub mod admission;
pub mod ai_obs;
pub mod alerts;
pub mod apiserver;
pub mod artifacts;
pub mod auth;
pub mod backup;
pub mod cache;
pub mod cdc;
pub mod certs;
pub mod chaos;
pub mod chat;
pub mod cloud_controller_manager;
pub mod cluster;
pub mod compliance;
pub mod meta_audit;

/// 2026-05-13 portal-persona fix: `/admin/adr` Architecture Decision
/// Record browser. Platform-only — walks `docs/adr/*.md` and
/// excludes `docs/adr/internal/`.
pub mod adr;

/// 2026-05-13 Portal UX foundation: top bar + sidebar + breadcrumb
/// + footer + command palette (Cmd+K) + keyboard shortcuts (g h/k/c)
/// + dark mode toggle + tooltips + empty states + skeleton loaders
/// + toast notifications. See `layout/mod.rs` for the entry points.
pub mod layout;
pub mod container_scan;
pub mod contributions;
pub mod controller_manager;
pub mod cost;
pub mod cri;
pub mod crm;
pub mod crossplane;
pub mod dashboard;
pub mod dast;
pub mod deploy;
pub mod devlake;
pub mod dns;
pub mod docdb;
pub mod erp;
pub mod etcd;
pub mod forensics;
pub mod gateway;
pub mod gitops_config;
pub mod ha;
pub mod iam;
pub mod incidents;
pub mod infra;
pub mod karpenter;
pub mod knative;
pub mod kubevirt;
pub mod ledger;
pub mod llm_gateway;
pub mod local_llm;
pub mod logs;
pub mod metrics;
pub mod kamaji;
pub mod keda;
pub mod kube_proxy;
pub mod kubelet;
pub mod lakehouse;
pub mod mesh;
pub mod net;
pub mod oncall;
pub mod pam;
pub mod pg;
pub mod pipelines;
pub mod policy;
pub mod rdbms;
pub mod rdbms_operator;
pub mod rollouts;
pub mod sbom;
pub mod scan;
pub mod scheduler;
pub mod search;
pub mod secrets;
pub mod security;
pub mod slo;
pub mod store;
pub mod streams;
pub mod trace;
pub mod tracker;
pub mod uptime;
pub mod upstream;
pub mod vulns;
pub mod workflows;
pub mod tenant_dashboard;
pub mod vault;

// ── 2026-05-11 batch I: upstream-UI parity admin pages ──────────────
pub mod grafana;
pub mod k8s_dashboard;
pub mod kiali;
pub mod loki;
pub mod prometheus;

// ── 2026-05-13 realtime + power-user batch ──────────────────────────
pub mod events;
pub mod audit;
pub mod global_search;
pub mod quick_actions;
pub mod onboarding;
pub mod cluster_live;
pub mod bulk;

// ── 2026-05-13 P1 scratch pages (iceberg / mlflow / litellm) ───────
pub mod iceberg;
pub mod mlflow;
pub mod litellm;

// ── 2026-05-15 cave-auth deep push (sub-agent A6): account + auth_admin ──
pub mod account;
pub mod auth_admin;

use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::Html,
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub use permission::{AuthError, Permission, Persona, RequestCtx};
pub use state::AdminState;
pub use types::{Cite, TenantId, UPSTREAM_VERSION};

use cave_auth::jwt_middleware::JwtClaims;

#[derive(Debug, Deserialize)]
pub struct AdminQuery {
    pub tenant_id: String,
}

/// Helper that turns `(?tenant_id=acme)` plus an authenticated principal
/// into a [`RequestCtx`]. Real deployments derive principal + permissions
/// from session cookies / JWT; for now we accept defaults from a header
/// so integration tests can drive routes directly.
pub fn extract_ctx_from_query(q: AdminQuery) -> RequestCtx {
    // In production this is filled by the cave-auth middleware; here we
    // grant every documented permission and assume the caller completed
    // WebAuthn. Per-route handlers still call `ctx.authorise(...)` so
    // tightening this later is a one-line change.
    let perms = [
        Permission::DashboardRead,
        Permission::EtcdRead,
        Permission::EtcdWatch,
        Permission::CriRead,
        Permission::CriExec,
        Permission::ApiserverRead,
        Permission::IamRead,
        Permission::IamWrite,
        Permission::MeshRead,
        Permission::MeshWrite,
        Permission::PgRead,
        Permission::PgQuery,
        Permission::VaultRead,
        Permission::ContributionsRead,
        Permission::KedaRead,
        Permission::KedaWrite,
        Permission::SchedulerRead,
        Permission::SchedulerWrite,
        Permission::ControllerManagerRead,
        Permission::KubeletRead,
        Permission::KubeletExec,
        Permission::CloudControllerRead,
        Permission::KamajiRead,
        Permission::KamajiWrite,
        Permission::NetRead,
        Permission::NetWrite,
        Permission::RdbmsRead,
        Permission::RdbmsQuery,
        Permission::DocdbRead,
        Permission::DocdbQuery,
        Permission::CacheRead,
        Permission::CacheWrite,
        Permission::RdbmsOperatorRead,
        Permission::RdbmsOperatorFailover,
        Permission::RdbmsOperatorBackup,
        Permission::LakehouseRead,
        Permission::LakehouseSnapshot,
        Permission::StreamsRead,
        Permission::StreamsAdmin,
        Permission::AdminComplianceView,
        Permission::AdminComplianceRefresh,
        Permission::PolicyRead,
        Permission::PolicyWrite,
        Permission::ArtifactsRead,
        Permission::AlertsRead,
        Permission::AlertsAck,
        Permission::BackupRead,
        Permission::BackupTrigger,
        Permission::IncidentsRead,
        Permission::IncidentsWrite,
        Permission::VulnsRead,
        Permission::WorkflowsRead,
        Permission::ChaosRead,
        Permission::ChaosTrigger,
        Permission::SloRead,
        Permission::AiObsRead,
        Permission::ChatRead,
        Permission::CostRead,
        Permission::DastRead,
        Permission::DevlakeRead,
        Permission::ForensicsRead,
        Permission::GatewayRead,
        Permission::InfraRead,
        Permission::PamRead,
        Permission::SbomRead,
        Permission::ScanRead,
        Permission::SecretsBrowserRead,
        Permission::UptimeRead,
        Permission::ClusterRead,
        Permission::KubeProxyRead,
        Permission::StoreRead,
        Permission::MetricsRead,
        Permission::TraceRead,
        Permission::AuthSessionsRead,
        Permission::DashboardRead2,
        Permission::DnsRead,
        Permission::LogsRead,
        Permission::SecurityRead,
        Permission::HaRead,
        Permission::ErpRead,
        Permission::DeployRead,
        Permission::PipelinesRead,
        Permission::RolloutsRead,
        Permission::KnativeRead,
        Permission::LlmGatewayRead,
        Permission::LocalLlmRead,
        Permission::TrackerRead,
        Permission::UpstreamRead,
        Permission::ContainerScanRead,
        Permission::AdmissionRead,
        Permission::CdcRead,
        Permission::CertsRead,
        Permission::CrmRead,
        Permission::CrossplaneRead,
        Permission::GitopsRead,
        Permission::KarpenterRead,
        Permission::KubevirtRead,
        Permission::LedgerRead,
        Permission::OncallRead,
        Permission::SearchRead,
        Permission::KedaScaledObjectRead,
        Permission::KedaScaledObjectWrite,
        Permission::KedaScaledJobRead,
        Permission::KedaScaledJobWrite,
        Permission::KedaTriggerAuthRead,
        Permission::KedaTriggerAuthWrite,
        Permission::KedaScalerCatalog,
        Permission::KedaMetricsRead,
        // 2026-05-11 batch I: upstream-UI parity pages.
        Permission::GrafanaRead,
        Permission::PrometheusRead,
        Permission::LokiRead,
        Permission::K8sDashboardRead,
        Permission::KialiRead,
        // 2026-05-13 realtime + power-user batch.
        Permission::EventsSubscribe,
        Permission::AuditRead,
        Permission::AuditWrite,
        Permission::OnboardRead,
        Permission::OnboardWrite,
        Permission::GlobalSearchRead,
        Permission::QuickActionTrigger,
        Permission::ClusterLiveRead,
        Permission::BulkOpsSubmit,
        // 2026-05-13 P1 scratch pages.
        Permission::IcebergRead,
        Permission::MlflowRead,
        Permission::LiteLlmRead,
    ];
    RequestCtx::developer(&q.tenant_id, &perms)
}

/// Like [`extract_ctx_from_query`], but derives the [`Persona`] from
/// the JWT cookie when one is present. Used by *platform-only*
/// handlers (compliance dashboard, upstream parity, ADR Browser)
/// that must reject `tenant_admin` cookies — the plain extractor
/// defaults to `Persona::PlatformAdmin` to keep the dev
/// `?tenant_id=...` shortcut backwards compatible for tenant-scoped
/// views.
///
/// * No claims  → `Persona::Anonymous` (rejected by every
///   `require_persona` gate).
/// * Claims present → persona derived via [`Persona::from_roles`].
///
/// The permission bag is unchanged from the plain extractor — every
/// dev request keeps grant-all semantics so a handler that opts in
/// only adds the persona check, doesn't lose its existing permission
/// flow.
pub fn extract_ctx_from_query_with_claims(
    q: AdminQuery,
    claims: Option<&JwtClaims>,
) -> RequestCtx {
    let mut ctx = extract_ctx_from_query(q);
    ctx.persona = match claims {
        Some(c) => Persona::from_roles(&c.roles),
        None => Persona::Anonymous,
    };
    ctx.principal = match claims {
        Some(c) => c.sub.clone(),
        None => ctx.principal,
    };
    ctx
}

fn load_contributions() -> Vec<contributions::Contribution> {
    let path = std::env::var("CAVE_CONTRIBUTIONS_JSONL")
        .unwrap_or_else(|_| "tools/night-pump/contributions.jsonl".into());
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    contributions::parse_jsonl(&raw).unwrap_or_default()
}

fn err_to_response(e: impl ToString) -> (StatusCode, Html<String>) {
    (
        StatusCode::FORBIDDEN,
        Html(render::permission_denied(&e.to_string())),
    )
}

async fn etcd_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    etcd::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn cri_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    cri::render_list_page(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn apiserver_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    apiserver::render(&state, &ctx, None).map(Html).map_err(err_to_response)
}

async fn auth_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    iam::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn mesh_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    mesh::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn pg_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    pg::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn vault_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    vault::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn keda_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    if let Err(e) = state.materialise_keda_scaled_objects(&ctx.tenant).await {
        tracing::warn!(error = %e, "keda materialise failed; falling back to cached rows");
    }
    keda::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn keda_scaledobjects_list_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_objects::render_list(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scaledobjects_new_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_objects::render_new_form(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scaledobjects_detail_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path((ns, name)): Path<(String, String)>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_objects::render_detail(&state, &ctx, &ns, &name)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scaledobjects_edit_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path((ns, name)): Path<(String, String)>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_objects::render_edit_yaml(&state, &ctx, &ns, &name)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scaledobjects_delete_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path((ns, name)): Path<(String, String)>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_objects::render_delete_confirm(&state, &ctx, &ns, &name)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scaledjobs_list_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_jobs::render_list(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scaledjobs_detail_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path((ns, name)): Path<(String, String)>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scaled_jobs::render_detail(&state, &ctx, &ns, &name)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_triggerauth_list_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::trigger_authentications::render_list(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_triggerauth_detail_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path((ns, name)): Path<(String, String)>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::trigger_authentications::render_detail(&state, &ctx, &ns, &name)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_scalers_list_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scalers::render(&ctx).map(Html).map_err(err_to_response)
}

async fn keda_scalers_detail_handler(
    Path(kind): Path<String>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::scalers::render_detail(&ctx, &kind)
        .map(Html)
        .map_err(err_to_response)
}

async fn keda_metrics_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::metrics::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

// 2026-05-14 K8s pages consolidation:
// `scheduler_handler` + `kubelet_handler` removed — their routes now
// redirect 308 to /admin/k8s-dashboard/{scheduler/queue,pods}. The
// per-tab content is served from kubelet::*::render_section +
// scheduler::*::render_section via k8s_dash_*_handler wrappers below.

async fn controller_manager_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    controller_manager::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn cloud_controller_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    cloud_controller_manager::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn kamaji_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    kamaji::render(&state, &ctx).map(Html).map_err(err_to_response)
}

/// 2026-05-14 consolidation: `/admin/net` 308-redirects into the
/// unified `/admin/mesh` page so the dashboard has one canonical
/// service-mesh + network surface. Cilium-specific sections
/// (Flows, NetworkPolicies, Nodes, Identities) are composed into
/// the mesh page with anchor IDs preserved (`#net-flows`,
/// `#net-policies`, `#net-nodes`, `#net-identities`), so existing
/// deep-links land on the right tab.
async fn net_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> axum::response::Response {
    use axum::response::{IntoResponse, Redirect};
    let tenant_id = q.tenant_id.clone();
    // Pre-warm the state cache so the redirected mesh page renders
    // fresh net data on first paint.
    let ctx = extract_ctx_from_query(q);
    if let Err(e) = state.materialise_net_endpoints(&ctx.tenant).await {
        tracing::warn!(error = %e, "net materialise failed; mesh page will use cached rows");
    }
    Redirect::permanent(&format!(
        "/admin/mesh?tenant_id={}#net-flows",
        urlencode_query(&tenant_id),
    ))
    .into_response()
}

/// Minimal URL encoder for query-string segments. Mirrors the same
/// helper used by `admin::layout::nav`; kept local here to avoid an
/// extra cross-module import.
fn urlencode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

async fn rdbms_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    rdbms::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn docdb_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    docdb::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn cache_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    cache::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn rdbms_operator_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    rdbms_operator::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn lakehouse_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    lakehouse::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn streams_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    streams::render(&state, &ctx).map(Html).map_err(err_to_response)
}

#[derive(Debug, Deserialize)]
pub struct ComplianceQuery {
    pub tenant_id: String,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
}

/// `/admin/_audit` — consolidated portal-wide audit roll-up.
/// PlatformAdmin only.
async fn meta_audit_handler(
    Query(q): Query<AdminQuery>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    meta_audit::render(&ctx)
        .map(Html)
        .map_err(meta_audit_err_to_response)
}

/// `/admin/_audit.json` — JSON feed consumed by `cavectl portal
/// audit`.
async fn meta_audit_json_handler(
    Query(q): Query<AdminQuery>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<axum::Json<meta_audit::AuditSummary>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    meta_audit::render_json(&ctx)
        .map(axum::Json)
        .map_err(meta_audit_err_to_response)
}

fn meta_audit_err_to_response(
    e: meta_audit::AuditViewError,
) -> (StatusCode, Html<String>) {
    use meta_audit::AuditViewError as E;
    match e {
        E::PersonaRequired => (
            StatusCode::FORBIDDEN,
            Html(format!(
                "<p>{}</p>",
                render::escape("audit dashboard requires platform_admin persona"),
            )),
        ),
        E::Auth(a) => err_to_response(a),
        E::Compliance(c) => err_to_response(c),
    }
}

async fn compliance_handler(
    Query(q): Query<ComplianceQuery>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let view = compliance::ViewQuery {
        sort: q
            .sort
            .as_deref()
            .map(compliance::SortKey::parse)
            .unwrap_or_default(),
        filter: q
            .filter
            .as_deref()
            .map(compliance::FilterMode::parse)
            .unwrap_or_default(),
    };
    let ctx = extract_ctx_from_query_with_claims(
        AdminQuery { tenant_id: q.tenant_id },
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    // Platform-only gate — Charter compliance is cross-tenant.
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    let snap = compliance::cached_snapshot_or_refresh();
    compliance::render_with_view(&snap, &ctx, view)
        .map(Html)
        .map_err(err_to_response)
}

async fn compliance_detail_handler(
    Query(q): Query<AdminQuery>,
    axum::extract::Path(crate_name): axum::extract::Path<String>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    let root = compliance::workspace_root();
    let detail = compliance::build_crate_detail(&root, &crate_name)
        .map_err(|e| err_to_response(e))?;
    compliance::render_detail(&detail, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn compliance_refresh_handler(
    Query(q): Query<AdminQuery>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    compliance::handle_refresh(&ctx).map(Html).map_err(err_to_response)
}

// 2026-05-13 portal-persona fix: ADR Browser. Platform-only — the
// route handler builds the persona-aware ctx, the module re-checks
// the persona inside its `render` so misconfiguration anywhere up
// the chain fails closed.
async fn adr_handler(
    Query(q): Query<AdminQuery>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    adr::render(&ctx).map(Html).map_err(err_to_response)
}

async fn adr_detail_handler(
    Query(q): Query<AdminQuery>,
    axum::extract::Path(stem): axum::extract::Path<String>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    adr::render_detail(&ctx, &stem)
        .map(Html)
        .map_err(|e| match e {
            adr::AdrViewError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Html(render::permission_denied(&e.to_string())),
            ),
            _ => err_to_response(e),
        })
}

async fn policy_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    policy::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn artifacts_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    artifacts::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn alerts_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    alerts::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn backup_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    backup::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn incidents_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    incidents::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn vulns_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    vulns::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn workflows_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    workflows::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn chaos_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    chaos::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn slo_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    slo::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn ai_obs_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); ai_obs::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn chat_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); chat::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn cost_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); cost::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn dast_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); dast::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn devlake_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); devlake::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn forensics_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); forensics::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn gateway_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); gateway::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn infra_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); infra::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn pam_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); pam::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn sbom_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); sbom::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn scan_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); scan::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn secrets_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); secrets::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn uptime_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); uptime::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn cluster_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); cluster::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn kube_proxy_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); kube_proxy::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn store_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); store::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn metrics_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); metrics::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn trace_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); trace::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn auth_sessions_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); auth::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn dashboard_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); dashboard::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn dns_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); dns::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn logs_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); logs::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn security_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); security::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn ha_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); ha::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn erp_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); erp::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn deploy_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); deploy::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn pipelines_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); pipelines::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn rollouts_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); rollouts::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn knative_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); knative::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn llm_gateway_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); llm_gateway::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn local_llm_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); local_llm::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn tracker_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); tracker::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn upstream_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query_with_claims(
        q,
        claims.as_ref().map(|axum::Extension(c)| c),
    );
    // Platform-only — upstream parity is a cross-tenant control-plane view.
    ctx.require_persona(Persona::PlatformAdmin)
        .map_err(err_to_response)?;
    upstream::render(&s, &ctx).map(Html).map_err(err_to_response)
}
async fn container_scan_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); container_scan::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn admission_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); admission::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn cdc_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); cdc::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn certs_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); certs::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn crm_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); crm::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn crossplane_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); crossplane::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn gitops_config_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); gitops_config::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn karpenter_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); karpenter::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn kubevirt_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); kubevirt::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn ledger_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); ledger::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn oncall_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); oncall::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn search_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); search::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn grafana_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); grafana::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn prometheus_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); prometheus::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn loki_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); loki::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn k8s_dashboard_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); k8s_dashboard::render(&s, &ctx).map(Html).map_err(err_to_response) }
/// 2026-05-14 consolidation: `/admin/kiali` 308-redirects into the
/// unified `/admin/mesh` page. Topology / Traffic / Validations /
/// Workloads / Services are composed into mesh; anchor IDs
/// preserved (`#kiali-topology`, `#kiali-traffic`, etc.) so old
/// links keep working.
async fn kiali_handler(
    AxumState(_s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> axum::response::Response {
    use axum::response::{IntoResponse, Redirect};
    Redirect::permanent(&format!(
        "/admin/mesh?tenant_id={}#kiali-topology",
        urlencode_query(&q.tenant_id),
    ))
    .into_response()
}

// ── 2026-05-14 K8s pages consolidation ─────────────────────────────
// Wrap kubelet + scheduler sub-tabs under the canonical
// /admin/k8s-dashboard/ surface. Each handler enforces both the
// K8sDashboardRead gate (for the parent surface) and the per-tab
// upstream permission (KubeletRead / SchedulerRead).

fn k8s_dash_section(
    ctx: &RequestCtx,
    title: &str,
    section: String,
) -> Html<String> {
    Html(render::page_shell_full(
        ctx,
        "/admin/k8s-dashboard",
        &format!("k8s-dashboard / {title} · {}", render::escape(ctx.tenant.as_str())),
        &section,
    ))
}

async fn k8s_dash_pods_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    if let Err(e) = s.materialise_kubelet_pods(&ctx.tenant).await {
        tracing::warn!(error = %e, "kubelet materialise failed; falling back to cached rows");
    }
    let section = kubelet::pods::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "pods", section))
}

async fn k8s_dash_nodes_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = kubelet::nodes::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "nodes", section))
}

async fn k8s_dash_volumes_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = kubelet::volumes::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "volumes", section))
}

async fn k8s_dash_events_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = kubelet::events::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "events", section))
}

async fn k8s_dash_metrics_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = kubelet::metrics::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "metrics", section))
}

async fn k8s_dash_queue_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    if let Err(e) = s.materialise_scheduler_nodes(&ctx.tenant).await {
        tracing::warn!(error = %e, "scheduler materialise failed; falling back to cached rows");
    }
    let section = scheduler::queue::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "scheduler-queue", section))
}

async fn k8s_dash_sched_plugins_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = scheduler::plugins::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "scheduler-plugins", section))
}

async fn k8s_dash_sched_bindings_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = scheduler::bindings::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "scheduler-bindings", section))
}

async fn k8s_dash_sched_nodescores_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = scheduler::nodescores::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "scheduler-nodescores", section))
}

async fn k8s_dash_sched_events_handler(
    AxumState(s): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    ctx.authorise(Permission::K8sDashboardRead).map_err(err_to_response)?;
    let section = scheduler::events::render_section(&s, &ctx).map_err(err_to_response)?;
    Ok(k8s_dash_section(&ctx, "scheduler-events", section))
}

/// Build a 308 Permanent Redirect to the canonical `/admin/k8s-dashboard/...`
/// URL. Preserves the tenant query string.
fn redirect_308(target: String) -> Result<axum::response::Response, (StatusCode, Html<String>)> {
    let mut resp = axum::response::Response::builder()
        .status(axum::http::StatusCode::PERMANENT_REDIRECT)
        .header(axum::http::header::LOCATION, target)
        .body(axum::body::Body::empty())
        .map_err(|e| err_to_response(e.to_string()))?;
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    Ok(resp)
}

fn redirect_with_tenant(target_path: &str, q: &AdminQuery) -> Result<axum::response::Response, (StatusCode, Html<String>)> {
    redirect_308(format!("{target_path}?tenant_id={}", q.tenant_id))
}

// Legacy /admin/kubelet redirects → /admin/k8s-dashboard/pods (landing tab).
async fn legacy_kubelet_redirect(
    Query(q): Query<AdminQuery>,
) -> Result<axum::response::Response, (StatusCode, Html<String>)> {
    redirect_with_tenant("/admin/k8s-dashboard/pods", &q)
}

async fn legacy_scheduler_redirect(
    Query(q): Query<AdminQuery>,
) -> Result<axum::response::Response, (StatusCode, Html<String>)> {
    redirect_with_tenant("/admin/k8s-dashboard/scheduler/queue", &q)
}

// ── 2026-05-13 realtime + power-user handlers ──────────────────────

#[derive(Debug, Deserialize)]
struct AuditQuery {
    tenant_id: String,
    #[serde(default)]
    from_unix: Option<i64>,
    #[serde(default)]
    to_unix: Option<i64>,
    #[serde(default)]
    persona: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    target: Option<String>,
}

fn audit_filter_from(q: &AuditQuery) -> audit::AuditFilter {
    audit::AuditFilter {
        from_unix: q.from_unix,
        to_unix: q.to_unix,
        persona: q.persona.clone(),
        action: q.action.clone(),
        target: q.target.clone(),
    }
}

async fn audit_page_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AuditQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(AdminQuery { tenant_id: q.tenant_id.clone() });
    let filter = audit_filter_from(&q);
    audit::render(state.audit_store.clone(), &ctx, &filter)
        .map(Html)
        .map_err(err_to_response)
}

async fn audit_csv_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AuditQuery>,
) -> Result<axum::response::Response, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(AdminQuery { tenant_id: q.tenant_id.clone() });
    let filter = audit_filter_from(&q);
    let body = audit::export_csv(&state.audit_store, &ctx, &filter).map_err(err_to_response)?;
    let mut resp = axum::response::Response::new(axum::body::Body::from(body));
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_static("attachment; filename=\"audit.csv\""),
    );
    Ok(resp)
}

async fn cluster_live_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let snap = state.cluster_live.read(&ctx).map_err(err_to_response)?;
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Cluster · live</h2>{}</section>"#,
        snap.render_html()
    );
    Ok(Html(render::page_shell_full(
        &ctx,
        "/admin/cluster/live",
        &format!("cluster · {}", render::escape(ctx.tenant.as_str())),
        &body,
    )))
}

async fn onboard_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let progress = state.onboarding.read(&ctx).map_err(err_to_response)?;
    let next = state
        .onboarding
        .next_step(&ctx)
        .map_err(err_to_response)?;
    let pct = progress.percent_complete(ctx.persona);
    let next_html = match next {
        Some(s) => format!(
            r#"<div class="mt-3"><a class="text-blue-700 underline" href="{href}">Next step: {title}</a><p class="text-sm text-gray-600">{desc}</p></div>"#,
            href = render::escape(&s.href),
            title = render::escape(&s.title),
            desc = render::escape(&s.description),
        ),
        None => r#"<div class="mt-3 text-sm text-gray-600">All caught up — tour complete or dismissed.</div>"#.into(),
    };
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Onboarding · {pct}%</h2>{next}</section>"#,
        pct = pct,
        next = next_html,
    );
    Ok(Html(render::page_shell_full(
        &ctx,
        "/admin/onboard",
        &format!("onboard · {}", render::escape(ctx.tenant.as_str())),
        &body,
    )))
}

#[derive(Debug, Deserialize)]
struct GlobalSearchQuery {
    tenant_id: String,
    #[serde(default)]
    q: Option<String>,
}

async fn global_search_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<GlobalSearchQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(AdminQuery { tenant_id: q.tenant_id.clone() });
    let query = q.q.unwrap_or_default();
    let hits = state
        .global_search
        .query(&ctx, &query, 25)
        .map_err(err_to_response)?;
    let rows: String = hits
        .iter()
        .map(|h| {
            format!(
                r#"<li class="py-1"><a class="text-blue-700 underline" href="{href}">{label}</a> <span class="text-xs text-gray-500">({kind})</span></li>"#,
                href = render::escape(&h.doc.href),
                label = render::escape(&h.doc.label),
                kind = match h.doc.kind {
                    global_search::DocKind::Route => "route",
                    global_search::DocKind::Resource => "resource",
                    global_search::DocKind::Command => "command",
                    global_search::DocKind::Crate => "crate",
                },
            )
        })
        .collect();
    let body = format!(
        r#"<section><form><input class="border rounded px-2 py-1" name="q" value="{q}" placeholder="search routes / resources / crates"/><input type="hidden" name="tenant_id" value="{tid}"/></form><ul class="mt-3">{rows}</ul></section>"#,
        q = render::escape(&query),
        tid = render::escape(ctx.tenant.as_str()),
        rows = rows,
    );
    Ok(Html(render::page_shell_full(
        &ctx,
        "/admin/search",
        &format!("search · {}", render::escape(ctx.tenant.as_str())),
        &body,
    )))
}

async fn events_stream_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<axum::response::sse::Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>, (StatusCode, Html<String>)> {
    use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
    let ctx = extract_ctx_from_query(q);
    let sub = state.event_bus.subscribe(ctx).map_err(err_to_response)?;
    let stream = async_stream::stream! {
        let mut sub = sub;
        loop {
            match sub.next_with_timeout(std::time::Duration::from_secs(15)).await {
                Ok(Some(ev)) => {
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    yield Ok::<_, std::convert::Infallible>(SseEvent::default().event(ev.kind()).data(data));
                }
                Ok(None) => {
                    // keepalive handled by axum's KeepAlive helper
                    continue;
                }
                Err(_) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15))))
}

async fn bulk_submit_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
    axum::Json(req): axum::Json<bulk::BulkOpRequest>,
) -> Result<axum::Json<bulk::BulkOpResult>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    // No-op executor by default — submit endpoint is exposed for
    // dashboards that build their own executor via the in-process
    // shared state. For now we emulate the dry-run flow so the
    // round trip is verifiable.
    let exec = bulk::FixedExecutor::new();
    let result = bulk::submit(&ctx, &req, &exec).map_err(err_to_response)?;
    // Best-effort audit record so the activity feed sees the action.
    state.audit_store.record(
        ctx.persona.as_str(),
        audit::AuditAction::Operate,
        format!("bulk/{}", req.kind.as_str()),
        if result.is_full_success() {
            audit::AuditResult::Ok
        } else {
            audit::AuditResult::Error
        },
        format!("targets={} ok={} fail={}", req.targets.len(), result.ok_count, result.fail_count),
    );
    Ok(axum::Json(result))
}

// ── 2026-05-13 P1 scratch handlers ──────────────────────────────────

async fn iceberg_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); iceberg::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn iceberg_tables_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); iceberg::tables::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn iceberg_snapshots_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); iceberg::snapshots::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn iceberg_partitions_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); iceberg::partitions::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn iceberg_schemas_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); iceberg::schemas::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn iceberg_manifests_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); iceberg::manifests::render(&s, &ctx).map(Html).map_err(err_to_response) }

async fn mlflow_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); mlflow::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn mlflow_experiments_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); mlflow::experiments::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn mlflow_runs_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); mlflow::runs::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn mlflow_models_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); mlflow::models::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn mlflow_registered_models_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); mlflow::registered_models::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn mlflow_deployments_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); mlflow::deployments::render(&s, &ctx).map(Html).map_err(err_to_response) }

async fn litellm_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); litellm::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn litellm_models_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); litellm::models::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn litellm_routes_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); litellm::routes::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn litellm_api_keys_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); litellm::api_keys::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn litellm_budgets_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); litellm::budgets::render(&s, &ctx).map(Html).map_err(err_to_response) }
async fn litellm_monitoring_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); litellm::monitoring::render(&s, &ctx).map(Html).map_err(err_to_response) }

async fn tenant_dashboard_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path(tenant): Path<String>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let path_tenant = TenantId::new(tenant).expect("test fixture");
    tenant_dashboard::render(&state, &ctx, &path_tenant)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_overview_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_overview(&recs, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_worker_handler(
    Path(worker_id): Path<String>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_worker_detail(&recs, &worker_id, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_timeline_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_timeline(&recs, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_leaderboard_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_leaderboard(&recs, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

// ── 2026-05-15 Account console (A6) ─────────────────────────────────
async fn account_profile_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    account::profile::render(&ctx).map(Html).map_err(err_to_response)
}
async fn account_password_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    account::password::render(&ctx).map(Html).map_err(err_to_response)
}
async fn account_two_factor_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    account::two_factor::render(&ctx).map(Html).map_err(err_to_response)
}
async fn account_applications_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    account::applications::render(&ctx).map(Html).map_err(err_to_response)
}
async fn account_sessions_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    account::sessions::render(&s, &ctx).map(Html).map_err(err_to_response)
}

// ── 2026-05-15 Auth Admin console (A6) ──────────────────────────────
async fn auth_admin_realms_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::realms::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_clients_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::clients::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_users_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::users::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_roles_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::roles::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_groups_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::groups::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_idp_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::idp::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_flows_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::flows::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_events_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::events::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_saml_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::saml::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_webauthn_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::webauthn::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_ldap_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::ldap::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_kerberos_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::kerberos::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_uma_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::uma::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_token_exchange_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::token_exchange::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_dpop_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::dpop::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_jwe_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::jwe::render(&ctx).map(Html).map_err(err_to_response)
}
async fn auth_admin_oauth_endpoints_handler(Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    auth_admin::oauth_endpoints::render(&ctx).map(Html).map_err(err_to_response)
}

/// Build the admin router. Mount as `app.merge(admin::router(state))`.
pub fn router(state: Arc<AdminState>) -> Router {
    Router::new()
        .route("/admin/etcd", get(etcd_handler))
        .route("/admin/cri", get(cri_handler))
        .route("/admin/apiserver", get(apiserver_handler))
        .route("/admin/auth", get(auth_handler))
        .route("/admin/mesh", get(mesh_handler))
        .route("/admin/pg", get(pg_handler))
        .route("/admin/vault", get(vault_handler))
        .route("/admin/keda", get(keda_handler))
        .route("/admin/keda/scaledobjects", get(keda_scaledobjects_list_handler))
        .route("/admin/keda/scaledobjects/new", get(keda_scaledobjects_new_handler))
        .route(
            "/admin/keda/scaledobjects/{ns}/{name}",
            get(keda_scaledobjects_detail_handler),
        )
        .route(
            "/admin/keda/scaledobjects/{ns}/{name}/edit",
            get(keda_scaledobjects_edit_handler),
        )
        .route(
            "/admin/keda/scaledobjects/{ns}/{name}/delete",
            get(keda_scaledobjects_delete_handler),
        )
        .route("/admin/keda/scaledjobs", get(keda_scaledjobs_list_handler))
        .route(
            "/admin/keda/scaledjobs/{ns}/{name}",
            get(keda_scaledjobs_detail_handler),
        )
        .route(
            "/admin/keda/triggerauthentications",
            get(keda_triggerauth_list_handler),
        )
        .route(
            "/admin/keda/triggerauthentications/{ns}/{name}",
            get(keda_triggerauth_detail_handler),
        )
        .route("/admin/keda/scalers", get(keda_scalers_list_handler))
        .route("/admin/keda/scalers/{kind}", get(keda_scalers_detail_handler))
        .route("/admin/keda/metrics", get(keda_metrics_handler))
        // 2026-05-14 consolidation: legacy top-level routes redirect 308
        // to the canonical /admin/k8s-dashboard/ landing tabs.
        .route("/admin/scheduler", get(legacy_scheduler_redirect))
        .route("/admin/controller-manager", get(controller_manager_handler))
        .route("/admin/kubelet", get(legacy_kubelet_redirect))
        .route("/admin/cloud-controller", get(cloud_controller_handler))
        .route("/admin/kamaji", get(kamaji_handler))
        .route("/admin/net", get(net_handler))
        .route("/admin/rdbms", get(rdbms_handler))
        .route("/admin/docdb", get(docdb_handler))
        .route("/admin/cache", get(cache_handler))
        .route("/admin/rdbms-operator", get(rdbms_operator_handler))
        .route("/admin/lakehouse", get(lakehouse_handler))
        .route("/admin/streams", get(streams_handler))
        .route("/admin/_audit", get(meta_audit_handler))
        .route("/admin/_audit.json", get(meta_audit_json_handler))
        .route("/admin/compliance", get(compliance_handler))
        .route("/admin/compliance/refresh", get(compliance_refresh_handler))
        .route("/admin/compliance/{crate_name}", get(compliance_detail_handler))
        // 2026-05-13 portal-persona fix: ADR Browser (Platform-only).
        .route("/admin/adr", get(adr_handler))
        .route("/admin/adr/{stem}", get(adr_detail_handler))
        .route("/admin/policy", get(policy_handler))
        .route("/admin/artifacts", get(artifacts_handler))
        .route("/admin/alerts", get(alerts_handler))
        .route("/admin/backup", get(backup_handler))
        .route("/admin/incidents", get(incidents_handler))
        .route("/admin/vulns", get(vulns_handler))
        .route("/admin/workflows", get(workflows_handler))
        .route("/admin/chaos", get(chaos_handler))
        .route("/admin/slo", get(slo_handler))
        .route("/admin/ai-obs", get(ai_obs_handler))
        .route("/admin/chat", get(chat_handler))
        .route("/admin/cost", get(cost_handler))
        .route("/admin/dast", get(dast_handler))
        .route("/admin/devlake", get(devlake_handler))
        .route("/admin/forensics", get(forensics_handler))
        .route("/admin/gateway", get(gateway_handler))
        .route("/admin/infra", get(infra_handler))
        .route("/admin/pam", get(pam_handler))
        .route("/admin/sbom", get(sbom_handler))
        .route("/admin/scan", get(scan_handler))
        .route("/admin/secrets", get(secrets_handler))
        .route("/admin/uptime", get(uptime_handler))
        .route("/admin/cluster", get(cluster_handler))
        .route("/admin/kube-proxy", get(kube_proxy_handler))
        .route("/admin/store", get(store_handler))
        .route("/admin/metrics", get(metrics_handler))
        .route("/admin/trace", get(trace_handler))
        .route("/admin/auth-sessions", get(auth_sessions_handler))
        .route("/admin/dashboard-catalog", get(dashboard_handler))
        .route("/admin/dns", get(dns_handler))
        .route("/admin/logs", get(logs_handler))
        .route("/admin/security", get(security_handler))
        .route("/admin/ha", get(ha_handler))
        .route("/admin/erp", get(erp_handler))
        .route("/admin/deploy", get(deploy_handler))
        .route("/admin/pipelines", get(pipelines_handler))
        .route("/admin/rollouts", get(rollouts_handler))
        .route("/admin/knative", get(knative_handler))
        .route("/admin/llm-gateway", get(llm_gateway_handler))
        .route("/admin/local-llm", get(local_llm_handler))
        .route("/admin/tracker", get(tracker_handler))
        .route("/admin/upstream", get(upstream_handler))
        .route("/admin/container-scan", get(container_scan_handler))
        .route("/admin/admission", get(admission_handler))
        .route("/admin/cdc", get(cdc_handler))
        .route("/admin/certs", get(certs_handler))
        .route("/admin/crm", get(crm_handler))
        .route("/admin/crossplane", get(crossplane_handler))
        .route("/admin/gitops-config", get(gitops_config_handler))
        .route("/admin/karpenter", get(karpenter_handler))
        .route("/admin/kubevirt", get(kubevirt_handler))
        .route("/admin/ledger", get(ledger_handler))
        .route("/admin/oncall", get(oncall_handler))
        .route("/admin/search", get(search_handler))
        // 2026-05-11 batch I: upstream-UI parity pages.
        .route("/admin/grafana", get(grafana_handler))
        .route("/admin/prometheus", get(prometheus_handler))
        .route("/admin/loki", get(loki_handler))
        .route("/admin/k8s-dashboard", get(k8s_dashboard_handler))
        // 2026-05-14 K8s pages consolidation — sub-tabs absorbed from
        // /admin/kubelet/* and /admin/scheduler/*.
        .route("/admin/k8s-dashboard/pods", get(k8s_dash_pods_handler))
        .route("/admin/k8s-dashboard/nodes", get(k8s_dash_nodes_handler))
        .route("/admin/k8s-dashboard/volumes", get(k8s_dash_volumes_handler))
        .route("/admin/k8s-dashboard/events", get(k8s_dash_events_handler))
        .route("/admin/k8s-dashboard/metrics", get(k8s_dash_metrics_handler))
        .route("/admin/k8s-dashboard/scheduler/queue", get(k8s_dash_queue_handler))
        .route("/admin/k8s-dashboard/scheduler/plugins", get(k8s_dash_sched_plugins_handler))
        .route("/admin/k8s-dashboard/scheduler/bindings", get(k8s_dash_sched_bindings_handler))
        .route("/admin/k8s-dashboard/scheduler/nodescores", get(k8s_dash_sched_nodescores_handler))
        .route("/admin/k8s-dashboard/scheduler/events", get(k8s_dash_sched_events_handler))
        .route("/admin/kiali", get(kiali_handler))
        .route("/admin/contributions", get(contributions_overview_handler))
        .route("/admin/contributions/timeline", get(contributions_timeline_handler))
        .route("/admin/contributions/leaderboard", get(contributions_leaderboard_handler))
        .route("/admin/contributions/{worker_id}", get(contributions_worker_handler))
        .route("/t/{tenant}/dashboard", get(tenant_dashboard_handler))

        .route("/admin/audit", get(audit_page_handler))
        .route("/admin/audit.csv", get(audit_csv_handler))
        .route("/admin/cluster/live", get(cluster_live_handler))
        .route("/admin/onboard", get(onboard_handler))
        .route("/admin/global-search", get(global_search_handler))
        .route("/api/events/stream", get(events_stream_handler))
        .route("/api/bulk/submit", axum::routing::post(bulk_submit_handler))
        // 2026-05-13 P1 scratch pages.
        .route("/admin/iceberg", get(iceberg_handler))
        .route("/admin/iceberg/tables", get(iceberg_tables_handler))
        .route("/admin/iceberg/snapshots", get(iceberg_snapshots_handler))
        .route("/admin/iceberg/partitions", get(iceberg_partitions_handler))
        .route("/admin/iceberg/schemas", get(iceberg_schemas_handler))
        .route("/admin/iceberg/manifests", get(iceberg_manifests_handler))
        .route("/admin/mlflow", get(mlflow_handler))
        .route("/admin/mlflow/experiments", get(mlflow_experiments_handler))
        .route("/admin/mlflow/runs", get(mlflow_runs_handler))
        .route("/admin/mlflow/models", get(mlflow_models_handler))
        .route("/admin/mlflow/registered-models", get(mlflow_registered_models_handler))
        .route("/admin/mlflow/deployments", get(mlflow_deployments_handler))
        .route("/admin/litellm", get(litellm_handler))
        .route("/admin/litellm/models", get(litellm_models_handler))
        .route("/admin/litellm/routes", get(litellm_routes_handler))
        .route("/admin/litellm/api-keys", get(litellm_api_keys_handler))
        .route("/admin/litellm/budgets", get(litellm_budgets_handler))
        .route("/admin/litellm/monitoring", get(litellm_monitoring_handler))
        // 2026-05-15 Account console (A6).
        .route("/account/profile", get(account_profile_handler))
        .route("/account/password", get(account_password_handler))
        .route("/account/two-factor", get(account_two_factor_handler))
        .route("/account/applications", get(account_applications_handler))
        .route("/account/sessions", get(account_sessions_handler))
        // 2026-05-15 Auth Admin console (A6).
        .route("/admin/auth/realms", get(auth_admin_realms_handler))
        .route("/admin/auth/clients", get(auth_admin_clients_handler))
        .route("/admin/auth/users", get(auth_admin_users_handler))
        .route("/admin/auth/roles", get(auth_admin_roles_handler))
        .route("/admin/auth/groups", get(auth_admin_groups_handler))
        .route("/admin/auth/idp", get(auth_admin_idp_handler))
        .route("/admin/auth/flows", get(auth_admin_flows_handler))
        .route("/admin/auth/events", get(auth_admin_events_handler))
        .route("/admin/auth/saml", get(auth_admin_saml_handler))
        .route("/admin/auth/webauthn", get(auth_admin_webauthn_handler))
        .route("/admin/auth/ldap", get(auth_admin_ldap_handler))
        .route("/admin/auth/kerberos", get(auth_admin_kerberos_handler))
        .route("/admin/auth/uma", get(auth_admin_uma_handler))
        .route("/admin/auth/token-exchange", get(auth_admin_token_exchange_handler))
        .route("/admin/auth/dpop", get(auth_admin_dpop_handler))
        .route("/admin/auth/jwe", get(auth_admin_jwe_handler))
        .route("/admin/auth/oauth-endpoints", get(auth_admin_oauth_endpoints_handler))
        .with_state(state)
}

#[cfg(test)]
mod router_tests {
    use super::*;
    use crate::portal_test_ctx;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::util::ServiceExt;

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn admin_compliance_route_renders_behavioral_parity_card() {
        // End-to-end mount smoke: drive the router through `/admin/compliance`
        // with a forged platform_admin JWT extension (the handler is
        // platform-only since the persona-fix batch on 2026-05-13).
        let (_cite, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Behavioral.tsx",
            "complianceRouteSmoke",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let claims = cave_auth::jwt_middleware::JwtClaims {
            sub: "platform-admin".into(),
            email: "admin@cave".into(),
            roles: vec!["platform_admin".into()],
            exp: 9_999_999_999,
        };
        let mut req = Request::builder()
            .uri("/admin/compliance?tenant_id=acme")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(claims);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(
            body.contains("Behavioral Parity"),
            "expected Behavioral Parity card in live render"
        );
        assert!(
            body.contains(">behavioral<"),
            "expected behavioral column header in live render"
        );
        // Source-of-truth: at least one of the 5 audited crates surfaces
        // a per-row count (cave-scheduler is 13/16 at landing time).
        // We assert the structural shape, not the literal count, so the
        // test stays stable as audits expand.
        assert!(
            body.contains("upstream tests"),
            "expected behavioral card explanatory text"
        );
    }

    /// 2026-05-15 polish — `/admin/_audit` mount smoke. Drives the
    /// router with a forged platform_admin JWT and asserts the
    /// dashboard renders all 5 grade cards. Tenant-admin gets 403.
    #[tokio::test]
    async fn meta_audit_route_renders_five_axis_dashboard() {
        let app = router(Arc::new(AdminState::seeded()));
        let claims = cave_auth::jwt_middleware::JwtClaims {
            sub: "platform-admin".into(),
            email: "admin@cave".into(),
            roles: vec!["platform_admin".into()],
            exp: 9_999_999_999,
        };
        let mut req = Request::builder()
            .uri("/admin/_audit?tenant_id=acme")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(claims);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        for axis in [
            "Structural",
            "Upstream Parity",
            "Honest Parity",
            "Behavioral Parity",
            "Accessibility",
        ] {
            assert!(body.contains(axis), "missing axis card: {axis}");
        }
        // Refresh + JSON action links must be reachable.
        assert!(body.contains("/admin/_audit.json?tenant_id=acme"));
        assert!(body.contains("/admin/compliance/refresh?tenant_id=acme"));
    }

    #[tokio::test]
    async fn meta_audit_route_returns_403_for_tenant_admin() {
        let app = router(Arc::new(AdminState::seeded()));
        let claims = cave_auth::jwt_middleware::JwtClaims {
            sub: "tenant-admin".into(),
            email: "ta@cave".into(),
            roles: vec!["tenant_admin".into()],
            exp: 9_999_999_999,
        };
        let mut req = Request::builder()
            .uri("/admin/_audit?tenant_id=acme")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(claims);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn meta_audit_json_route_returns_axes_payload() {
        let app = router(Arc::new(AdminState::seeded()));
        let claims = cave_auth::jwt_middleware::JwtClaims {
            sub: "platform-admin".into(),
            email: "admin@cave".into(),
            roles: vec!["platform_admin".into()],
            exp: 9_999_999_999,
        };
        let mut req = Request::builder()
            .uri("/admin/_audit.json?tenant_id=acme")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(claims);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        let axes = v["axes"].as_array().expect("axes array");
        assert_eq!(axes.len(), 5);
        let names: Vec<&str> = axes.iter().map(|a| a["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"structural"));
        assert!(names.contains(&"accessibility"));
    }

    #[tokio::test]
    async fn etcd_route_renders_only_owner_kv_when_tenant_query_set() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "EtcdKVTab",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/etcd?tenant_id=acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("/cfg/feature_x"));
        assert!(!body.contains("/cfg/feature_y"));
    }

    #[tokio::test]
    async fn tenant_dashboard_path_tenant_must_match_query_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "tenantUrlGuard",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/t/acme/dashboard?tenant_id=evil")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn vault_route_never_exposes_other_tenant_paths() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretsPage.tsx",
            "SecretsPage",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/vault?tenant_id=acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_text(resp).await;
        assert!(body.contains("metadata only"));
        assert!(body.contains("kv/db"));
        assert!(!body.contains("kv/secret"));
    }

    #[tokio::test]
    async fn auth_route_lists_users_for_named_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "listUsers",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/auth?tenant_id=acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("alice"));
        assert!(body.contains("bob"));
        assert!(!body.contains("mallory"));
    }
}
