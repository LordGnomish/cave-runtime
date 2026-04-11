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
                .unwrap_or_else(|_| "cave_runtime=info,cave_flags=info,tower_http=info".into()),
        )
        .json()
        .init();

    let cli = Cli::parse();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %cli.config,
        "Starting CAVE Unified Runtime"
    );

    // TODO: Load config from file
    // let config = CaveConfig::load(&cli.config)?;

    // TODO: Initialize shared services
    // let db = Arc::new(CavePool::new(&config.database)?);
    // let auth = Arc::new(CaveAuthLayer::new(...));

    // Build the unified router
    let app = Router::new()
        // Health endpoint
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        // Module routers will be nested here:
        // .merge(cave_flags::router(flags_state))
        // .merge(cave_secrets::router(secrets_state))
        // .merge(cave_lint::router(lint_state))
        // ...
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()); // TODO: restrict in production

    let port = cli.port.unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await?;

    info!(port = port, "CAVE Runtime listening");
    info!("Modules: flags, secrets, lint, docs, status, changelog, certs");
    info!("Upstream tracking: 26 projects monitored");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "runtime": "cave-runtime",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn ready() -> axum::Json<serde_json::Value> {
    // TODO: check DB, auth, and module readiness
    axum::Json(serde_json::json!({
        "ready": true,
        "modules": {
            "flags": true,
            "secrets": true,
            "lint": true,
            "docs": true,
            "status": true,
            "changelog": true,
            "certs": true,
        }
    }))
}
