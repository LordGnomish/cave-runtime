// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Unified Runtime — entry point.
//!
//! Single binary that hosts all enabled platform modules.
//! Native Okta/Keycloak auth, shared PostgreSQL, eBPF hooks.

use axum::{
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use clap::{Parser, Subcommand};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

mod cluster;
mod cluster_runtime;
mod portal;
mod raft_apply;
mod raft_command;
mod raft_core;
mod raft_driver;
mod raft_transport;

static PORTAL_HTML: &str = include_str!("portal_index.html");

#[derive(Parser)]
#[command(name = "cave-runtime", version, about = "CAVE Platform Unified Runtime")]
struct Cli {
    /// Legacy: path to runtime config. Used when no subcommand is given
    /// (treated as implicit `serve --config <path>`).
    #[arg(short, long, default_value = "cave-runtime.yaml", global = true)]
    config: String,
    /// Legacy: override listen port for implicit `serve`.
    #[arg(short, long, global = true)]
    port: Option<u16>,
    /// Cluster data dir. If `<data_dir>/cluster.json` exists, `serve` starts
    /// dedicated TLS listeners for cave-etcd (2379) and cave-apiserver (6443).
    /// Falls back to `$CAVE_DATA_DIR` or `$HOME/.cave/` when omitted.
    #[arg(long, global = true)]
    data_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the unified runtime (default if no subcommand is given).
    Serve,
    /// Manage cluster lifecycle: init, join, status, destroy.
    Cluster {
        #[command(subcommand)]
        cmd: cluster::ClusterCmd,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cave_runtime=info,tower_http=info".into()),
        )
        .json()
        .init();

    let cli = Cli::parse();

    // Dispatch non-serve subcommands and return early.
    match &cli.command {
        Some(Command::Cluster { cmd }) => {
            return cluster::dispatch(cmd.clone()).await;
        }
        Some(Command::Serve) | None => { /* fall through to serve */ }
    }

    info!(version = env!("CARGO_PKG_VERSION"), config = %cli.config, "Starting CAVE Unified Runtime");

    // Phase 1 states
    let net_state = cave_net::new_state();
    let kubelet_state = cave_kubelet::new_state();
    let scheduler_state = cave_scheduler::new_state();
    let apiserver_state = cave_apiserver::new_state();
    let etcd_state = cave_etcd::new_state();
    let cri_state = cave_cri::new_state();
    let secrets_state = Arc::new(cave_secrets::SecretsState::default());
    let lint_state = Arc::new(cave_lint::LintState::default());
    let flags_state = Arc::new(cave_flags::FlagsState::default());

    // Phase 2 states
    let vulns_state = Arc::new(cave_vulns::State::default());
    let sbom_state = Arc::new(cave_sbom::State::default());
    let uptime_state = Arc::new(cave_uptime::State::default());
    let cost_state = Arc::new(cave_cost::CostState::default());
    let sign_state = Arc::new(cave_sign::State::default());
    let forensics_state = Arc::new(cave_forensics::State::default());

    // Phase 3 states
    let devlake_state = Arc::new(cave_devlake::State::default());
    let ai_obs_state = Arc::new(cave_ai_obs::State::default());
    let pii_state = Arc::new(cave_pii::State::default());
    let incidents_state = Arc::new(cave_incidents::State::default());
    let chat_state = Arc::new(cave_chat::State::default());
    let slo_state = Arc::new(cave_slo::State::default());
    let alerts_state = Arc::new(cave_alerts::State::default());
    let profiler_state = Arc::new(cave_profiler::State::default());

    // Phase 4 states
    // (cave_registry::RegistryState lives inside cave_artifacts::harbor as of the
    //  multi-upstream consolidation; mounted via cave_artifacts::router below.)
    let workflows_state = Arc::new(cave_workflows::State::default());
    let scan_state = Arc::new(cave_scan::State::default());
    let portal_state = Arc::new(cave_portal::PortalState::default());
    // AdminState backs the per-module `/admin/*` views. Probe the data
    // dir for a kubeconfig; if found, install an `ApiserverClient` so
    // the admin views materialise live data from the apiserver instead
    // of seeded fixtures.
    let admin_state = Arc::new(cave_portal::admin::state::AdminState::seeded());
    {
        use cave_portal::admin::runtime_client::{probe_data_dir_for_runtime, WireOutcome};
        let outcome = probe_data_dir_for_runtime(&admin_state, cli.data_dir.as_deref());
        match outcome {
            WireOutcome::Wired => {
                info!(data_dir = ?cli.data_dir, "portal admin → ApiserverClient (real-runtime mode)");
            }
            WireOutcome::NoDataDir => {
                info!("portal admin → seeded fixtures (no data dir / no kubeconfig)");
            }
            WireOutcome::KubeconfigBroken => {
                tracing::warn!("portal admin → seeded fixtures (kubeconfig present but unparseable)");
            }
        }
    }
    let scaffold_state = Arc::new(cave_scaffold::State::default());
    let chaos_state = Arc::new(cave_chaos::State::default());
    let policy_state = Arc::new(cave_policy::State::default());
    let dast_state = Arc::new(cave_dast::State::default());
    let backup_state = Arc::new(cave_backup::BackupState::default());
    let pam_state = Arc::new(cave_pam::State::default());

    // LLM Gateway
    let llm_gateway_state = Arc::new(cave_llm_gateway::GatewayState::default());

    // Infrastructure & Networking
    let api_gateway_state = Arc::new(cave_gateway::GatewayState::default());
    let dns_zones = Arc::new(cave_dns::zone::ZoneManager::default());
    let mesh_state = Arc::new(cave_mesh::MeshState::default());
    let cluster_state = Arc::new(cave_cluster::ClusterState::default());
    let infra_state = Arc::new(cave_infra::InfraState::default());

    // Data & Storage
    let pg_state = Arc::new(cave_rdbms_operator::PgState::default());
    let store_state = cave_store::StoreState::in_memory();
    let streams_state = Arc::new(cave_streams::StreamsState::default());

    // Observability
    let metrics_state = cave_metrics::MetricsState::new();
    let logs_state = cave_logs::default_state();
    let trace_state = Arc::new(cave_trace::TraceState::new(&cave_trace::TraceConfig::default()));

    // Security & Admission
    let admission_state = Arc::new(cave_admission::AdmissionState::default());
    let security_state = Arc::new(cave_security::SecurityState::default());
    let vault_state = cave_vault::VaultState::new();

    // Developer Experience
    let dashboard_state = Arc::new(cave_dashboard::DashboardState::new());
    let docs_site_state = Arc::new(cave_docs_site::DocsSiteState::default());
    let deploy_state = Arc::new(cave_deploy::DeployState::default());
    let pipelines_state = Arc::new(cave_pipelines::State::default());
    let rollouts_state = Arc::new(cave_rollouts::RolloutsState::default());

    // Assets
    let artifacts_state = Arc::new(cave_artifacts::ArtifactsState::default());

    // Governance
    let tracker_state = Arc::new(cave_tracker::TrackerState::default());
    let runbook_state = Arc::new(cave_runbook::RunbookState::default());
    let gitops_state = Arc::new(cave_gitops_config::routes::GitOpsAppState::default());
    let compliance_state = Arc::new(cave_compliance::ComplianceState::default());
    let cost_alloc_state = Arc::new(cave_cost_alloc::CostAllocState::default());

    // New crates (this session)
    let oncall_state = cave_oncall::new_state();
    let container_scan_state = cave_container_scan::new_state();
    let erp_state = cave_erp::new_state();
    let crm_state = cave_crm::new_state();
    let docdb_state = cave_docdb::new_state();
    let rdbms_state = cave_rdbms::new_state();
    let kamaji_state = Arc::new(cave_kamaji::KamajiState::default());

    // Start background tasks
    metrics_state.start_background_tasks();

    // Populate parity cache at startup by discovering every crate that ships a
    // `parity.manifest.toml`. The workspace root is taken from
    // `CAVE_WORKSPACE_ROOT` (defaults to the current working directory).
    {
        let workspace_root = std::env::var("CAVE_WORKSPACE_ROOT").unwrap_or_else(|_| ".".into());
        let mut cache = portal_state.parity_cache.write().await;
        for d in cave_kernel::parity::discover_workspace(&workspace_root) {
            cache.insert(d.report.module.clone(), d.report);
        }
        info!(modules = cache.len(), "parity cache populated from manifests");
    }

    let app = Router::new()
        // Portal UI
        .route("/", get(portal))
        // Core health endpoints
        .route("/health", get(health))
        .route("/ready", get(ready))
        // API
        .route("/api/modules", get(api_modules))
        .route("/api/health", get(api_health))
        // cave-cache is a standalone RESP server — surface a small admin health here.
        .route("/api/cache/health", get(api_cache_health))
        // controller-manager + cloud-controller-manager admin (in-tree library crates,
        // surfaced as inline endpoints — no separate axum router needed).
        .route("/api/portal/controller-manager/health", get(api_controller_manager_health))
        .route("/api/controller-manager/leader", get(api_controller_manager_leader))
        .route("/api/controller-manager/controllers", get(api_controller_manager_controllers))
        .route("/api/controller-manager/status", get(api_controller_manager_status))
        .route("/api/controller-manager/parity", get(api_controller_manager_parity))
        .route("/api/portal/cloud-controller-manager/health", get(api_cloud_controller_manager_health))
        .route("/api/cloud-controller-manager/cloud-controllers", get(api_cloud_controller_manager_controllers))
        .route("/api/cloud-controller-manager/status", get(api_cloud_controller_manager_status))
        .route("/api/cloud-controller-manager/parity", get(api_cloud_controller_manager_parity))
        // Phase 1 module routers
        .merge(cave_net::router(net_state))
        .merge(cave_kubelet::router(kubelet_state))
        .merge(cave_scheduler::router(scheduler_state))
        .merge(cave_apiserver::router(apiserver_state))
        .merge(cave_etcd::router(etcd_state))
        .merge(cave_cri::router(cri_state))
        .merge(cave_secrets::router(secrets_state))
        .merge(cave_lint::router(lint_state))
        .merge(cave_flags::router(flags_state))
        .merge(cave_docs::router())
        .merge(cave_status::router())
        .merge(cave_changelog::router())
        .merge(cave_certs::router())
        // Phase 2
        .merge(cave_vulns::router(vulns_state))
        .merge(cave_sbom::router(sbom_state))
        .merge(cave_uptime::router(uptime_state))
        .merge(cave_cost::router(cost_state))
        .merge(cave_sign::router(sign_state))
        .merge(cave_forensics::router(forensics_state))
        // Phase 3
        .merge(cave_devlake::router(devlake_state))
        .merge(cave_ai_obs::router(ai_obs_state))
        .merge(cave_pii::router(pii_state))
        .merge(cave_incidents::router(incidents_state))
        .merge(cave_chat::router(chat_state))
        .merge(cave_slo::router(slo_state))
        .merge(cave_alerts::router(alerts_state))
        .merge(cave_profiler::router(profiler_state))
        // Phase 4
        // cave_registry routes are now served by cave_artifacts::router (harbor sub-module).
        .merge(cave_workflows::router(workflows_state))
        .merge(cave_scan::router(scan_state))
        .merge(cave_portal::router(portal_state))
        // Per-module /admin/* views (compliance dashboard, keda, vault,
        // grafana, ...). `admin_state` was built at line ~120 above
        // (with the optional RaftBridge-backed runtime client wired
        // via probe_data_dir_for_runtime). Mount the admin router
        // here so the HTTPS surface actually serves what the
        // dashboard tests verify.
        .merge(cave_portal::admin::router(admin_state.clone()))
        .merge(cave_scaffold::router(scaffold_state))
        .merge(cave_chaos::router(chaos_state))
        .merge(cave_policy::router(policy_state))
        .merge(cave_dast::router(dast_state))
        .merge(cave_backup::router(backup_state))
        .merge(cave_pam::router(pam_state))
        // LLM Gateway
        .merge(cave_llm_gateway::router(llm_gateway_state))
        // Infrastructure & Networking
        .merge(cave_gateway::router(api_gateway_state))
        .merge(cave_dns::router(dns_zones))
        .merge(cave_mesh::router(mesh_state))
        .merge(cave_cluster::router(cluster_state))
        .merge(cave_infra::router(infra_state))
        // Data & Storage
        .merge(cave_rdbms_operator::router(pg_state))
        .merge(cave_store::router(store_state))
        .merge(cave_streams::router(streams_state))
        // Observability
        .merge(cave_metrics::router(metrics_state.clone()))
        .merge(cave_logs::router(logs_state))
        .merge(cave_trace::router(trace_state))
        // Security & Admission
        .merge(cave_admission::router(admission_state))
        .merge(cave_security::router(security_state))
        .merge(cave_vault::router(vault_state))
        // Developer Experience
        .merge(cave_dashboard::router(dashboard_state))
        .merge(cave_docs_site::router(docs_site_state))
        .merge(cave_deploy::router(deploy_state))
        .merge(cave_pipelines::router(pipelines_state))
        .merge(cave_rollouts::router(rollouts_state))
        // Assets
        .merge(cave_artifacts::router(artifacts_state))
        // Governance
        .merge(cave_tracker::router(tracker_state))
        .merge(cave_runbook::router(runbook_state))
        .merge(cave_gitops_config::router(gitops_state))
        .merge(cave_compliance::router(compliance_state))
        .merge(cave_cost_alloc::router(cost_alloc_state))
        // SCIM 2.0 provisioning
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(
                std::env::var("CAVE_JWT_SECRET")
                    .expect("CAVE_JWT_SECRET must be set (use any string for dev)")
                    .as_bytes(),
            )),
        ))
        // New crates (this session)
        .merge(cave_oncall::router(oncall_state))
        .merge(cave_container_scan::router(container_scan_state))
        .merge(cave_erp::router(erp_state))
        .merge(cave_crm::router(crm_state))
        .merge(cave_docdb::router(docdb_state.clone()))
        .merge(cave_rdbms::router(rdbms_state.clone()))
        .merge(cave_kamaji::router(kamaji_state))
        // Auth endpoints
        .merge(cave_auth::auth_routes::router())
        // Portal-facing handlers: persona auth, upstream tracker, ADR browser, attribution
        .merge(portal::router())
        // JWT auth middleware
        .layer(axum::middleware::from_fn(|mut req: axum::extract::Request, next: axum::middleware::Next| async move {
            let state = req.extensions().get::<Arc<cave_auth::jwt_middleware::AuthState>>().cloned();
            match state {
                Some(s) => cave_auth::jwt_middleware::auth_middleware_inner(s, req, next).await,
                None => next.run(req).await,
            }
        }))
        .layer(axum::Extension(Arc::new(cave_auth::jwt_middleware::AuthState {
            jwt_secret: std::env::var("CAVE_JWT_SECRET")
                .expect("CAVE_JWT_SECRET must be set (use any string for dev)"),
            bypass_paths: vec![
                "_exact:/".into(),
                "/health".into(), "/ready".into(),
                "/api/modules".into(), "/api/health".into(),
                "/portal/".into(), "/api/portal/".into(), "/api/auth/".into(),
                // Portal sign-in surface — must be reachable without a session.
                "/login".into(),
                "/v2/".into(),
                "/loki/".into(), "/tempo/".into(),
                "/api/registry/".into(),
                // Per-module admin views are mounted via
                // `cave_portal::admin::router`. Authorisation is
                // enforced inside each handler via
                // `RequestCtx::authorise(Permission::...)` against the
                // dev-token granted in `extract_ctx_from_query`. The
                // JWT middleware shouldn't double-gate — that would
                // make the dashboard unreachable without an
                // externally-issued session, which is the wrong UX
                // for the development serve.
                "/admin/".into(),
                "/api/compliance/".into(),
                // 2026-05-13 realtime + power-user batch: SSE event
                // stream + bulk-op submit endpoints. The handlers
                // re-check Permission inside the request context
                // (extract_ctx_from_query grants a dev-token), so
                // the JWT layer doesn't double-gate.
                "/api/events/".into(),
                "/api/bulk/".into(),
            ],
        })))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive());

    // Production-mode cluster runtime: if cluster.json exists, spawn dedicated
    // TLS listeners for cave-etcd (2379) and cave-apiserver (6443).
    let cluster_handles =
        match cluster_runtime::ClusterRuntime::load(cli.data_dir.as_deref()).await {
            Ok(Some(rt)) => {
                info!(
                    cluster = %rt.manifest.cluster_name,
                    data_dir = %rt.data_dir.display(),
                    "production-mode cluster detected — starting dedicated TLS listeners"
                );
                let rt_for_shutdown = rt.clone();
                tokio::spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        info!("Ctrl-C received, persisting etcd snapshot");
                        let _ = rt_for_shutdown.shutdown_persist().await;
                    }
                });
                rt.spawn_listeners().await.ok()
            }
            Ok(None) => {
                info!("no cluster.json found — development mode (unified listener only)");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load cluster.json — falling back to development mode");
                None
            }
        };
    let _ = cluster_handles; // handles run for the lifetime of the process

    let port = cli.port.unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!(port = port, "CAVE Runtime listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn portal() -> impl IntoResponse {
    Html(PORTAL_HTML)
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "runtime": "cave-runtime",
        "version": env!("CARGO_PKG_VERSION"),
        "upstream_tracked": cave_upstream::TRACKED_PROJECTS.len(),
    }))
}

async fn ready() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "ready": true }))
}

async fn api_modules() -> axum::Json<serde_json::Value> {
    let phase1 = ["secrets", "lint", "docs", "status", "changelog", "certs", "portal"];
    let modules = cave_upstream::TRACKED_PROJECTS
        .iter()
        .map(|p| {
            let live = phase1.contains(&p.cave_module.trim_start_matches("cave-"));
            serde_json::json!({
                "id": p.cave_module.trim_start_matches("cave-"),
                "crate": p.cave_module,
                "upstream": p.name,
                "github": p.github_repo,
                "status": if live { "healthy" } else { "pending" },
                "track_features": p.track_features,
                "check_frequency": p.check_frequency,
            })
        })
        .collect::<Vec<_>>();

    axum::Json(serde_json::json!({
        "total": modules.len(),
        "modules": modules,
    }))
}

async fn api_controller_manager_health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "module": "cave-controller-manager",
        "upstream_version": cave_controller_manager::UPSTREAM_VERSION,
        "upstream_pkg": cave_controller_manager::UPSTREAM_PKG,
        "controllers_active": cave_controller_manager::CONTROLLERS.len(),
    }))
}

async fn api_controller_manager_leader() -> axum::Json<serde_json::Value> {
    axum::Json(cave_controller_manager::leader_state(
        std::env::var("CAVE_POD_NAME").as_deref().unwrap_or("manager-0"),
    ))
}

async fn api_controller_manager_controllers() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "total": cave_controller_manager::CONTROLLERS.len(),
        "controllers": cave_controller_manager::CONTROLLERS,
    }))
}

async fn api_controller_manager_status() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "controllers_active": cave_controller_manager::CONTROLLERS.len(),
        "upstream_version": cave_controller_manager::UPSTREAM_VERSION,
    }))
}

async fn api_controller_manager_parity() -> axum::Json<serde_json::Value> {
    match cave_controller_manager::calculate_parity() {
        Ok(r) => axum::Json(serde_json::to_value(r).unwrap_or(serde_json::Value::Null)),
        Err(e) => axum::Json(serde_json::json!({ "error": e })),
    }
}

async fn api_cloud_controller_manager_health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "module": "cave-cloud-controller-manager",
        "upstream_version": cave_cloud_controller_manager::UPSTREAM_VERSION,
        "controllers_active": cave_cloud_controller_manager::CLOUD_CONTROLLERS.len(),
        "providers": cave_cloud_controller_manager::PROVIDERS,
    }))
}

async fn api_cloud_controller_manager_controllers() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "total": cave_cloud_controller_manager::CLOUD_CONTROLLERS.len(),
        "controllers": cave_cloud_controller_manager::CLOUD_CONTROLLERS,
        "providers": cave_cloud_controller_manager::PROVIDERS,
    }))
}

async fn api_cloud_controller_manager_status() -> axum::Json<serde_json::Value> {
    axum::Json(cave_cloud_controller_manager::provider_snapshot())
}

async fn api_cloud_controller_manager_parity() -> axum::Json<serde_json::Value> {
    match cave_cloud_controller_manager::calculate_parity() {
        Ok(r) => axum::Json(serde_json::to_value(r).unwrap_or(serde_json::Value::Null)),
        Err(e) => axum::Json(serde_json::json!({ "error": e })),
    }
}

async fn api_cache_health() -> axum::Json<serde_json::Value> {
    // cave-cache runs out-of-process as a RESP3 server.
    // This admin endpoint reports its in-tree presence and default port.
    let cfg = cave_cache::Config::default();
    axum::Json(serde_json::json!({
        "status": "healthy",
        "module": "cave-cache",
        "protocol": "RESP3 / RESP2",
        "bind": cfg.bind,
        "default_port": cfg.port,
        "default_databases": cfg.databases,
    }))
}

async fn api_health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "runtime": {
            "status": "healthy",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "modules": {
            "secrets": { "status": "healthy", "endpoint": "/api/secrets/health" },
            "lint":    { "status": "healthy", "endpoint": "/api/lint/health" },
            "docs":    { "status": "healthy", "endpoint": "/api/docs/health" },
            "status":  { "status": "healthy", "endpoint": "/api/status/health" },
            "changelog":{ "status": "healthy", "endpoint": "/api/changelog/health" },
            "certs":   { "status": "healthy", "endpoint": "/api/certs/health" },
            "portal":  { "status": "healthy", "endpoint": "/api/portal/health" },
            "controller-manager":      { "status": "healthy", "endpoint": "/api/portal/controller-manager/health" },
            "cloud-controller-manager":{ "status": "healthy", "endpoint": "/api/portal/cloud-controller-manager/health" },
            "pg":      { "status": "healthy", "endpoint": "/api/pg/health" },
            "docdb":   { "status": "healthy", "endpoint": "/api/docdb/health" },
            "cache":   { "status": "healthy", "endpoint": "/api/cache/health" },
        },
        "upstream_tracked": cave_upstream::TRACKED_PROJECTS.len(),
    }))
}
