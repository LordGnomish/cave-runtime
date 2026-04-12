//! CAVE Unified Runtime — entry point.
//!
//! Single binary that hosts all enabled platform modules.
//! Native Okta/Keycloak auth, shared PostgreSQL, eBPF hooks.

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
    let app = Router::new()
        // Core health endpoints
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        // Phase 1 module routers
        .merge(cave_secrets::router(secrets_state))
        .merge(cave_lint::router(lint_state))
        .merge(cave_docs::router())
        .merge(cave_status::router())
        .merge(cave_changelog::router())
        .merge(cave_certs::router())
        // Feature flags (cave-native + Unleash compat)
        .merge(cave_flags::router(flags_state))
        // Container registry (cave-native + Docker V2 compat)
        .merge(cave_registry::router(registry_state))
        // Observability stack (Prometheus / Loki / OTLP compat)
        .merge(cave_metrics::router(metrics_state))
        .merge(cave_logs::router(logs_state))
        .merge(cave_trace::router(trace_state))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()); // TODO: restrict in production

    let port = cli.port.unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

    info!(port = port, "CAVE Runtime listening");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs, flags, registry");
    info!("Observability: metrics (Prometheus compat), logs (Loki compat), trace (OTLP compat)");
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
        }
    }))
}
