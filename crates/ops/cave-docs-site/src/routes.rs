// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-docs-site.

use crate::{
    DocsSiteState,
    models::{DocPage, DocSite},
    renderer,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<DocsSiteState>) -> Router {
    Router::new()
        // Sites
        .route("/api/v1/docs-site/sites", get(list_sites).post(create_site))
        .route(
            "/api/v1/docs-site/sites/{id}",
            get(get_site).put(update_site).delete(delete_site),
        )
        // Navigation tree
        .route("/api/v1/docs-site/sites/{id}/nav", get(get_nav))
        // Pages
        .route(
            "/api/v1/docs-site/sites/{id}/pages",
            get(list_pages).post(create_page),
        )
        .route(
            "/api/v1/docs-site/sites/{id}/pages/{page_id}",
            get(get_page).put(update_page).delete(delete_page),
        )
        // Search
        .route("/api/v1/docs-site/sites/{id}/search", get(search_pages))
        // Versioning
        .route(
            "/api/v1/docs-site/sites/{id}/versions",
            get(list_versions).post(create_version),
        )
        // Health
        .route("/api/v1/docs-site/health", get(health))
        .with_state(state)
}

// ── Request / Response DTOs ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateSiteRequest {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub base_url: String,
}

#[derive(Deserialize)]
pub struct CreatePageRequest {
    pub title: String,
    pub path: String,
    pub content: String,
    pub order: u32,
    pub parent_id: Option<Uuid>,
    pub version: String,
}

#[derive(Deserialize)]
pub struct UpdatePageRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub order: Option<u32>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResult {
    pub page_id: Uuid,
    pub title: String,
    pub path: String,
    pub excerpt: String,
}

#[derive(Deserialize)]
pub struct CreateVersionRequest {
    pub label: String,
    pub is_default: bool,
}

#[derive(Serialize)]
pub struct VersionInfo {
    pub label: String,
    pub page_count: usize,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn list_sites(State(state): State<Arc<DocsSiteState>>) -> Json<Vec<DocSite>> {
    Json(state.sites.lock().unwrap().clone())
}

async fn create_site(
    State(state): State<Arc<DocsSiteState>>,
    Json(req): Json<CreateSiteRequest>,
) -> (StatusCode, Json<DocSite>) {
    let site = DocSite::new(req.name, req.slug, req.description, req.base_url);
    state.sites.lock().unwrap().push(site.clone());
    (StatusCode::CREATED, Json(site))
}

async fn get_site(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DocSite>, StatusCode> {
    state
        .sites
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_site(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateSiteRequest>,
) -> Result<Json<DocSite>, StatusCode> {
    let mut sites = state.sites.lock().unwrap();
    let site = sites
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    site.name = req.name;
    site.slug = req.slug;
    site.description = req.description;
    site.base_url = req.base_url;
    site.updated_at = Utc::now();
    Ok(Json(site.clone()))
}

async fn delete_site(State(state): State<Arc<DocsSiteState>>, Path(id): Path<Uuid>) -> StatusCode {
    let mut sites = state.sites.lock().unwrap();
    let before = sites.len();
    sites.retain(|s| s.id != id);
    if sites.len() < before {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_nav(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<renderer::NavNode>>, StatusCode> {
    {
        let sites = state.sites.lock().unwrap();
        if sites.iter().all(|s| s.id != id) {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    let pages = state.pages.lock().unwrap();
    let site_pages: Vec<DocPage> = pages.iter().filter(|p| p.site_id == id).cloned().collect();
    Ok(Json(renderer::build_nav_tree(&site_pages)))
}

async fn list_pages(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<DocPage>> {
    let pages = state.pages.lock().unwrap();
    Json(pages.iter().filter(|p| p.site_id == id).cloned().collect())
}

async fn create_page(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreatePageRequest>,
) -> (StatusCode, Json<DocPage>) {
    let page = DocPage::new(
        id,
        req.title,
        req.path,
        req.content,
        req.order,
        req.parent_id,
        req.version,
    );
    state.pages.lock().unwrap().push(page.clone());
    (StatusCode::CREATED, Json(page))
}

async fn get_page(
    State(state): State<Arc<DocsSiteState>>,
    Path((site_id, page_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<DocPage>, StatusCode> {
    state
        .pages
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.site_id == site_id && p.id == page_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_page(
    State(state): State<Arc<DocsSiteState>>,
    Path((site_id, page_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdatePageRequest>,
) -> Result<Json<DocPage>, StatusCode> {
    let mut pages = state.pages.lock().unwrap();
    let page = pages
        .iter_mut()
        .find(|p| p.site_id == site_id && p.id == page_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    if let Some(t) = req.title {
        page.title = t;
    }
    if let Some(c) = req.content {
        page.content = c;
    }
    if let Some(o) = req.order {
        page.order = o;
    }
    page.updated_at = Utc::now();
    Ok(Json(page.clone()))
}

async fn delete_page(
    State(state): State<Arc<DocsSiteState>>,
    Path((site_id, page_id)): Path<(Uuid, Uuid)>,
) -> StatusCode {
    let mut pages = state.pages.lock().unwrap();
    let before = pages.len();
    pages.retain(|p| !(p.site_id == site_id && p.id == page_id));
    if pages.len() < before {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn search_pages(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
    Query(params): Query<SearchQuery>,
) -> Json<Vec<SearchResult>> {
    let pages = state.pages.lock().unwrap();
    let q = params.q.to_lowercase();
    let version = params.version.unwrap_or_else(|| "latest".to_string());

    let results = pages
        .iter()
        .filter(|p| p.site_id == id)
        .filter(|p| version == "latest" || p.version == version)
        .filter(|p| p.title.to_lowercase().contains(&q) || p.content.to_lowercase().contains(&q))
        .map(|p| {
            let excerpt: String = p
                .content
                .lines()
                .find(|l| l.to_lowercase().contains(&q))
                .unwrap_or(&p.title)
                .trim()
                .chars()
                .take(160)
                .collect();
            SearchResult {
                page_id: p.id,
                title: p.title.clone(),
                path: p.path.clone(),
                excerpt,
            }
        })
        .collect();

    Json(results)
}

async fn list_versions(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<VersionInfo>> {
    let pages = state.pages.lock().unwrap();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for page in pages.iter().filter(|p| p.site_id == id) {
        *counts.entry(page.version.clone()).or_insert(0) += 1;
    }
    Json(
        counts
            .into_iter()
            .map(|(label, page_count)| VersionInfo { label, page_count })
            .collect(),
    )
}

async fn create_version(
    State(state): State<Arc<DocsSiteState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateVersionRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut pages = state.pages.lock().unwrap();
    let source: Vec<DocPage> = pages.iter().filter(|p| p.site_id == id).cloned().collect();
    let mut cloned: Vec<DocPage> = source
        .into_iter()
        .map(|mut p| {
            p.id = Uuid::new_v4();
            p.version = req.label.clone();
            p.created_at = Utc::now();
            p.updated_at = Utc::now();
            p
        })
        .collect();
    pages.append(&mut cloned);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "label": req.label,
            "is_default": req.is_default,
            "site_id": id,
        })),
    )
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-docs-site",
        "status": "ok",
        "upstream": "GitBook / Docusaurus",
    }))
}
