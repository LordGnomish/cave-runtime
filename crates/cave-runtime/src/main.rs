//! CAVE Unified Runtime — entry point.
//!
//! Single binary that hosts all enabled platform modules.
//! Native Okta/Keycloak auth, shared PostgreSQL, eBPF hooks.

use axum::{
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use clap::Parser;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

static PORTAL_HTML: &str = include_str!("portal_index.html");

#[derive(Parser)]
#[command(name = "cave-runtime", version, about = "CAVE Platform Unified Runtime")]
struct Cli {
    #[arg(short, long, default_value = "cave-runtime.yaml")]
    config: String,
    #[arg(short, long)]
    port: Option<u16>,
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
    let registry_state = Arc::new(cave_registry::RegistryState::default());
    let workflows_state = Arc::new(cave_workflows::State::default());
    let scan_state = Arc::new(cave_scan::State::default());
    let portal_state = Arc::new(cave_portal::PortalState::default());
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
    let pg_state = Arc::new(cave_pg::PgState::default());
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
    let docdb_state = cave_docdb::new_state();
    let rdbms_state = cave_rdbms::new_state();
    let kamaji_state = Arc::new(cave_kamaji::KamajiState::default());

    // Start background tasks
    metrics_state.start_background_tasks();

    // Populate parity cache at startup
    {
        let mut cache = portal_state.parity_cache.write().await;
        if let Ok(r) = cave_etcd::calculate_parity() {
            cache.insert("etcd".to_string(), r);
        }
        if let Ok(r) = cave_cri::calculate_parity() {
            cache.insert("cri".to_string(), r);
        }
        if let Ok(r) = cave_apiserver::calculate_parity() {
            cache.insert("apiserver".to_string(), r);
        }
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
        .merge(cave_registry::router(registry_state))
        .merge(cave_workflows::router(workflows_state))
        .merge(cave_scan::router(scan_state))
        .merge(cave_portal::router(portal_state))
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
        .merge(cave_pg::router(pg_state))
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
        .merge(cave_docdb::router(docdb_state.clone()))
        .merge(cave_rdbms::router(rdbms_state.clone()))
        .merge(cave_kamaji::router(kamaji_state))
        // Auth endpoints
        .merge(cave_auth::auth_routes::router())
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
                "/api/upstream/".into(), "/v2/".into(),
                "/api/v1/attribution".into(),
                "/loki/".into(), "/tempo/".into(),
                "/api/registry/".into(),
            ],
        })))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive());

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
        },
        "upstream_tracked": cave_upstream::TRACKED_PROJECTS.len(),
    }))
}
