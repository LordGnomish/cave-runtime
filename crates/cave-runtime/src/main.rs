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

    // ── Auth layer ────────────────────────────────────────────────────────────
    //
    // Reads OKTA_DOMAIN, OKTA_AUTH_SERVER_ID, OKTA_AUDIENCE from the environment.
    // Falls back to dev-bypass when CAVE_AUTH_DISABLED=true.
    let auth_layer = cave_auth::auth_layer_from_env();

    // ── Module states ─────────────────────────────────────────────────────────
    let secrets_state = Arc::new(cave_secrets::SecretsState::default());
    let lint_state = Arc::new(cave_lint::LintState::default());
<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
    let pg_state = Arc::new(cave_pg::PgState::default());
=======
    let deploy_state = Arc::new(cave_deploy::DeployState::default());
>>>>>>> claude/cranky-hellman
=======
    let docs_site_state = Arc::new(cave_docs_site::DocsSiteState::default());
    let dns_state = Arc::new(cave_dns::DnsState::default());
>>>>>>> claude/cranky-khorana

    // ── Protected module router ───────────────────────────────────────────────
    //
    // All module routes are wrapped with AuthLayer.  Every handler can use
    // `cave_auth::AuthCtx` extractor or `require_permission!` macro.
    let protected = Router::new()
=======
    let cluster_state = Arc::new(cave_cluster::ClusterState::default());
=======
    let vault_store = Arc::new(std::sync::Mutex::new(cave_vault::VaultStore::default()));
>>>>>>> claude/ecstatic-chebyshev

    // Build the unified router with all Phase 1 modules
    let app = Router::new()
        // Core health endpoints
        .route("/health", axum::routing::get(health))
        .route("/ready", axum::routing::get(ready))
        // Platform modules
        .merge(cave_cluster::router(cluster_state))
>>>>>>> claude/cranky-wozniak
        // Phase 1 module routers
        .merge(cave_secrets::router(secrets_state))
        .merge(cave_lint::router(lint_state))
        .merge(cave_docs::router())
        .merge(cave_status::router())
        .merge(cave_changelog::router())
        .merge(cave_certs::router())
<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
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
=======
        // GitOps
        .merge(cave_deploy::router(deploy_state))
=======
        .merge(cave_docs_site::router(docs_site_state))
        .merge(cave_dns::router(dns_state))
>>>>>>> claude/cranky-khorana
=======
        // Vault / Secrets Management
        .merge(cave_vault::router(vault_store))
>>>>>>> claude/ecstatic-chebyshev
        // Middleware
>>>>>>> claude/cranky-hellman
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()); // TODO: restrict origins in production

    let port = cli.port.unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

    info!(port = port, "CAVE Runtime listening");
<<<<<<< HEAD
<<<<<<< HEAD
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs, pg");
    info!(
        auth_disabled = std::env::var("CAVE_AUTH_DISABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false),
        "Auth layer active"
    );
=======
=======
    info!("Platform modules: cluster");
>>>>>>> claude/cranky-wozniak
    info!("Phase 1 modules: secrets, lint, docs, status, changelog, certs");
    info!("Phase 5 modules: docs-site, dns");
>>>>>>> claude/cranky-khorana
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
<<<<<<< HEAD
            "pg": true,
=======
            "docs-site": true,
            "dns": true,
>>>>>>> claude/cranky-khorana
        }
    }))
}
