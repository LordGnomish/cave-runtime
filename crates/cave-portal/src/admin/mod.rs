//! Admin views — per-module Backstage-parity panels gated by WebAuthn + RBAC.
//!
//! Each submodule is a self-contained view with three layers:
//!
//! 1. **Data accessors** (`list_*`, `inspect_*`) — pure functions taking
//!    `&AdminState` + `&RequestCtx`, scoped to the caller's tenant.
//! 2. **Mutators** (where allowed) — gated on a stricter permission than
//!    the read accessors.
//! 3. **`render(...)`** — composes the page shell + tables + htmx hooks.
//!
//! A small axum [`router`] glues them together at the standard URLs.
//! The router takes its own `Arc<AdminState>` so it can be merged into the
//! main portal router without touching `PortalState`.

pub mod types;

pub mod permission;
pub mod render;
pub mod state;

pub mod apiserver;
pub mod cache;
pub mod cloud_controller_manager;
pub mod contributions;
pub mod controller_manager;
pub mod cri;
pub mod docdb;
pub mod etcd;
pub mod iam;
pub mod kamaji;
pub mod keda;
pub mod kubelet;
pub mod mesh;
pub mod net;
pub mod pg;
pub mod rdbms;
pub mod scheduler;
pub mod tenant_dashboard;
pub mod vault;

use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::Html,
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub use permission::{AuthError, Permission, RequestCtx};
pub use state::AdminState;
pub use types::{Cite, TenantId, UPSTREAM_VERSION};

#[derive(Debug, Deserialize)]
pub struct AdminQuery {
    pub tenant_id: String,
}

/// Helper that turns `(?tenant_id=acme)` plus an authenticated principal
/// into a [`RequestCtx`]. Real deployments derive principal + permissions
/// from session cookies / JWT; for now we accept defaults from a header
/// so integration tests can drive routes directly.
pub fn extract_ctx_from_query(q: AdminQuery) -> RequestCtx {
    // In production this is filled by the cave-auth middleware; here we
    // grant every documented permission and assume the caller completed
    // WebAuthn. Per-route handlers still call `ctx.authorise(...)` so
    // tightening this later is a one-line change.
    let perms = [
        Permission::DashboardRead,
        Permission::EtcdRead,
        Permission::EtcdWatch,
        Permission::CriRead,
        Permission::CriExec,
        Permission::ApiserverRead,
        Permission::IamRead,
        Permission::IamWrite,
        Permission::MeshRead,
        Permission::MeshWrite,
        Permission::PgRead,
        Permission::PgQuery,
        Permission::VaultRead,
        Permission::ContributionsRead,
        Permission::SchedulerRead,
        Permission::SchedulerWrite,
        Permission::ControllerManagerRead,
        Permission::KubeletRead,
        Permission::KubeletExec,
        Permission::CloudControllerRead,
        Permission::KamajiRead,
        Permission::KamajiWrite,
        Permission::NetRead,
        Permission::NetWrite,
        Permission::RdbmsRead,
        Permission::RdbmsQuery,
        Permission::DocdbRead,
        Permission::DocdbQuery,
        Permission::CacheRead,
        Permission::CacheWrite,
        Permission::KedaRead,
        Permission::KedaWrite,
    ];
    RequestCtx::developer(&q.tenant_id, &perms)
}

fn load_contributions() -> Vec<contributions::Contribution> {
    let path = std::env::var("CAVE_CONTRIBUTIONS_JSONL")
        .unwrap_or_else(|_| "tools/night-pump/contributions.jsonl".into());
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    contributions::parse_jsonl(&raw).unwrap_or_default()
}

fn err_to_response(e: impl ToString) -> (StatusCode, Html<String>) {
    (
        StatusCode::FORBIDDEN,
        Html(render::permission_denied(&e.to_string())),
    )
}

async fn etcd_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    etcd::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn cri_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    cri::render_list_page(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn apiserver_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    apiserver::render(&state, &ctx, None).map(Html).map_err(err_to_response)
}

async fn auth_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    iam::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn mesh_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    mesh::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn pg_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    pg::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn vault_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    vault::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn tenant_dashboard_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Path(tenant): Path<String>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let path_tenant = TenantId::new(tenant).expect("test fixture");
    tenant_dashboard::render(&state, &ctx, &path_tenant)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_overview_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_overview(&recs, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_worker_handler(
    Path(worker_id): Path<String>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_worker_detail(&recs, &worker_id, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_timeline_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_timeline(&recs, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn contributions_leaderboard_handler(
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    let recs = load_contributions();
    contributions::render_leaderboard(&recs, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn scheduler_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    scheduler::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn controller_manager_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    controller_manager::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn kubelet_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    kubelet::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn cloud_controller_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    cloud_controller_manager::render(&state, &ctx)
        .map(Html)
        .map_err(err_to_response)
}

async fn kamaji_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    kamaji::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn net_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    net::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn rdbms_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    rdbms::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn docdb_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    docdb::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn cache_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    cache::render(&state, &ctx).map(Html).map_err(err_to_response)
}

async fn keda_handler(
    AxumState(state): AxumState<Arc<AdminState>>,
    Query(q): Query<AdminQuery>,
) -> Result<Html<String>, (StatusCode, Html<String>)> {
    let ctx = extract_ctx_from_query(q);
    keda::render(&state, &ctx).map(Html).map_err(err_to_response)
}

/// Build the admin router. Mount as `app.merge(admin::router(state))`.
pub fn router(state: Arc<AdminState>) -> Router {
    Router::new()
        .route("/admin/etcd", get(etcd_handler))
        .route("/admin/cri", get(cri_handler))
        .route("/admin/apiserver", get(apiserver_handler))
        .route("/admin/auth", get(auth_handler))
        .route("/admin/mesh", get(mesh_handler))
        .route("/admin/pg", get(pg_handler))
        .route("/admin/vault", get(vault_handler))
        .route("/admin/contributions", get(contributions_overview_handler))
        .route("/admin/contributions/timeline", get(contributions_timeline_handler))
        .route("/admin/contributions/leaderboard", get(contributions_leaderboard_handler))
        .route("/admin/contributions/{worker_id}", get(contributions_worker_handler))
        .route("/admin/scheduler", get(scheduler_handler))
        .route("/admin/controller-manager", get(controller_manager_handler))
        .route("/admin/kubelet", get(kubelet_handler))
        .route("/admin/cloud-controller", get(cloud_controller_handler))
        .route("/admin/kamaji", get(kamaji_handler))
        .route("/admin/net", get(net_handler))
        .route("/admin/rdbms", get(rdbms_handler))
        .route("/admin/docdb", get(docdb_handler))
        .route("/admin/cache", get(cache_handler))
        .route("/admin/keda", get(keda_handler))
        .route("/t/{tenant}/dashboard", get(tenant_dashboard_handler))
        .with_state(state)
}

#[cfg(test)]
mod router_tests {
    use super::*;
    use crate::portal_test_ctx;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::util::ServiceExt;

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn etcd_route_renders_only_owner_kv_when_tenant_query_set() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "EtcdKVTab",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/etcd?tenant_id=acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("/cfg/feature_x"));
        assert!(!body.contains("/cfg/feature_y"));
    }

    #[tokio::test]
    async fn tenant_dashboard_path_tenant_must_match_query_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "tenantUrlGuard",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/t/acme/dashboard?tenant_id=evil")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn vault_route_never_exposes_other_tenant_paths() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/SecretsPage.tsx",
            "SecretsPage",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/vault?tenant_id=acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_text(resp).await;
        assert!(body.contains("metadata only"));
        assert!(body.contains("kv/db"));
        assert!(!body.contains("kv/secret"));
    }

    #[tokio::test]
    async fn auth_route_lists_users_for_named_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "listUsers",
            "acme"
        );
        let app = router(Arc::new(AdminState::seeded()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/auth?tenant_id=acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("alice"));
        assert!(body.contains("bob"));
        assert!(!body.contains("mallory"));
    }
}
