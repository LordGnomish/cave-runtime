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

use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::Html,
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub use permission::{AuthError, Permission, RequestCtx};
pub use state::AdminState;
pub use types::{Cite, TenantId, UPSTREAM_VERSION};

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
    ];
    RequestCtx::developer(&q.tenant_id, &perms)
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
    keda::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn scheduler_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    scheduler::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn controller_manager_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    controller_manager::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn kubelet_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    kubelet::render(&state, &ctx).map(Html).map_err(err_to_response)
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

async fn net_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    net::render(&state, &ctx).map(Html).map_err(err_to_response)
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

async fn compliance_handler(
    Query(q): Query<ComplianceQuery>,
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
    let ctx = extract_ctx_from_query(AdminQuery {
        tenant_id: q.tenant_id,
    });
    let snap = compliance::cached_snapshot_or_refresh();
    compliance::render_with_view(&snap, &ctx, view)
        .map(Html)
        .map_err(err_to_response)
}

async fn compliance_detail_handler(
    Query(q): Query<AdminQuery>,
    axum::extract::Path(crate_name): axum::extract::Path<String>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let root = compliance::workspace_root();
    let detail = compliance::build_crate_detail(&root, &crate_name)
        .map_err(|e| err_to_response(e))?;
    compliance::render_detail(&detail, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn compliance_refresh_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    compliance::handle_refresh(&ctx).map(Html).map_err(err_to_response)
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
async fn upstream_handler(AxumState(s): AxumState<Arc<AdminState>>, Query(q): Query<AdminQuery>) -> Result<Html<String>, (StatusCode, Html<String>)> { let ctx = extract_ctx_from_query(q); upstream::render(&s, &ctx).map(Html).map_err(err_to_response) }
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
        .route("/admin/scheduler", get(scheduler_handler))
        .route("/admin/controller-manager", get(controller_manager_handler))
        .route("/admin/kubelet", get(kubelet_handler))
        .route("/admin/cloud-controller", get(cloud_controller_handler))
        .route("/admin/kamaji", get(kamaji_handler))
        .route("/admin/net", get(net_handler))
        .route("/admin/rdbms", get(rdbms_handler))
        .route("/admin/docdb", get(docdb_handler))
        .route("/admin/cache", get(cache_handler))
        .route("/admin/rdbms-operator", get(rdbms_operator_handler))
        .route("/admin/lakehouse", get(lakehouse_handler))
        .route("/admin/streams", get(streams_handler))
        .route("/admin/compliance", get(compliance_handler))
        .route("/admin/compliance/refresh", get(compliance_refresh_handler))
        .route("/admin/compliance/:crate_name", get(compliance_detail_handler))
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
        .route("/admin/contributions", get(contributions_overview_handler))
        .route("/admin/contributions/timeline", get(contributions_timeline_handler))
        .route("/admin/contributions/leaderboard", get(contributions_leaderboard_handler))
        .route("/admin/contributions/{worker_id}", get(contributions_worker_handler))
        .route("/t/{tenant}/dashboard", get(tenant_dashboard_handler))
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
