//! CAVE Unified Runtime — entry point.
//!
//! Single binary that hosts all enabled platform modules.
//! Native Okta/Keycloak auth, shared PostgreSQL, eBPF hooks.

use axum::{Router, routing::get};
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

    // Load configuration
    let config: cave_core::config::CaveConfig = {
        let text = std::fs::read_to_string(&cli.config).unwrap_or_else(|_| {
            tracing::warn!(path = %cli.config, "Config file not found, using defaults");
            include_str!("../../../cave-runtime.yaml").to_string()
        });
        serde_yaml::from_str(&text).expect("Invalid config file")
    };

    // Allow DATABASE_URL env var to override config
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| config.database.url.clone());

    let db_config = cave_core::config::DatabaseConfig {
        url: db_url,
        max_pool_size: config.database.max_pool_size,
    };

    let pool = Arc::new(
        cave_db::CavePool::new(&db_config)
            .expect("Failed to create database connection pool"),
    );

    // ── Phase 1: initialize legacy states (no pool required) ──────────────
    let secrets_state = Arc::new(cave_secrets::SecretsState::default());
    let lint_state = Arc::new(cave_lint::LintState::default());

    // ── Phase 1: flags needs a pool ────────────────────────────────────────
    let flags_state = Arc::new(cave_flags::FlagsState { pool: pool.clone() });

    // ── Phase 2–4: pool-backed module states ──────────────────────────────
    let vulns_state     = Arc::new(cave_vulns::State     { pool: pool.clone() });
    let sbom_state      = Arc::new(cave_sbom::State      { pool: pool.clone() });
    let uptime_state    = Arc::new(cave_uptime::State    { pool: pool.clone() });
    let cost_state      = Arc::new(cave_cost::State      { pool: pool.clone() });
    let sign_state      = Arc::new(cave_sign::State      { pool: pool.clone() });
    let forensics_state = Arc::new(cave_forensics::State { pool: pool.clone() });
    let devlake_state   = Arc::new(cave_devlake::State   { pool: pool.clone() });
    let ai_obs_state    = Arc::new(cave_ai_obs::State    { pool: pool.clone() });
    let pii_state       = Arc::new(cave_pii::State       { pool: pool.clone() });
    let incidents_state = Arc::new(cave_incidents::State { pool: pool.clone() });
    let chat_state      = Arc::new(cave_chat::State      { pool: pool.clone() });
    let slo_state       = Arc::new(cave_slo::State       { pool: pool.clone() });
    let alerts_state    = Arc::new(cave_alerts::State    { pool: pool.clone() });
    let profiler_state  = Arc::new(cave_profiler::State  { pool: pool.clone() });
    let registry_state  = Arc::new(cave_registry::State  { pool: pool.clone() });
    let workflows_state = Arc::new(cave_workflows::State { pool: pool.clone() });
    let scan_state      = Arc::new(cave_scan::State      { pool: pool.clone() });
    let portal_state    = Arc::new(cave_portal::State    { pool: pool.clone() });
    let scaffold_state  = Arc::new(cave_scaffold::State  { pool: pool.clone() });
    let chaos_state     = Arc::new(cave_chaos::State     { pool: pool.clone() });
    let policy_state    = Arc::new(cave_policy::State    { pool: pool.clone() });
    let dast_state      = Arc::new(cave_dast::State      { pool: pool.clone() });
    let backup_state    = Arc::new(cave_backup::State    { pool: pool.clone() });
    let pam_state       = Arc::new(cave_pam::State       { pool: pool.clone() });

    // ── Build unified router ───────────────────────────────────────────────
    let app = Router::new()
        // Portal dashboard UI served at "/"
        .route("/", get(portal_index))
        // Core health/ready probes
        .route("/health", get(health))
        .route("/ready", get(ready))
        // Phase 1 — legacy modules (self-routed paths)
        .merge(cave_secrets::router(secrets_state))
        .merge(cave_lint::router(lint_state))
        .merge(cave_docs::router())
        .merge(cave_status::router())
        .merge(cave_changelog::router())
        .merge(cave_certs::router())
        // Phase 1 — flags
        .merge(cave_flags::router(flags_state))
        // Phase 2 — security & reliability
        .merge(cave_vulns::router(vulns_state))
        .merge(cave_sbom::router(sbom_state))
        .merge(cave_uptime::router(uptime_state))
        .merge(cave_cost::router(cost_state))
        .merge(cave_sign::router(sign_state))
        .merge(cave_forensics::router(forensics_state))
        // Phase 3 — platform intelligence
        .merge(cave_devlake::router(devlake_state))
        .merge(cave_ai_obs::router(ai_obs_state))
        .merge(cave_pii::router(pii_state))
        .merge(cave_incidents::router(incidents_state))
        .merge(cave_chat::router(chat_state))
        .merge(cave_slo::router(slo_state))
        .merge(cave_alerts::router(alerts_state))
        .merge(cave_profiler::router(profiler_state))
        // Phase 4 — heavy hitters
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
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()); // TODO: restrict in production

    let port = cli.port.unwrap_or(config.server.port);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

    info!(port = port, "CAVE Runtime listening");
    info!(
        modules = 30,
        upstream_tracked = cave_upstream::TRACKED_PROJECTS.len(),
        "All modules mounted"
    );

    axum::serve(listener, app).await?;

    Ok(())
}

/// Serves the portal dashboard HTML at "/".
async fn portal_index() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("portal_index.html"))
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
            // Phase 1
            "secrets": true,
            "lint": true,
            "docs": true,
            "status": true,
            "changelog": true,
            "certs": true,
            "flags": true,
            // Phase 2
            "vulns": true,
            "sbom": true,
            "uptime": true,
            "cost": true,
            "sign": true,
            "forensics": true,
            // Phase 3
            "devlake": true,
            "ai-obs": true,
            "pii": true,
            "incidents": true,
            "chat": true,
            "slo": true,
            "alerts": true,
            "profiler": true,
            // Phase 4
            "registry": true,
            "workflows": true,
            "scan": true,
            "portal": true,
            "scaffold": true,
            "chaos": true,
            "policy": true,
            "dast": true,
            "backup": true,
            "pam": true,
        }
    }))
}
