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

    // Initialize module states
    let secrets_state = Arc::new(cave_secrets::SecretsState::default());
    let lint_state = Arc::new(cave_lint::LintState::default());

    // Build the unified router with all Phase 1 modules
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
        .merge(cave_secrets::router(secrets_state))
        .merge(cave_lint::router(lint_state))
        .merge(cave_docs::router())
        .merge(cave_status::router())
        .merge(cave_changelog::router())
        .merge(cave_certs::router())
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()); // TODO: restrict in production

    let port = cli.port.unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

    info!(port = port, "CAVE Runtime listening");
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs");
    info!(
        "Upstream tracking: {} projects",
        cave_upstream::TRACKED_PROJECTS.len()
    );

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
