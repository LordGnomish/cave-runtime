//! CAVE Unified Runtime — entry point.
//!
//! Single binary that hosts all enabled platform modules.
//! Native Okta/Keycloak auth, shared PostgreSQL, eBPF hooks.
//!
//! ## Auth wiring
//!
//! All module routers are wrapped with `cave_auth::AuthLayer`.
//! Set `CAVE_AUTH_DISABLED=true` to bypass auth in local development.
//! Health / readiness probes (`/health`, `/ready`) are always unauthenticated.

use axum::Router;
use clap::Parser;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Parser)]
#[command(name = "cave-runtime", version, about = "CAVE Platform Unified Runtime")]
struct Cli {
    /// Config file path
    #[arg(short, long, default_value = "cave-runtime.yaml")]
    config: String,

    /// Listen port (overrides config)
    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cave_runtime=info,tower_http=info".into()),
        )
        .json()
        .init();

    let cli = Cli::parse();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %cli.config,
        "Starting CAVE Unified Runtime"
    );

    let pg_state = Arc::new(cave_pg::PgState::default());
    let deploy_state = Arc::new(cave_deploy::DeployState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let docs_site_state = Arc::new(cave_docs_site::DocsSiteState::default());
    let dns_state = Arc::new(cave_dns::DnsState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let gateway_state = Arc::new(cave_gateway::GatewayState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let infra_state = Arc::new(cave_infra::InfraModuleState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let gitops_config_state = Arc::new(cave_gitops_config::AppState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let tracker_state = Arc::new(cave_tracker::TrackerState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let mesh_state = Arc::new(cave_mesh::MeshState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let metrics_state = Arc::new(cave_metrics::MetricsState::new());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let cost_alloc_state = Arc::new(cave_cost_alloc::CostAllocState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let infra_state = Arc::new(cave_infra::InfraState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let runbook_state = Arc::new(cave_runbook::RunbookState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let streams_state = Arc::new(cave_streams::StreamsState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let compliance_state = Arc::new(cave_compliance::ComplianceState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    let ha_state = Arc::new(cave_ha::HaState::default());

    // ── Protected module router ───────────────────────────────────────────────
    //
    // All module routes are wrapped with AuthLayer.  Every handler can use
    // `cave_auth::AuthCtx` extractor or `require_permission!` macro.
    let protected = Router::new()
    let pg_state = Arc::new(cave_pg::PgState::default());
    let vault_store = Arc::new(std::sync::Mutex::new(cave_vault::VaultStore::default()));
    let pg_state = Arc::new(cave_pg::PgState::default());
    let trace_state = Arc::new(cave_trace::TraceState::default());
    let pg_state = Arc::new(cave_pg::PgState::default());
    // Build shared database pool (all DB-backed modules share one pool).
    // DATABASE_URL env var overrides the config file.
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://localhost/cave".to_string());
    let db_config = cave_core::config::DatabaseConfig {
        url: db_url,
        max_pool_size: Some(20),
    };
    let pool = Arc::new(
        cave_db::CavePool::new(&db_config)
            .expect("Failed to create database connection pool"),
    );

    // Initialize module states
    let secrets_state  = Arc::new(cave_secrets::SecretsState::default());
    let lint_state     = Arc::new(cave_lint::LintState::default());
    let flags_state    = Arc::new(cave_flags::FlagsState { pool: Arc::clone(&pool) });
    let registry_state = Arc::new(cave_registry::State { pool: Arc::clone(&pool) });
    let metrics_state  = Arc::new(cave_metrics::MetricsState { pool: Arc::clone(&pool) });
    let logs_state     = Arc::new(cave_logs::LogsState { pool: Arc::clone(&pool) });
    let trace_state    = Arc::new(cave_trace::TraceState { pool: Arc::clone(&pool) });

    // Build the unified router with all modules
    let pg_state = Arc::new(cave_pg::PgState::default());
    let cache_state = Arc::new(cave_cache::CacheState::new());
    let store_state = Arc::new(cave_store::StoreState::new());
    // Build the unified router with all Phase 1 modules + data services
    let pg_state = Arc::new(cave_pg::PgState::default());
    let logs_state = Arc::new(cave_logs::LogsState::default());
    // Build the unified router with all Phase 1 + Phase 3 modules
    let pg_state = Arc::new(cave_pg::PgState::default());
    let llm_gateway_state = Arc::new(cave_llm_gateway::GatewayState::default());
    // Build the unified router with all Phase 1 + Phase 3 modules
    let app = Router::new()
        // Core health endpoints
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        // Platform modules
        .merge(cave_cluster::router(cluster_state))
    let pg_state = Arc::new(cave_pg::PgState::default());
    let llm_gateway_state = Arc::new(cave_llm_gateway::GatewayState::default());
    // Build the unified router with all Phase 1 + Phase 3 modules
        // Phase 1 module routers
        .merge(cave_secrets::router(secrets_state))
        .merge(cave_lint::router(lint_state))
        .merge(cave_trace::router(trace_state))
        .merge(cave_docs::router())
        .merge(cave_status::router())
        .merge(cave_changelog::router())
        .merge(cave_certs::router())
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        .merge(cave_docs_site::router(docs_site_state))
        .merge(cave_dns::router(dns_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Vault / Secrets Management
        .merge(cave_vault::router(vault_store))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Feature flags (cave-native + Unleash compat)
        .merge(cave_flags::router(flags_state))
        // Container registry (cave-native + Docker V2 compat)
        .merge(cave_registry::router(registry_state))
        // Observability stack (Prometheus / Loki / OTLP compat)
        .merge(cave_metrics::router(metrics_state))
        .merge(cave_logs::router(logs_state))
        .merge(cave_trace::router(trace_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Gateway module
        .merge(cave_gateway::router(gateway_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        .merge(cave_infra::router(infra_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Data services
        .merge(cave_cache::router(cache_state))
        .merge(cave_store::router(store_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Platform API
        .merge(cave_gitops_config::router(gitops_config_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Phase 4 module routers
        .merge(cave_tracker::router(tracker_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Service mesh
        .merge(cave_mesh::router(mesh_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Metrics
        .merge(cave_metrics::router(metrics_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // FinOps
        .merge(cave_cost_alloc::router(cost_alloc_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Phase 3 module routers
        .merge(cave_logs::router(logs_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        .merge(cave_infra::router(infra_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Phase 4: runbook automation
        .merge(cave_runbook::router(runbook_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Phase 3 module routers
        .merge(cave_llm_gateway::router(llm_gateway_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // Phase 4 module routers
        .merge(cave_streams::router(streams_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        .merge(cave_compliance::router(compliance_state))
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // HA & DR
        .merge(cave_ha::router(ha_state))
        // Middleware
        .merge(cave_pg::router(pg_state))
        // SCIM 2.0 provisioning (Okta user lifecycle)
        .merge(cave_auth::okta::scim_router(
            std::sync::Arc::new(cave_auth::TokenStore::new(b"change-me")),
        ))
        // Apply the auth layer to all module routes
        .layer(auth_layer);

    // ── Full app router ───────────────────────────────────────────────────────
    //
    // Health / readiness probes sit outside the auth layer so monitoring
    // systems can reach them without credentials.
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        .merge(protected)
        // Observability / transport middleware (outermost = last applied)
        // HA & DR
        .merge(cave_ha::router(ha_state))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()); // TODO: restrict origins in production

    let port = cli.port.unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

    info!(port = port, "CAVE Runtime listening");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs, pg");
    info!(
        auth_disabled = std::env::var("CAVE_AUTH_DISABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false),
        "Auth layer active"
    );
    info!("Platform modules: cluster");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs");
    info!("Phase 5 modules: docs-site, dns");
    info!("Platform modules: cluster");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs, trace");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs, flags, registry");
    info!("Observability: metrics (Prometheus compat), logs (Loki compat), trace (OTLP compat)");
    info!("Phase 5 modules: docs-site, dns");
    info!("Gateway module: routes, upstreams, rate-limiting, auth, circuit-breaker");
    info!("Phase 5 modules: docs-site, dns");
    info!("Infrastructure module: infra (replaces Terraform + Crossplane)");
    info!("Phase 5 modules: docs-site, dns");
    info!("Data services: cache (Redis replacement), store (MinIO replacement)");
    info!("Phase 5 modules: docs-site, dns");
    info!("Platform API: gitops-config (Promises, Compositions, Environments, Claims)");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 4 modules: tracker");
    info!("Phase 5 modules: docs-site, dns");
    info!("Metrics: Prometheus/Thanos replacement active");
    info!("Phase 5 modules: docs-site, dns");
    info!("FinOps modules: cost-alloc (showback/chargeback, replaces Kubecost/CloudHealth)");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 3 modules: logs");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 5 modules: infra (LLM+MCP IaC)");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 4 modules: runbook");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 3 modules: llm-gateway");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 4 modules: streams");
    info!("Phase 5 modules: docs-site, dns");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs, compliance");
    info!("Phase 5 modules: docs-site, dns");
    info!("HA/DR: Raft consensus, failover, cross-site replication enabled");
    info!(
        "Upstream tracking: {} projects",
        cave_upstream::TRACKED_PROJECTS.len()
    );

    axum::serve(listener, app).await?;

    Ok(())
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
    axum::Json(serde_json::json!({
        "ready": true,
        "modules": {
            "secrets": true,
            "lint": true,
            "docs": true,
            "status": true,
            "changelog": true,
            "certs": true,
            "pg": true,
            "docs-site": true,
            "dns": true,
            "pg": true,
            "gateway": true,
            "pg": true,
            "infra": true,
            "pg": true,
            "cache": true,
            "store": true,
            "pg": true,
            "gitops-config": true,
            "pg": true,
            "tracker": true,
            "pg": true,
            "metrics": true,
            "pg": true,
            "cost-alloc": true,
            "pg": true,
            "logs": true,
            "pg": true,
            "llm-gateway": true,
            "pg": true,
            "streams": true,
            "pg": true,
            "compliance": true,
            "pg": true,
            "ha": true,
        }
    }))
}
