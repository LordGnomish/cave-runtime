// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-crossplane.

use crate::CrossplaneState;
use crate::models::{
    CreateClaimRequest, CreateCompositionRequest, CreateProviderRequest, CreateXrdRequest,
    DeletionPolicy,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

type AppState = Arc<CrossplaneState>;

pub fn create_router(state: Arc<CrossplaneState>) -> Router {
    Router::new()
        // Health
        .route("/api/crossplane/health", get(health))
        // XRDs
        .route("/api/crossplane/xrds", get(list_xrds).post(create_xrd))
        .route(
            "/api/crossplane/xrds/{group}/{kind}",
            get(get_xrd).delete(delete_xrd),
        )
        .route(
            "/api/crossplane/xrds/{group}/{kind}/versions",
            get(get_xrd_versions),
        )
        .route("/api/crossplane/xrds/render-crd", post(render_xrd_crd))
        // Compositions
        .route(
            "/api/crossplane/compositions",
            get(list_compositions).post(create_composition),
        )
        .route(
            "/api/crossplane/compositions/{name}",
            get(get_composition).delete(delete_composition),
        )
        .route(
            "/api/crossplane/compositions/{name}/revisions",
            get(get_composition_revisions),
        )
        .route(
            "/api/crossplane/compositions/{name}/revisions/gc",
            post(gc_composition_revisions),
        )
        .route(
            "/api/crossplane/compositions/select",
            post(select_composition),
        )
        // Claims (namespaced)
        .route(
            "/api/crossplane/namespaces/{ns}/claims/{kind}",
            get(list_claims).post(create_claim),
        )
        .route(
            "/api/crossplane/namespaces/{ns}/claims/{kind}/{name}",
            get(get_claim).delete(delete_claim),
        )
        .route(
            "/api/crossplane/namespaces/{ns}/claims/{kind}/{name}/sync",
            post(sync_claim),
        )
        // Composites
        .route("/api/crossplane/composites", get(list_composites))
        .route(
            "/api/crossplane/composites/{kind}/{name}",
            get(get_composite),
        )
        // Providers
        .route(
            "/api/crossplane/providers",
            get(list_providers).post(install_provider),
        )
        .route("/api/crossplane/providers/catalog", get(provider_catalog))
        .route(
            "/api/crossplane/providers/{name}",
            get(get_provider).delete(delete_provider),
        )
        .route(
            "/api/crossplane/providers/{name}/managed-types",
            get(provider_managed_types),
        )
        .route(
            "/api/crossplane/providers/{name}/healthy",
            post(mark_provider_healthy),
        )
        // Usages (deletion protection)
        .route(
            "/api/crossplane/usages",
            get(list_usages).post(create_usage),
        )
        .route("/api/crossplane/usages/{name}", axum::routing::delete(delete_usage))
        .route(
            "/api/crossplane/usages/admit-deletion",
            post(admit_deletion),
        )
        // Reconcile
        .route("/api/crossplane/reconcile/queue", get(reconcile_queue))
        .route("/api/crossplane/reconcile/history", get(reconcile_history))
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-crossplane",
        "status": "ok",
        "upstream": "crossplane/crossplane"
    }))
}

// ── XRDs ──────────────────────────────────────────────────────────────────────

async fn list_xrds(State(s): State<AppState>) -> Json<serde_json::Value> {
    let xrds = s.xrd_store.list();
    Json(json!({
        "items": xrds.iter().map(|x| json!({
            "id": x.id,
            "name": x.name,
            "group": x.group,
            "kind": x.kind,
            "scope": format!("{:?}", x.scope),
            "status": format!("{:?}", x.status),
            "versions": x.versions.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
            "created_at": x.created_at,
        })).collect::<Vec<_>>(),
        "total": xrds.len(),
    }))
}

async fn create_xrd(
    State(s): State<AppState>,
    Json(req): Json<CreateXrdRequest>,
) -> impl IntoResponse {
    match s.xrd_store.create(req) {
        Ok(xrd) => (
            StatusCode::CREATED,
            Json(json!({
                "id": xrd.id,
                "name": xrd.name,
                "group": xrd.group,
                "kind": xrd.kind,
                "status": format!("{:?}", xrd.status),
                "created_at": xrd.created_at,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_xrd(
    State(s): State<AppState>,
    Path((group, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.xrd_store.get(&group, &kind) {
        Ok(xrd) => Json(json!({
            "id": xrd.id,
            "name": xrd.name,
            "group": xrd.group,
            "kind": xrd.kind,
            "list_kind": xrd.list_kind,
            "claim_kind": xrd.claim_kind,
            "scope": format!("{:?}", xrd.scope),
            "status": format!("{:?}", xrd.status),
            "versions": xrd.versions,
            "created_at": xrd.created_at,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_xrd(
    State(s): State<AppState>,
    Path((group, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.xrd_store.delete(&group, &kind) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_xrd_versions(
    State(s): State<AppState>,
    Path((group, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.xrd_store.get(&group, &kind) {
        Ok(xrd) => Json(json!({
            "group": xrd.group,
            "kind": xrd.kind,
            "versions": xrd.versions,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Compositions ──────────────────────────────────────────────────────────────

async fn list_compositions(State(s): State<AppState>) -> Json<serde_json::Value> {
    let compositions = s.composition_store.list();
    Json(json!({
        "items": compositions.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "composite_type_ref": c.composite_type_ref,
            "mode": format!("{:?}", c.mode),
            "status": format!("{:?}", c.status),
            "revision": c.revision,
            "created_at": c.created_at,
        })).collect::<Vec<_>>(),
        "total": compositions.len(),
    }))
}

async fn create_composition(
    State(s): State<AppState>,
    Json(req): Json<CreateCompositionRequest>,
) -> impl IntoResponse {
    match s.composition_store.create(req) {
        Ok(c) => (
            StatusCode::CREATED,
            Json(json!({
                "id": c.id,
                "name": c.name,
                "status": format!("{:?}", c.status),
                "revision": c.revision,
                "created_at": c.created_at,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_composition(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.composition_store.get(&name) {
        Ok(c) => Json(json!({
            "id": c.id,
            "name": c.name,
            "composite_type_ref": c.composite_type_ref,
            "resources": c.resources.len(),
            "pipeline_steps": c.pipeline.len(),
            "mode": format!("{:?}", c.mode),
            "status": format!("{:?}", c.status),
            "revision": c.revision,
            "created_at": c.created_at,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_composition(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.composition_store.delete(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_composition_revisions(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.composition_store.get_revisions(&name) {
        Ok(revisions) => Json(json!({
            "name": name,
            "revisions": revisions.iter().map(|r| json!({
                "revision": r.revision,
                "status": format!("{:?}", r.status),
                "created_at": r.created_at,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct GcQuery {
    /// revisionHistoryLimit: omitted → default 1; 0 → keep all.
    limit: Option<i64>,
}

/// Garbage-collect old composition revisions per `revisionHistoryLimit`.
async fn gc_composition_revisions(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<GcQuery>,
) -> impl IntoResponse {
    match s.composition_store.gc_revisions(&name, q.limit) {
        Ok(collected) => Json(json!({
            "name": name,
            "limit": q.limit,
            "collected": collected,
            "remaining": s
                .composition_store
                .get_revisions(&name)
                .map(|r| r.len())
                .unwrap_or(0),
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── XRD → CRD rendering ─────────────────────────────────────────────────────────

/// Render an XRD (supplied as an XrdSpec body) into its composite + claim CRDs
/// (preview — pure in-crate transform, mirrors xcrd.ForCompositeResource).
async fn render_xrd_crd(Json(spec): Json<crate::xrd::spec::XrdSpec>) -> impl IntoResponse {
    let xr = crate::xrd::crd_gen::for_composite_resource(&spec);
    let claim = crate::xrd::crd_gen::for_composite_resource_claim(&spec);
    Json(json!({
        "compositeResourceDefinition": xr,
        "claimDefinition": claim,
    }))
}

// ── Composition selection (matchLabels resolver) ───────────────────────────────

#[derive(Deserialize)]
struct CandidateBody {
    name: String,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
    api_version: String,
    kind: String,
}

#[derive(Deserialize)]
struct SelectBody {
    xr_api_version: String,
    xr_kind: String,
    #[serde(default)]
    composition_ref: Option<String>,
    #[serde(default)]
    match_labels: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    candidates: Vec<CandidateBody>,
}

/// Dry-run the `compositionSelector.matchLabels` resolver against a supplied
/// candidate set (preview — pure in-crate policy, no apiserver round-trip).
async fn select_composition(Json(body): Json<SelectBody>) -> impl IntoResponse {
    use crate::composition::selector::{
        CompositionCandidate, LabelSelectorResolver, SelectError, SelectionOutcome,
    };
    let candidates: Vec<CompositionCandidate> = body
        .candidates
        .into_iter()
        .map(|c| CompositionCandidate {
            name: c.name,
            labels: c.labels,
            api_version: c.api_version,
            kind: c.kind,
        })
        .collect();
    match LabelSelectorResolver::select(
        &body.xr_api_version,
        &body.xr_kind,
        body.composition_ref.as_deref(),
        body.match_labels.as_ref(),
        &candidates,
    ) {
        Ok(SelectionOutcome::AlreadySet(name)) => {
            Json(json!({"resolved": name, "mode": "alreadySet"})).into_response()
        }
        Ok(SelectionOutcome::Selected(name)) => {
            Json(json!({"resolved": name, "mode": "selected"})).into_response()
        }
        Err(SelectError::NoSelector) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "neither compositionRef nor compositionSelector provided"})),
        )
            .into_response(),
        Err(SelectError::NoCompatibleComposition) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no compatible composition matched the selector"})),
        )
            .into_response(),
    }
}

// ── Usages (deletion protection) ───────────────────────────────────────────────

#[derive(Deserialize)]
struct TargetBody {
    api_version: String,
    kind: String,
    name: String,
    #[serde(default)]
    namespace: Option<String>,
}

impl TargetBody {
    fn into_target(self) -> crate::usage::ResourceTarget {
        let t = crate::usage::ResourceTarget::new(self.api_version, self.kind, self.name);
        match self.namespace {
            Some(ns) => t.in_namespace(ns),
            None => t,
        }
    }
}

#[derive(Deserialize)]
struct CreateUsageBody {
    name: String,
    of: TargetBody,
    #[serde(default)]
    by: Option<TargetBody>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    replay_deletion: bool,
}

async fn list_usages(State(s): State<AppState>) -> Json<serde_json::Value> {
    let items = s.usage_store.list();
    Json(json!({
        "items": items.iter().map(|u| json!({
            "name": u.name,
            "of": {"apiVersion": u.of.api_version, "kind": u.of.kind, "name": u.of.name, "namespace": u.of.namespace},
            "by": u.by.as_ref().map(|b| json!({"apiVersion": b.api_version, "kind": b.kind, "name": b.name, "namespace": b.namespace})),
            "reason": u.reason,
            "replayDeletion": u.replay_deletion,
        })).collect::<Vec<_>>(),
        "total": items.len(),
        "inUseLabel": crate::usage::IN_USE_LABEL,
        "finalizer": crate::usage::FINALIZER,
    }))
}

async fn create_usage(
    State(s): State<AppState>,
    Json(req): Json<CreateUsageBody>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "usage name must not be empty"})),
        )
            .into_response();
    }
    let mut usage = crate::usage::Usage::new(req.name.clone(), req.of.into_target());
    if let Some(by) = req.by {
        usage = usage.with_by(by.into_target());
    }
    if let Some(reason) = req.reason {
        usage = usage.with_reason(reason);
    }
    usage = usage.with_replay_deletion(req.replay_deletion);
    s.usage_store.register(usage);
    (
        StatusCode::CREATED,
        Json(json!({"name": req.name, "status": "registered"})),
    )
        .into_response()
}

async fn delete_usage(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Plan any replay BEFORE removing, mirroring the upstream finalizer flow:
    // the replay decision is computed while the Usage still exists.
    let replay = s.usage_store.plan_replay(&name);
    match s.usage_store.remove(&name) {
        Some(_) => Json(json!({
            "name": name,
            "removed": true,
            "replayDeletion": replay.map(|r| json!({
                "apiVersion": r.api_version, "kind": r.kind, "name": r.name, "namespace": r.namespace,
            })),
        }))
        .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Admission decision for deleting a candidate `of` resource. Denies (HTTP 409)
/// while any Usage references it, recording a deletion attempt for replay.
async fn admit_deletion(
    State(s): State<AppState>,
    Json(body): Json<TargetBody>,
) -> impl IntoResponse {
    let target = body.into_target();
    match s.usage_store.admit_deletion(&target) {
        crate::usage::DeletionDecision::Allowed => Json(json!({
            "allowed": true,
        }))
        .into_response(),
        crate::usage::DeletionDecision::Denied { by_usages, message } => (
            StatusCode::CONFLICT,
            Json(json!({
                "allowed": false,
                "byUsages": by_usages,
                "message": message,
            })),
        )
            .into_response(),
    }
}

// ── Claims ────────────────────────────────────────────────────────────────────

async fn list_claims(
    State(s): State<AppState>,
    Path((ns, _kind)): Path<(String, String)>,
) -> Json<serde_json::Value> {
    let claims = s.claim_store.list_claims_for_namespace(&ns);
    Json(json!({
        "namespace": ns,
        "items": claims.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "namespace": c.namespace,
            "kind": c.kind,
            "status": format!("{:?}", c.status),
            "sync_status": format!("{:?}", c.sync_status),
            "composite_ref": c.composite_ref,
            "created_at": c.created_at,
        })).collect::<Vec<_>>(),
        "total": claims.len(),
    }))
}

#[derive(Deserialize)]
struct CreateClaimBody {
    name: String,
    api_version: String,
    spec: serde_json::Value,
    xrd_group: String,
    xrd_kind: String,
    composition_name: String,
}

async fn create_claim(
    State(s): State<AppState>,
    Path((ns, kind)): Path<(String, String)>,
    Json(body): Json<CreateClaimBody>,
) -> impl IntoResponse {
    // Resolve XRD and Composition
    let xrd = match s.xrd_store.get(&body.xrd_group, &body.xrd_kind) {
        Ok(x) => x,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("XRD not found: {}/{}", body.xrd_group, body.xrd_kind)})),
            )
                .into_response();
        }
    };

    let composition = match s.composition_store.get(&body.composition_name) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Composition not found: {}", body.composition_name)})),
            )
                .into_response();
        }
    };

    let req = CreateClaimRequest {
        name: body.name,
        namespace: ns,
        kind,
        api_version: body.api_version,
        spec: body.spec,
    };

    match s
        .claim_store
        .create_claim(req, &xrd, &composition, &s.engine)
    {
        Ok((claim, composite)) => (
            StatusCode::CREATED,
            Json(json!({
                "claim": {
                    "id": claim.id,
                    "name": claim.name,
                    "namespace": claim.namespace,
                    "kind": claim.kind,
                    "status": format!("{:?}", claim.status),
                    "composite_ref": claim.composite_ref,
                    "created_at": claim.created_at,
                },
                "composite": {
                    "id": composite.id,
                    "name": composite.name,
                    "kind": composite.kind,
                    "status": format!("{:?}", composite.status),
                    "synced_resources": composite.synced_resources.len(),
                    "created_at": composite.created_at,
                }
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_claim(
    State(s): State<AppState>,
    Path((ns, kind, name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match s.claim_store.get_claim(&ns, &name, &kind) {
        Ok(c) => Json(json!({
            "id": c.id,
            "name": c.name,
            "namespace": c.namespace,
            "kind": c.kind,
            "api_version": c.api_version,
            "spec": c.spec,
            "status": format!("{:?}", c.status),
            "sync_status": format!("{:?}", c.sync_status),
            "composite_ref": c.composite_ref,
            "created_at": c.created_at,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct DeleteClaimQuery {
    deletion_policy: Option<String>,
}

async fn delete_claim(
    State(s): State<AppState>,
    Path((ns, kind, name)): Path<(String, String, String)>,
    Query(q): Query<DeleteClaimQuery>,
) -> impl IntoResponse {
    let policy = match q.deletion_policy.as_deref() {
        Some("Orphan") | Some("orphan") => DeletionPolicy::Orphan,
        _ => DeletionPolicy::Delete,
    };
    match s.claim_store.delete_claim(&ns, &name, &kind, policy) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn sync_claim(
    State(s): State<AppState>,
    Path((ns, kind, name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let claim_key = format!("{}/{}/{}", ns, name, kind);
    match s.claim_store.sync_claim_from_composite(&claim_key) {
        Ok(()) => Json(json!({"synced": true, "claim_key": claim_key})).into_response(),
        Err(e) => (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Composites ────────────────────────────────────────────────────────────────

async fn list_composites(State(s): State<AppState>) -> Json<serde_json::Value> {
    let composites = s.claim_store.list_composites();
    Json(json!({
        "items": composites.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "kind": c.kind,
            "api_version": c.api_version,
            "status": format!("{:?}", c.status),
            "composition_ref": c.composition_ref,
            "synced_resources": c.synced_resources.len(),
            "created_at": c.created_at,
        })).collect::<Vec<_>>(),
        "total": composites.len(),
    }))
}

async fn get_composite(
    State(s): State<AppState>,
    Path((kind, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.claim_store.get_composite(&kind, &name) {
        Ok(c) => Json(json!({
            "id": c.id,
            "name": c.name,
            "namespace": c.namespace,
            "kind": c.kind,
            "api_version": c.api_version,
            "spec": c.spec,
            "status": format!("{:?}", c.status),
            "composition_ref": c.composition_ref,
            "claim_ref": c.claim_ref,
            "synced_resources": c.synced_resources,
            "created_at": c.created_at,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Providers ─────────────────────────────────────────────────────────────────

async fn list_providers(State(s): State<AppState>) -> Json<serde_json::Value> {
    let providers = s.provider_store.list();
    Json(json!({
        "items": providers.iter().map(|p| json!({
            "id": p.id,
            "name": p.name,
            "package": p.package,
            "provider_type": format!("{:?}", p.provider_type),
            "revision": p.revision,
            "status": format!("{:?}", p.status),
            "managed_resource_types": p.managed_resource_types.len(),
            "created_at": p.created_at,
        })).collect::<Vec<_>>(),
        "total": providers.len(),
    }))
}

async fn install_provider(
    State(s): State<AppState>,
    Json(req): Json<CreateProviderRequest>,
) -> impl IntoResponse {
    match s.provider_store.install(req) {
        Ok(p) => (
            StatusCode::CREATED,
            Json(json!({
                "id": p.id,
                "name": p.name,
                "package": p.package,
                "status": format!("{:?}", p.status),
                "managed_resource_types": p.managed_resource_types,
                "created_at": p.created_at,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn provider_catalog(State(s): State<AppState>) -> Json<serde_json::Value> {
    let catalog = s.provider_store.catalog();
    Json(json!({
        "items": catalog.iter().map(|p| json!({
            "name": p.name,
            "package": p.package,
            "provider_type": format!("{:?}", p.provider_type),
            "revision": p.revision,
            "managed_resource_types": p.managed_resource_types,
        })).collect::<Vec<_>>(),
        "total": catalog.len(),
    }))
}

async fn get_provider(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.provider_store.get(&name) {
        Ok(p) => Json(json!({
            "id": p.id,
            "name": p.name,
            "package": p.package,
            "provider_type": format!("{:?}", p.provider_type),
            "revision": p.revision,
            "status": format!("{:?}", p.status),
            "managed_resource_types": p.managed_resource_types,
            "created_at": p.created_at,
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_provider(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match s.provider_store.delete(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn provider_managed_types(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.provider_store.get(&name) {
        Ok(p) => Json(json!({
            "provider": p.name,
            "managed_resource_types": p.managed_resource_types,
            "total": p.managed_resource_types.len(),
        }))
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn mark_provider_healthy(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.provider_store.mark_healthy(&name) {
        Ok(()) => Json(json!({"provider": name, "status": "Installed"})).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Reconcile ─────────────────────────────────────────────────────────────────

async fn reconcile_queue(State(s): State<AppState>) -> Json<serde_json::Value> {
    let items = s.reconcile_queue.list_all().await;
    Json(json!({
        "items": items.iter().map(|i| json!({
            "id": i.id,
            "resource_kind": i.resource_kind,
            "resource_name": i.resource_name,
            "namespace": i.namespace,
            "status": format!("{:?}", i.status),
            "attempts": i.attempts,
            "last_error": i.last_error,
            "created_at": i.created_at,
            "updated_at": i.updated_at,
        })).collect::<Vec<_>>(),
        "total": items.len(),
    }))
}

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<usize>,
}

async fn reconcile_history(
    State(s): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let limit = q.limit.unwrap_or(50);
    let items = s.reconcile_queue.history(limit).await;
    Json(json!({
        "items": items.iter().map(|i| json!({
            "id": i.id,
            "resource_kind": i.resource_kind,
            "resource_name": i.resource_name,
            "namespace": i.namespace,
            "status": format!("{:?}", i.status),
            "attempts": i.attempts,
            "last_error": i.last_error,
            "created_at": i.created_at,
            "updated_at": i.updated_at,
        })).collect::<Vec<_>>(),
        "total": items.len(),
    }))
}
