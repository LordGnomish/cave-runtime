//! HTTP surface for the Nexus module.
//!
//! Endpoint base mirrors Nexus' `/service/rest/v1` surface but rooted under
//! `/api/nexus/v1` to live alongside the harbor and pulp surfaces in the
//! same axum router. Raw upload/download retains the upstream
//! `/repository/{name}/*path` URL so existing clients work with only a base
//! URL change.

use super::cleanup;
use super::error::NexusError;
use super::format::FormatRegistry;
use super::models::{
    Asset, BlobRef, CleanupCriteria, CleanupPolicy, Component, ComponentCoord,
    CreateCleanupPolicyRequest, CreateRepositoryRequest, CreateRoutingRuleRequest, ErrorBody,
    Format, Repository, RepositoryType, RoutingDecision, RoutingRule, UpdateRepositoryRequest,
    WritePolicy,
};
use super::routing;
use super::store::{sha256_hex, NexusStore};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// Bundled state for the Nexus subsystem.
pub struct NexusState {
    pub store: Arc<NexusStore>,
    pub formats: Arc<FormatRegistry>,
}

impl NexusState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            store: Arc::new(NexusStore::new()),
            formats: Arc::new(FormatRegistry::with_defaults()),
        })
    }
}

impl Default for NexusState {
    fn default() -> Self {
        Self {
            store: Arc::new(NexusStore::new()),
            formats: Arc::new(FormatRegistry::with_defaults()),
        }
    }
}

pub fn router(state: Arc<NexusState>) -> Router {
    Router::new()
        // Module health
        .route("/api/nexus/v1/health", get(health))
        // Repository management
        .route(
            "/api/nexus/v1/repositories",
            get(list_repositories).post(create_repository),
        )
        .route(
            "/api/nexus/v1/repositories/{name}",
            get(get_repository).put(update_repository).delete(delete_repository),
        )
        // Components
        .route("/api/nexus/v1/components", get(list_components))
        .route(
            "/api/nexus/v1/components/{id}",
            get(get_component).delete(delete_component),
        )
        // Assets
        .route("/api/nexus/v1/assets", get(list_assets))
        .route(
            "/api/nexus/v1/assets/{id}",
            get(get_asset).delete(delete_asset),
        )
        // Cleanup policies
        .route(
            "/api/nexus/v1/cleanup-policies",
            get(list_cleanup_policies).post(create_cleanup_policy),
        )
        .route(
            "/api/nexus/v1/cleanup-policies/{name}",
            get(get_cleanup_policy).delete(delete_cleanup_policy),
        )
        .route(
            "/api/nexus/v1/cleanup-policies/{name}/apply",
            post(apply_cleanup_policy),
        )
        // Routing rules
        .route(
            "/api/nexus/v1/routing-rules",
            get(list_routing_rules).post(create_routing_rule),
        )
        .route(
            "/api/nexus/v1/routing-rules/{name}",
            get(get_routing_rule).delete(delete_routing_rule),
        )
        .route(
            "/api/nexus/v1/routing-rules/{name}/test",
            post(test_routing_rule),
        )
        // Raw repository content surface (mirrors Nexus' /repository/{name}/*path)
        .route(
            "/api/nexus/repository/{name}/{*path}",
            put(upload_raw).get(download_raw).delete(delete_raw),
        )
        .with_state(state)
}

// ── Error mapping ───────────────────────────────────────────────────────

fn map_err(e: NexusError) -> Response {
    let (code, key) = match &e {
        NexusError::RepositoryNotFound(_)
        | NexusError::ComponentNotFound(_)
        | NexusError::AssetNotFound(_)
        | NexusError::BlobNotFound(_)
        | NexusError::CleanupPolicyNotFound(_)
        | NexusError::RoutingRuleNotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        NexusError::RepositoryAlreadyExists(_) => (StatusCode::CONFLICT, "conflict"),
        NexusError::WritePolicyDeny(_) | NexusError::RoutingDenied(_) => {
            (StatusCode::FORBIDDEN, "forbidden")
        }
        NexusError::GroupMemberMissing(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_member"),
        NexusError::InvalidPath(_)
        | NexusError::InvalidRegex(_)
        | NexusError::UnsupportedRepositoryType(_)
        | NexusError::FormatUnavailable(_) => (StatusCode::BAD_REQUEST, "bad_request"),
    };
    (code, Json(ErrorBody::new(key, e.to_string()))).into_response()
}

// ── Health ──────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<NexusState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "module": "nexus",
        "stats": {
            "repositories": state.store.list_repositories().len(),
            "components":   state.store.list_components(None).len(),
            "assets":       state.store.list_assets(None).len(),
            "blobs":        state.store.blob_count(),
            "cleanup_policies": state.store.list_cleanup_policies().len(),
            "routing_rules": state.store.list_routing_rules().len(),
        },
        "supported_formats": state.formats.supported().iter().map(|f| f.as_str()).collect::<Vec<_>>(),
    }))
}

// ── Repositories ────────────────────────────────────────────────────────

async fn list_repositories(State(state): State<Arc<NexusState>>) -> Json<Vec<Repository>> {
    Json(state.store.list_repositories())
}

async fn create_repository(
    State(state): State<Arc<NexusState>>,
    Json(req): Json<CreateRepositoryRequest>,
) -> Response {
    let now = Utc::now();
    let repo = Repository {
        id: Uuid::new_v4(),
        name: req.name,
        format: req.format,
        repo_type: req.repo_type,
        online: req.online,
        cleanup_policies: req.cleanup_policies,
        created_at: now,
        updated_at: now,
    };
    match state.store.create_repository(repo) {
        Ok(r) => (StatusCode::CREATED, Json(r)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn get_repository(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
) -> Response {
    match state.store.get_repository(&name) {
        Ok(r) => Json(r).into_response(),
        Err(e) => map_err(e),
    }
}

async fn update_repository(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Response {
    let result = state.store.update_repository(&name, |repo| {
        if let Some(online) = req.online {
            repo.online = online;
        }
        if let Some(policies) = req.cleanup_policies {
            repo.cleanup_policies = policies;
        }
        match &mut repo.repo_type {
            RepositoryType::Hosted { write_policy } => {
                if let Some(p) = req.write_policy {
                    *write_policy = p;
                }
            }
            RepositoryType::Proxy {
                remote_url,
                cache_ttl_minutes,
            } => {
                if let Some(u) = req.remote_url {
                    *remote_url = u;
                }
                if let Some(t) = req.cache_ttl_minutes {
                    *cache_ttl_minutes = t;
                }
            }
            RepositoryType::Group { member_names } => {
                if let Some(m) = req.member_names {
                    *member_names = m;
                }
            }
        }
    });
    match result {
        Ok(r) => Json(r).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_repository(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
) -> Response {
    match state.store.delete_repository(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

// ── Components ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ComponentQuery {
    repository: Option<String>,
}

async fn list_components(
    State(state): State<Arc<NexusState>>,
    Query(q): Query<ComponentQuery>,
) -> Json<Vec<Component>> {
    Json(state.store.list_components(q.repository.as_deref()))
}

async fn get_component(
    State(state): State<Arc<NexusState>>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.store.get_component(id) {
        Ok(c) => Json(c).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_component(
    State(state): State<Arc<NexusState>>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.store.delete_component(id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

// ── Assets ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AssetQuery {
    repository: Option<String>,
}

async fn list_assets(
    State(state): State<Arc<NexusState>>,
    Query(q): Query<AssetQuery>,
) -> Json<Vec<Asset>> {
    Json(state.store.list_assets(q.repository.as_deref()))
}

async fn get_asset(State(state): State<Arc<NexusState>>, Path(id): Path<Uuid>) -> Response {
    match state.store.get_asset(id) {
        Ok(a) => Json(a).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_asset(State(state): State<Arc<NexusState>>, Path(id): Path<Uuid>) -> Response {
    match state.store.delete_asset(id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

// ── Cleanup policies ────────────────────────────────────────────────────

async fn list_cleanup_policies(State(state): State<Arc<NexusState>>) -> Json<Vec<CleanupPolicy>> {
    Json(state.store.list_cleanup_policies())
}

async fn create_cleanup_policy(
    State(state): State<Arc<NexusState>>,
    Json(req): Json<CreateCleanupPolicyRequest>,
) -> Response {
    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: req.name,
        format: req.format,
        criteria: req.criteria,
        created_at: Utc::now(),
    };
    let stored = state.store.create_cleanup_policy(policy);
    (StatusCode::CREATED, Json(stored)).into_response()
}

async fn get_cleanup_policy(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
) -> Response {
    match state.store.get_cleanup_policy(&name) {
        Ok(p) => Json(p).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_cleanup_policy(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
) -> Response {
    match state.store.delete_cleanup_policy(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct ApplyCleanupQuery {
    repository: String,
    #[serde(default)]
    dry_run: bool,
}

async fn apply_cleanup_policy(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
    Query(q): Query<ApplyCleanupQuery>,
) -> Response {
    let policy = match state.store.get_cleanup_policy(&name) {
        Ok(p) => p,
        Err(e) => return map_err(e),
    };
    if q.dry_run {
        match cleanup::evaluate(&state.store, &policy, &q.repository) {
            Ok(ids) => Json(serde_json::json!({
                "policy": policy.name,
                "repository": q.repository,
                "would_delete": ids.len(),
                "asset_ids": ids,
            }))
            .into_response(),
            Err(e) => map_err(e),
        }
    } else {
        match cleanup::apply(&state.store, &policy, &q.repository) {
            Ok(n) => Json(serde_json::json!({
                "policy": policy.name,
                "repository": q.repository,
                "deleted": n,
            }))
            .into_response(),
            Err(e) => map_err(e),
        }
    }
}

// ── Routing rules ───────────────────────────────────────────────────────

async fn list_routing_rules(State(state): State<Arc<NexusState>>) -> Json<Vec<RoutingRule>> {
    Json(state.store.list_routing_rules())
}

async fn create_routing_rule(
    State(state): State<Arc<NexusState>>,
    Json(req): Json<CreateRoutingRuleRequest>,
) -> Response {
    let rule = RoutingRule {
        id: Uuid::new_v4(),
        name: req.name,
        mode: req.mode,
        matchers: req.matchers,
        created_at: Utc::now(),
    };
    let stored = state.store.create_routing_rule(rule);
    (StatusCode::CREATED, Json(stored)).into_response()
}

async fn get_routing_rule(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
) -> Response {
    match state.store.get_routing_rule(&name) {
        Ok(r) => Json(r).into_response(),
        Err(e) => map_err(e),
    }
}

async fn delete_routing_rule(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
) -> Response {
    match state.store.delete_routing_rule(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
struct TestRoutingQuery {
    path: String,
}

async fn test_routing_rule(
    State(state): State<Arc<NexusState>>,
    Path(name): Path<String>,
    Query(q): Query<TestRoutingQuery>,
) -> Response {
    let rule = match state.store.get_routing_rule(&name) {
        Ok(r) => r,
        Err(e) => return map_err(e),
    };
    match routing::evaluate(&rule, &q.path) {
        Ok(decision) => Json(serde_json::json!({
            "rule": rule.name,
            "path": q.path,
            "decision": decision,
        }))
        .into_response(),
        Err(e) => map_err(e),
    }
}

// ── Raw format upload / download / delete ───────────────────────────────

async fn upload_raw(
    State(state): State<Arc<NexusState>>,
    Path((name, path)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let repo = match state.store.get_repository(&name) {
        Ok(r) => r,
        Err(e) => return map_err(e),
    };
    let write_policy = match repo.repo_type {
        RepositoryType::Hosted { write_policy } => write_policy,
        _ => {
            return map_err(NexusError::UnsupportedRepositoryType(format!(
                "raw upload requires hosted, got {} for {}",
                repo_type_name(&repo.repo_type),
                name
            )))
        }
    };
    let adapter = match state.formats.get(repo.format) {
        Ok(a) => a,
        Err(e) => return map_err(e),
    };
    if let Err(e) = adapter.validate_upload(&path, &body) {
        return map_err(e);
    }

    // Check write policy when an existing asset is found at this path.
    let existing = state.store.get_asset_by_path(&name, &path).ok();
    if let Some(prev) = &existing {
        match write_policy {
            WritePolicy::Deny => {
                return map_err(NexusError::WritePolicyDeny(path));
            }
            WritePolicy::AllowOnce => {
                return map_err(NexusError::WritePolicyDeny(format!(
                    "{path} already uploaded, allow_once policy"
                )));
            }
            WritePolicy::Allow => {
                // Replace: delete the previous asset (and dec-ref its blob).
                if let Err(e) = state.store.delete_asset(prev.id) {
                    return map_err(e);
                }
            }
        }
    }

    // Parse coordinate; create or reuse an existing component for the
    // (path's group, name) pair within the same repo.
    let coord = match adapter.parse_path(&path) {
        Ok(c) => c,
        Err(e) => return map_err(e),
    };
    let component = find_or_create_component(&state.store, &repo, &coord);

    let bytes = body.to_vec();
    let sha = sha256_hex(&bytes);
    let size = bytes.len() as u64;
    let now = Utc::now();
    let asset = Asset {
        id: Uuid::new_v4(),
        component_id: component.id,
        repository_id: repo.id,
        repository_name: repo.name.clone(),
        path: path.clone(),
        blob: BlobRef {
            sha256: sha,
            size,
        },
        content_type: adapter.content_type(&path).to_string(),
        created_at: now,
        last_modified: now,
        last_downloaded: None,
        download_count: 0,
    };
    match state.store.put_asset(asset, bytes) {
        Ok(a) => (StatusCode::CREATED, Json(a)).into_response(),
        Err(e) => map_err(e),
    }
}

async fn download_raw(
    State(state): State<Arc<NexusState>>,
    Path((name, path)): Path<(String, String)>,
) -> Response {
    let asset = match state.store.get_asset_by_path(&name, &path) {
        Ok(a) => a,
        Err(e) => return map_err(e),
    };
    let blob = match state.store.read_blob(&asset.blob.sha256) {
        Ok(b) => b,
        Err(e) => return map_err(e),
    };
    let _ = state.store.record_download(asset.id);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&asset.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        "X-Nexus-SHA256",
        HeaderValue::from_str(&asset.blob.sha256)
            .unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    (StatusCode::OK, headers, blob).into_response()
}

async fn delete_raw(
    State(state): State<Arc<NexusState>>,
    Path((name, path)): Path<(String, String)>,
) -> Response {
    let asset = match state.store.get_asset_by_path(&name, &path) {
        Ok(a) => a,
        Err(e) => return map_err(e),
    };
    match state.store.delete_asset(asset.id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_err(e),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn repo_type_name(t: &RepositoryType) -> &'static str {
    match t {
        RepositoryType::Hosted { .. } => "hosted",
        RepositoryType::Proxy { .. } => "proxy",
        RepositoryType::Group { .. } => "group",
    }
}

/// Look for an existing component in `repo` whose (group, name) matches
/// the coordinate. If none exists, create one and return it. Components
/// dedupe across many uploads in raw repos because the same logical file
/// receives many physical assets only when the path differs.
fn find_or_create_component(
    store: &NexusStore,
    repo: &Repository,
    coord: &ComponentCoord,
) -> Component {
    if let Some(existing) = store
        .list_components(Some(&repo.name))
        .into_iter()
        .find(|c| c.group == coord.group && c.name == coord.name && c.version == coord.version)
    {
        return existing;
    }
    let component = Component {
        id: Uuid::new_v4(),
        repository_id: repo.id,
        repository_name: repo.name.clone(),
        format: repo.format,
        group: coord.group.clone(),
        name: coord.name.clone(),
        version: coord.version.clone(),
        created_at: Utc::now(),
    };
    store.create_component(component)
}

// Keep the unused-criteria type accessible through the public re-exports.
#[allow(dead_code)]
fn _criteria_sentinel() -> CleanupCriteria {
    CleanupCriteria::default()
}

// Keep Format reachable for callers that need it via routes::Format.
#[allow(dead_code)]
fn _format_sentinel(_: Format, _: RoutingDecision) {}
