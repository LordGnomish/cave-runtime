use crate::error::DocsError;
use crate::openapi::ApiRefGenerator;
use crate::search::SearchIndex;
use crate::store::DocsStore;
use crate::toc::TocGenerator;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

pub struct DocsState {
    pub store: DocsStore,
    pub search: Mutex<SearchIndex>,
}

impl DocsState {
    pub fn new() -> Self {
        DocsState {
            store: DocsStore::new(),
            search: Mutex::new(SearchIndex::new()),
        }
    }
}

impl Default for DocsState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<DocsState>) -> Router {
    Router::new()
        .route("/api/docs/health", get(health))
        .route("/api/docs/spaces", get(list_spaces).post(create_space))
        .route(
            "/api/docs/spaces/:id",
            get(get_space).delete(delete_space),
        )
        .route(
            "/api/docs/spaces/:id/pages",
            get(list_pages).post(create_page),
        )
        .route(
            "/api/docs/spaces/:id/pages/:page_id",
            get(get_page).put(update_page).delete(delete_page),
        )
        .route("/api/docs/spaces/:id/toc", get(get_toc))
        .route("/api/docs/spaces/:id/toc/html", get(get_toc_html))
        .route("/api/docs/search", get(search))
        .route(
            "/api/docs/spaces/:id/versions",
            get(list_versions).post(create_version),
        )
        .route("/api/docs/spaces/:id/openapi", post(import_openapi))
        .with_state(state)
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "module": crate::MODULE_NAME}))
}

#[derive(Deserialize)]
struct CreateSpaceBody {
    slug: String,
    title: String,
    description: String,
}

async fn list_spaces(State(state): State<Arc<DocsState>>) -> impl IntoResponse {
    let spaces = state.store.list_spaces();
    Json(spaces)
}

async fn create_space(
    State(state): State<Arc<DocsState>>,
    Json(body): Json<CreateSpaceBody>,
) -> impl IntoResponse {
    match state
        .store
        .create_space(&body.slug, &body.title, &body.description)
    {
        Ok(space) => (StatusCode::CREATED, Json(serde_json::to_value(space).unwrap())),
        Err(DocsError::SpaceExists(s)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("space exists: {s}")})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_space(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_space(&id) {
        Ok(space) => (StatusCode::OK, Json(serde_json::to_value(space).unwrap())),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn delete_space(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.delete_space(&id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"deleted": id})),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<String>,
}

async fn list_pages(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
    Query(q): Query<VersionQuery>,
) -> impl IntoResponse {
    let version = q.version.as_deref().unwrap_or("main");
    let pages = state.store.list_pages(&id, version);
    Json(serde_json::to_value(pages).unwrap())
}

#[derive(Deserialize)]
struct CreatePageBody {
    slug: String,
    title: String,
    content: String,
    group_id: Option<String>,
    order: Option<u32>,
    version: Option<String>,
}

async fn create_page(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
    Json(body): Json<CreatePageBody>,
) -> impl IntoResponse {
    let version = body.version.as_deref().unwrap_or("main");
    match state.store.create_page(
        &id,
        &body.slug,
        &body.title,
        &body.content,
        body.group_id,
        body.order.unwrap_or(0),
        version,
    ) {
        Ok(page) => {
            // Index for search
            state.search.lock().unwrap().index_page(&page);
            (StatusCode::CREATED, Json(serde_json::to_value(page).unwrap()))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_page(
    State(state): State<Arc<DocsState>>,
    Path((_id, page_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.store.get_page(&page_id) {
        Ok(page) => (StatusCode::OK, Json(serde_json::to_value(page).unwrap())),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

#[derive(Deserialize)]
struct UpdatePageBody {
    title: Option<String>,
    content: Option<String>,
}

async fn update_page(
    State(state): State<Arc<DocsState>>,
    Path((_id, page_id)): Path<(String, String)>,
    Json(body): Json<UpdatePageBody>,
) -> impl IntoResponse {
    match state.store.update_page(
        &page_id,
        body.title.as_deref(),
        body.content.as_deref(),
    ) {
        Ok(page) => {
            // Re-index
            let mut search = state.search.lock().unwrap();
            search.remove_page(&page.id);
            search.index_page(&page);
            (StatusCode::OK, Json(serde_json::to_value(page).unwrap()))
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn delete_page(
    State(state): State<Arc<DocsState>>,
    Path((_id, page_id)): Path<(String, String)>,
) -> impl IntoResponse {
    state.search.lock().unwrap().remove_page(&page_id);
    match state.store.delete_page(&page_id) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"deleted": page_id}))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_toc(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
    Query(q): Query<VersionQuery>,
) -> impl IntoResponse {
    let version = q.version.as_deref().unwrap_or("main");
    let groups = state.store.list_groups(&id, version);
    let pages = state.store.list_pages(&id, version);
    let toc = TocGenerator::build(&groups, &pages);
    Json(serde_json::to_value(toc).unwrap())
}

async fn get_toc_html(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
    Query(q): Query<VersionQuery>,
) -> impl IntoResponse {
    let version = q.version.as_deref().unwrap_or("main");
    let groups = state.store.list_groups(&id, version);
    let pages = state.store.list_pages(&id, version);
    let toc = TocGenerator::build(&groups, &pages);
    let html = TocGenerator::to_html(&toc);
    axum::response::Html(html)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    space_id: Option<String>,
    version: Option<String>,
    limit: Option<usize>,
}

async fn search(
    State(state): State<Arc<DocsState>>,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let results = state.search.lock().unwrap().search(
        &q.q,
        q.space_id.as_deref(),
        q.version.as_deref(),
        q.limit.unwrap_or(20),
    );
    Json(serde_json::to_value(results).unwrap())
}

async fn list_versions(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let versions = state.store.list_versions(&id);
    Json(serde_json::to_value(versions).unwrap())
}

#[derive(Deserialize)]
struct CreateVersionBody {
    name: String,
    branch: Option<String>,
}

async fn create_version(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateVersionBody>,
) -> impl IntoResponse {
    match state
        .store
        .create_version(&id, &body.name, body.branch.as_deref())
    {
        Ok(v) => (StatusCode::CREATED, Json(serde_json::to_value(v).unwrap())),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn import_openapi(
    State(state): State<Arc<DocsState>>,
    Path(id): Path<String>,
    body: String,
) -> impl IntoResponse {
    match ApiRefGenerator::parse(&body) {
        Ok(spec) => {
            let page = ApiRefGenerator::to_page(&spec, &id, "main");
            state.search.lock().unwrap().index_page(&page);
            (
                StatusCode::CREATED,
                Json(serde_json::to_value(page).unwrap()),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}
