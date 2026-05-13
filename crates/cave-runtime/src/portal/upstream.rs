//! Upstream Tracker page + API.
//!
//! Renders the 65+ tracked upstream OSS projects with the four columns Burak
//! asked for:
//!   * Upstream identity (`owner/repo`)
//!   * Cave crate name + parity %
//!   * ADR(s) that justify the choice (links into `/adr/<id>`)
//!   * Last commit timestamp + sync status
//!
//! Routes
//! ──────
//!   GET  /upstream                       → tracker HTML page (auth required)
//!   GET  /api/upstream/tracker           → JSON tracker rows (live)
//!   GET  /api/upstream/{repo:.*}/details → parity manifest + recent commits
//!
//! `{repo:.*}` is the URL-encoded `owner/repo` from `TrackedProject`. The
//! `:.*` capture lets axum accept the embedded slash.

use std::{
    collections::HashMap,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::Path,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use cave_kernel::parity::{discover::DiscoveredReport, types::ParityReport};
use cave_upstream::{adr_links, projects::TrackedProject, TRACKED_PROJECTS};
use serde::Serialize;
use serde_json::json;
use tokio::sync::RwLock;

use super::workspace_root;

static PAGE_HTML: &str = include_str!("templates/upstream.html");

/// In-memory snapshot of "last touched" data per crate. Computed on first
/// `/api/upstream/tracker` hit and cached for 60 s — every subsequent caller
/// gets the cached map without re-forking `git`.
#[derive(Default, Clone)]
struct LastTouchCache {
    by_crate: HashMap<String, LastTouched>,
    fetched_at: Option<Instant>,
}

#[derive(Default, Clone, Serialize)]
struct LastTouched {
    sha: String,
    iso_time: String,
}

static CACHE: once_cell_lazy::Lazy<Arc<RwLock<LastTouchCache>>> =
    once_cell_lazy::Lazy::new(|| Arc::new(RwLock::new(LastTouchCache::default())));

mod once_cell_lazy {
    // Tiny stand-in so we don't pull in once_cell — `std::sync::OnceLock`
    // would suffice but we want a typed init closure. This is a one-shot
    // lazy that uses `OnceLock` internally.
    use std::sync::OnceLock;
    pub struct Lazy<T> {
        cell: OnceLock<T>,
        init: fn() -> T,
    }
    impl<T> Lazy<T> {
        pub const fn new(init: fn() -> T) -> Self {
            Self {
                cell: OnceLock::new(),
                init,
            }
        }
    }
    impl<T> std::ops::Deref for Lazy<T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.cell.get_or_init(self.init)
        }
    }
}

const CACHE_TTL: Duration = Duration::from_secs(60);

/// Output row for `/api/upstream/tracker`.
#[derive(Debug, Clone, Serialize)]
pub struct TrackerRow {
    pub upstream_name: String,
    pub upstream_repo: String,
    pub cave_crate: String,
    pub category: String,
    pub phase: u8,
    pub adr_refs: Vec<String>,
    /// 0.0 – 1.0; -1.0 if no parity manifest exists for this crate.
    pub parity_overall: f32,
    pub parity_status: String, // "synced" | "behind" | "error" | "pending"
    pub last_commit_sha: Option<String>,
    pub last_commit_at: Option<String>,
    pub manifest_present: bool,
}

/// GET /upstream — tracker HTML page. Auth-gated by the runtime
/// middleware (`/upstream` is NOT in the JWT bypass list, so the
/// middleware redirects to /login on no-cookie). Additionally
/// persona-gated to `platform_admin` here — the upstream parity
/// tracker is cross-tenant control-plane info.
pub async fn page(
    claims: Option<axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Response {
    if !is_platform_admin(claims.as_ref()) {
        return persona_denied_response("/upstream", "platform_admin");
    }
    Html(PAGE_HTML).into_response()
}

/// GET /api/upstream/tracker — JSON rows, gated identically to the
/// HTML page so a tenant admin can't bypass via the API.
pub fn is_platform_admin(
    claims: Option<&axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> bool {
    match claims {
        Some(axum::Extension(c)) => c.roles.iter().any(|r| r == "platform_admin"),
        None => false,
    }
}

pub fn persona_denied_response(path: &str, required: &str) -> Response {
    let body = format!(
        "<html><body><h1>403 Forbidden</h1><p>{path} requires persona <code>{required}</code>. \
         <a href=\"/login\">Sign in</a> with a platform_admin account.</p></body></html>"
    );
    (StatusCode::FORBIDDEN, Html(body)).into_response()
}

/// GET /api/upstream/tracker — live tracker rows. Persona-gated
/// identically to the HTML page (platform_admin only).
pub async fn api_tracker(
    claims: Option<axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Response {
    if !is_platform_admin(claims.as_ref()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "platform_admin role required" })),
        )
            .into_response();
    }
    let parity_index = build_parity_index().await;
    let last_touched = ensure_last_touched().await;

    let rows: Vec<TrackerRow> = TRACKED_PROJECTS
        .iter()
        .map(|p| build_row(p, &parity_index, &last_touched))
        .collect();

    let synced = rows.iter().filter(|r| r.parity_status == "synced").count();
    let behind = rows.iter().filter(|r| r.parity_status == "behind").count();
    let pending = rows.iter().filter(|r| r.parity_status == "pending").count();

    Json(json!({
        "total": rows.len(),
        "summary": {
            "synced": synced,
            "behind": behind,
            "pending": pending,
        },
        "rows": rows,
    }))
    .into_response()
}

/// GET /api/upstream/{owner/repo}/details — manifest TOML + recent commits.
/// Persona-gated to `platform_admin`.
pub async fn api_details(
    Path(repo): Path<String>,
    claims: Option<axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Response {
    if !is_platform_admin(claims.as_ref()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "platform_admin role required" })),
        )
            .into_response();
    }
    let project = match TRACKED_PROJECTS.iter().find(|p| p.github_repo == repo) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Unknown upstream {repo}") })),
            )
                .into_response();
        }
    };

    let crate_dir = workspace_root().join("crates").join(project.cave_module);
    let manifest_path = crate_dir.join("parity.manifest.toml");
    let manifest_toml = std::fs::read_to_string(&manifest_path).ok();
    let recent_commits = git_recent_commits(project.cave_module, 5);
    let parity = build_parity_index()
        .await
        .get(project.cave_module)
        .cloned();

    Json(json!({
        "project": project,
        "adr_refs": adr_links::adrs_for(repo.as_str()),
        "manifest_path": manifest_path,
        "manifest_present": manifest_toml.is_some(),
        "manifest_toml": manifest_toml,
        "parity": parity,
        "recent_commits": recent_commits,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_row(
    project: &TrackedProject,
    parity_by_crate: &HashMap<String, ParityReport>,
    last_touched: &HashMap<String, LastTouched>,
) -> TrackerRow {
    let parity = parity_by_crate.get(project.cave_module);
    let (overall, status) = match parity {
        Some(r) => {
            let s = if r.overall >= 0.7 {
                "synced"
            } else if r.overall >= 0.3 {
                "behind"
            } else {
                "pending"
            };
            (r.overall, s.to_string())
        }
        None => (-1.0, "pending".to_string()),
    };
    let touched = last_touched.get(project.cave_module).cloned();
    TrackerRow {
        upstream_name: project.name.to_string(),
        upstream_repo: project.github_repo.to_string(),
        cave_crate: project.cave_module.to_string(),
        category: project.category.to_string(),
        phase: project.phase,
        adr_refs: adr_links::adrs_for(project.github_repo)
            .iter()
            .map(|s| s.to_string())
            .collect(),
        parity_overall: overall,
        parity_status: status,
        last_commit_sha: touched.as_ref().map(|t| t.sha.clone()),
        last_commit_at: touched.as_ref().map(|t| t.iso_time.clone()),
        manifest_present: parity.is_some(),
    }
}

async fn build_parity_index() -> HashMap<String, ParityReport> {
    let root = workspace_root();
    let reports: Vec<DiscoveredReport> =
        tokio::task::spawn_blocking(move || cave_kernel::parity::discover_workspace(&root))
            .await
            .unwrap_or_default();

    // The manifest's `module.name` is bare (e.g. "net") whereas TrackedProject
    // carries the crate name ("cave-net"). Index by both — first preferring
    // the parent directory of the manifest (which is always the crate dir).
    let mut idx = HashMap::new();
    for d in reports {
        if let Some(crate_dir) = d.manifest_path.parent() {
            if let Some(name) = crate_dir.file_name().and_then(|s| s.to_str()) {
                idx.insert(name.to_string(), d.report.clone());
            }
        }
        // Fallback: the module name itself
        idx.entry(d.report.module.clone()).or_insert(d.report);
    }
    idx
}

/// Returns the cached "last touched per crate" map, refreshing if stale.
async fn ensure_last_touched() -> HashMap<String, LastTouched> {
    {
        let guard = CACHE.read().await;
        if let Some(at) = guard.fetched_at {
            if at.elapsed() < CACHE_TTL {
                return guard.by_crate.clone();
            }
        }
    }
    let computed = tokio::task::spawn_blocking(compute_last_touched)
        .await
        .unwrap_or_default();
    let mut guard = CACHE.write().await;
    guard.by_crate = computed.clone();
    guard.fetched_at = Some(Instant::now());
    computed
}

/// Single bulk `git log` (last 2000 commits touching crates/) → per-crate
/// most-recent (sha, iso) map. Skipping the per-crate fork explosion keeps
/// the tracker render under ~250 ms even on cold cache.
fn compute_last_touched() -> HashMap<String, LastTouched> {
    let root = workspace_root();
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&root).args([
        "log",
        "--max-count=2000",
        "--name-only",
        "--pretty=format:CAVE_C|%H|%cI",
        "--",
        "crates/",
    ]);
    let Ok(out) = cmd.output() else {
        return HashMap::new();
    };
    if !out.status.success() {
        return HashMap::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_log_for_last_touched(&stdout)
}

fn parse_log_for_last_touched(stdout: &str) -> HashMap<String, LastTouched> {
    let mut map: HashMap<String, LastTouched> = HashMap::new();
    let mut current: Option<LastTouched> = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("CAVE_C|") {
            let mut parts = rest.splitn(2, '|');
            let sha = parts.next().unwrap_or("").to_string();
            let iso = parts.next().unwrap_or("").to_string();
            current = Some(LastTouched { sha, iso_time: iso });
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // crates/<crate>/...
        if let Some(rest) = line.strip_prefix("crates/") {
            let crate_name = rest.split('/').next().unwrap_or("").to_string();
            if crate_name.is_empty() {
                continue;
            }
            if let Some(touched) = &current {
                map.entry(crate_name).or_insert_with(|| touched.clone());
            }
        }
    }
    map
}

#[derive(Debug, Clone, Serialize)]
struct CommitSummary {
    sha: String,
    iso_time: String,
    author: String,
    subject: String,
}

fn git_recent_commits(crate_name: &str, n: usize) -> Vec<CommitSummary> {
    let root = workspace_root();
    let path = format!("crates/{crate_name}");
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&root).args([
        "log",
        &format!("--max-count={n}"),
        "--pretty=format:%H|%cI|%an|%s",
        "--",
        &path,
    ]);
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '|');
            Some(CommitSummary {
                sha: parts.next()?.to_string(),
                iso_time: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                subject: parts.next()?.to_string(),
            })
        })
        .collect()
}

pub fn router() -> Router {
    Router::new()
        .route("/upstream", get(page))
        .route("/api/upstream/tracker", get(api_tracker))
        .route("/api/upstream/{*repo}", get(api_details))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use cave_auth::jwt_middleware::JwtClaims;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Test helper: build the upstream router with a platform_admin
    /// `JwtClaims` extension pre-injected. The persona gate is
    /// uniform across page/tracker/details, so every endpoint test
    /// uses this.
    fn router_as_platform() -> Router {
        let claims = JwtClaims {
            sub: "admin@platform.cave".into(),
            email: "admin@platform.cave".into(),
            roles: vec!["platform_admin".into()],
            exp: 4102444800,
        };
        router().layer(axum::Extension(claims))
    }

    #[test]
    fn parse_log_extracts_first_commit_per_crate() {
        let log = "\
CAVE_C|abc123|2026-05-02T12:00:00Z

crates/cave-net/src/foo.rs
crates/cave-pg/src/bar.rs

CAVE_C|def456|2026-05-01T11:00:00Z
crates/cave-net/src/baz.rs
crates/cave-runtime/src/main.rs
";
        let map = parse_log_for_last_touched(log);
        assert_eq!(map.get("cave-net").unwrap().sha, "abc123");
        assert_eq!(map.get("cave-pg").unwrap().sha, "abc123");
        assert_eq!(map.get("cave-runtime").unwrap().sha, "def456");
    }

    #[tokio::test]
    async fn tracker_endpoint_returns_all_projects() {
        let _g = crate::portal::WORKSPACE_ROOT_TEST_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: guarded by WORKSPACE_ROOT_TEST_GUARD.
        unsafe { std::env::set_var("CAVE_WORKSPACE_ROOT", repo_root()); }
        let app = router_as_platform();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/upstream/tracker")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 8 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            v["total"].as_u64().unwrap() as usize,
            TRACKED_PROJECTS.len()
        );
        let rows = v["rows"].as_array().unwrap();
        // At least one row should carry an ADR ref (we hardcoded several).
        assert!(rows.iter().any(|r| !r["adr_refs"].as_array().unwrap().is_empty()));
        // Cilium row must have ADR-004
        let cilium = rows
            .iter()
            .find(|r| r["upstream_repo"] == "cilium/cilium")
            .expect("cilium row");
        let adrs: Vec<&str> = cilium["adr_refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(adrs.contains(&"ADR-004"), "got {adrs:?}");
    }

    #[tokio::test]
    async fn details_endpoint_unknown_repo_404s() {
        let app = router_as_platform();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/upstream/no/such-repo")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn details_endpoint_returns_project_for_known_repo() {
        let _g = crate::portal::WORKSPACE_ROOT_TEST_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: guarded by WORKSPACE_ROOT_TEST_GUARD.
        unsafe { std::env::set_var("CAVE_WORKSPACE_ROOT", repo_root()); }
        let app = router_as_platform();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/upstream/cilium/cilium")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["project"]["github_repo"], "cilium/cilium");
        assert!(v["adr_refs"].as_array().unwrap().iter().any(|a| a == "ADR-004"));
    }

    fn repo_root() -> String {
        // CARGO_MANIFEST_DIR points at crates/cave-runtime — go two levels up.
        let here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        here.parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}
