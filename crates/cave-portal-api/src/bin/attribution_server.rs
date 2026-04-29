//! Standalone attribution server — for smoke-testing the dashboard
//! endpoints without standing up the full cave-runtime binary.
//!
//! Usage:
//!   cargo run -p cave-portal-api --bin attribution-server [-- <repo-path>] [--port 8081]
//!
//! Env:
//!   CAVE_WORKSPACE_ROOT   default repo path (overridden by positional arg)
//!   CAVE_ATTR_PORT        default port (overridden by --port flag)

use std::path::PathBuf;

use cave_portal_api::routes::activity;
use cave_portal_api::routes::attribution;
use cave_portal_api::routes::dora;
use cave_portal_api::routes::state::PortalDataState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,cave_portal_api=debug")
        .try_init()
        .ok();

    let mut repo: Option<PathBuf> = None;
    let mut port: u16 = std::env::var("CAVE_ATTR_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8081);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--port requires a value"))?
                    .parse()?;
            }
            "-h" | "--help" => {
                println!(
                    "usage: attribution-server [<repo-path>] [--port <n>]\n\
                     env:   CAVE_WORKSPACE_ROOT, CAVE_ATTR_PORT"
                );
                return Ok(());
            }
            other if !other.starts_with('-') && repo.is_none() => {
                repo = Some(PathBuf::from(other));
            }
            other => anyhow::bail!("unknown arg: {other}"),
        }
    }
    let repo = repo
        .or_else(|| std::env::var("CAVE_WORKSPACE_ROOT").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

    let state = PortalDataState::new(&repo);
    let app = axum::Router::new()
        .merge(attribution::router(state.clone()))
        .merge(activity::router(state.clone()))
        .merge(dora::router(state));
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("attribution-server listening on http://{addr}  (repo={})", repo.display());
    eprintln!("  GET /api/portal/attribution/{{summary,timeline,by-module}}");
    eprintln!("  GET /api/portal/activity/heatmap?range=7d|30d|90d");
    eprintln!("  GET /api/portal/dora/{{summary,timeline}}");
    eprintln!("  GET /admin/{{attribution,activity,dora}}");
    axum::serve(listener, app).await?;
    Ok(())
}
