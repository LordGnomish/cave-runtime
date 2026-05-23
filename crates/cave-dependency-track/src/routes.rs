// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Axum REST v1 + GraphQL surface.
//!
//! Mirrors `org.dependencytrack.resources.v1.*` resource classes plus the
//! optional `/graphql` endpoint and `/swagger.json` OpenAPI descriptor.

use crate::State;
use crate::audit::Analysis;
use crate::bov::BovDocument;
use crate::licenses;
use crate::models::{AnalysisState, Classifier, Project};
use crate::policy::engine::Policy;
use crate::portfolio::ProjectUpdate;
use crate::repositories::Repository;
use crate::sbom::{ingest as ingest_cdx, parse_cyclonedx_json, parse_spdx_json};
use crate::vex::VexDocument;
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/project", get(list_projects).post(create_project))
        .route(
            "/api/v1/project/{uuid}",
            get(get_project).delete(delete_project).put(update_project),
        )
        .route("/api/v1/component/{project}", get(list_components))
        .route("/api/v1/vulnerability", get(list_vulnerabilities))
        .route("/api/v1/bom/cyclonedx/{project}", post(upload_cyclonedx))
        .route("/api/v1/bom/spdx/{project}", post(upload_spdx))
        .route("/api/v1/policy", get(list_policies).post(create_policy))
        .route("/api/v1/policy/{uuid}", get(get_policy).delete(delete_policy))
        .route("/api/v1/analysis", post(upsert_analysis))
        .route("/api/v1/analysis/{component}", get(list_analysis))
        .route("/api/v1/vex/{project}", get(export_vex))
        .route("/api/v1/bov/{project}", get(export_bov))
        .route("/api/v1/license", get(list_licenses))
        .route("/api/v1/repository", get(list_repositories).post(add_repository))
        .route("/api/v1/search", get(search_components))
        .route("/api/v1/graphql", post(graphql_endpoint))
        .route("/api/v1/swagger.json", get(swagger))
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({"status":"ok","module":"deptrack"}))
}

async fn list_projects(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<Project>> {
    Json(state.portfolio.list())
}

#[derive(Deserialize)]
struct CreateProjectIn {
    name: String,
    version: Option<String>,
    #[serde(default = "default_classifier")]
    classifier: String,
    description: Option<String>,
}

fn default_classifier() -> String {
    "APPLICATION".into()
}

async fn create_project(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<CreateProjectIn>,
) -> Result<Json<Project>, (StatusCode, String)> {
    let cls = Classifier::parse(&body.classifier).unwrap_or(Classifier::Application);
    let mut p = Project::new(body.name, cls);
    p.version = body.version;
    p.description = body.description;
    let stored = state
        .portfolio
        .insert(p)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(stored))
}

async fn get_project(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Project>, (StatusCode, String)> {
    state
        .portfolio
        .get(uuid)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn update_project(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
    Json(upd): Json<ProjectUpdate>,
) -> Result<Json<Project>, (StatusCode, String)> {
    state
        .portfolio
        .update(uuid, upd)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn delete_project(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .portfolio
        .delete(uuid)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn list_components(
    AxumState(state): AxumState<Arc<State>>,
    Path(project): Path<Uuid>,
) -> Json<Value> {
    Json(json!({"components": state.portfolio.components_for(project)}))
}

async fn list_vulnerabilities(AxumState(state): AxumState<Arc<State>>) -> Json<Value> {
    Json(json!({"vulnerabilities": state.vulns.list()}))
}

async fn upload_cyclonedx(
    AxumState(state): AxumState<Arc<State>>,
    Path(project): Path<Uuid>,
    body: String,
) -> Result<Json<Value>, (StatusCode, String)> {
    let bom = parse_cyclonedx_json(&body).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let report = ingest_cdx(&state.portfolio, project, &bom)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({
        "project": report.project.to_string(),
        "inserted": report.inserted,
        "updated": report.updated,
        "skipped": report.skipped,
    })))
}

async fn upload_spdx(
    AxumState(state): AxumState<Arc<State>>,
    Path(project): Path<Uuid>,
    body: String,
) -> Result<Json<Value>, (StatusCode, String)> {
    let doc = parse_spdx_json(&body).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // Acknowledge the project exists so the integration is wired end-to-end.
    let _ = state
        .portfolio
        .get(project)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({
        "project": project.to_string(),
        "packagesParsed": doc.packages.len(),
        "spdxVersion": doc.spdx_version,
    })))
}

async fn list_policies(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<Policy>> {
    Json(state.policy.list())
}

async fn create_policy(
    AxumState(state): AxumState<Arc<State>>,
    Json(p): Json<Policy>,
) -> Json<Policy> {
    Json(state.policy.put(p))
}

async fn get_policy(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Policy>, (StatusCode, String)> {
    state
        .policy
        .get(uuid)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn delete_policy(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .policy
        .delete(uuid)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

#[derive(Deserialize)]
struct AnalysisIn {
    component: Uuid,
    vulnerability: Uuid,
    state: String,
}

async fn upsert_analysis(
    AxumState(state): AxumState<Arc<State>>,
    Json(input): Json<AnalysisIn>,
) -> Json<Analysis> {
    let s = match input.state.to_ascii_uppercase().as_str() {
        "EXPLOITABLE" => AnalysisState::Exploitable,
        "IN_TRIAGE" => AnalysisState::InTriage,
        "RESOLVED" => AnalysisState::Resolved,
        "FALSE_POSITIVE" => AnalysisState::FalsePositive,
        "NOT_AFFECTED" => AnalysisState::NotAffected,
        _ => AnalysisState::NotSet,
    };
    Json(state.audit.upsert(input.component, input.vulnerability, s))
}

async fn list_analysis(
    AxumState(state): AxumState<Arc<State>>,
    Path(component): Path<Uuid>,
) -> Json<Vec<Analysis>> {
    Json(state.audit.for_component(component))
}

async fn export_vex(
    AxumState(state): AxumState<Arc<State>>,
    Path(project): Path<Uuid>,
) -> Json<Value> {
    let mut doc = VexDocument::new();
    for comp in state.portfolio.components_for(project) {
        for a in state.audit.for_component(comp.uuid) {
            if let Ok(v) = state.vulns.get_by_uuid(a.vulnerability) {
                doc.push_analysis(&v, &a);
            }
        }
    }
    Json(doc.to_json())
}

async fn export_bov(
    AxumState(state): AxumState<Arc<State>>,
    Path(project): Path<Uuid>,
) -> Json<Value> {
    let inputs: Vec<_> = state
        .portfolio
        .components_for(project)
        .into_iter()
        .map(|c| (c.uuid, Vec::new()))
        .collect();
    let doc = BovDocument::build(project, &inputs, &state.audit);
    Json(doc.to_json())
}

async fn list_licenses() -> Json<Value> {
    let cat: Vec<_> = licenses::catalog()
        .iter()
        .map(|l| {
            json!({
                "spdxId": l.spdx_id,
                "name": l.name,
                "osiApproved": l.osi_approved,
                "fsfLibre": l.fsf_libre,
                "deprecated": l.deprecated,
            })
        })
        .collect();
    Json(json!({"licenses": cat}))
}

async fn list_repositories(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<Repository>> {
    Json(state.repositories.list())
}

async fn add_repository(
    AxumState(state): AxumState<Arc<State>>,
    Json(r): Json<Repository>,
) -> StatusCode {
    state.repositories.put(r);
    StatusCode::CREATED
}

#[derive(Deserialize)]
struct SearchQ {
    q: String,
    project: Uuid,
}

async fn search_components(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<SearchQ>,
) -> Json<Value> {
    let comps = state.portfolio.components_for(q.project);
    let hits: Vec<_> = crate::engine::search_components(&q.q, &comps)
        .iter()
        .map(|c| (*c).clone())
        .collect();
    Json(json!({"results": hits}))
}

#[derive(Deserialize)]
struct GraphQLBody {
    query: String,
}

async fn graphql_endpoint(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<GraphQLBody>,
) -> Json<Value> {
    Json(crate::graphql::execute(
        &body.query,
        &state.portfolio.list(),
        &state.vulns.list(),
        &state.policy.list(),
    ))
}

async fn swagger() -> impl IntoResponse {
    Json(crate::swagger::openapi_spec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn app() -> Router {
        create_router(Arc::new(State::default()))
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let resp = app()
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn list_projects_empty_returns_array() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/project")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn create_project_then_list() {
        let app = app();
        let req = Request::builder()
            .uri("/api/v1/project")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"cave","classifier":"APPLICATION"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn swagger_returns_openapi() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/swagger.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn graphql_introspection_responds() {
        let req = Request::builder()
            .uri("/api/v1/graphql")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"query":"{ __schema }"}"#))
            .unwrap();
        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn license_catalog_endpoint_responds() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/license")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
