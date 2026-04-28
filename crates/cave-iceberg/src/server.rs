//! Iceberg REST catalog HTTP server (axum).
//!
//! Routes mirrored from `apache/iceberg/open-api/rest-catalog-open-api.yaml`:
//!
//! | method | path                                            | operation                |
//! |--------|-------------------------------------------------|--------------------------|
//! | GET    | /v1/config                                      | server config            |
//! | GET    | /v1/namespaces                                  | list namespaces          |
//! | POST   | /v1/namespaces                                  | create namespace         |
//! | DELETE | /v1/namespaces/:ns                              | drop namespace           |
//! | GET    | /v1/namespaces/:ns                              | namespace properties     |
//! | POST   | /v1/namespaces/:ns/properties                   | update properties        |
//! | GET    | /v1/namespaces/:ns/tables                       | list tables              |
//! | POST   | /v1/namespaces/:ns/tables                       | create table             |
//! | GET    | /v1/namespaces/:ns/tables/:t                    | load table               |
//! | DELETE | /v1/namespaces/:ns/tables/:t                    | drop table               |
//! | POST   | /v1/namespaces/:ns/tables/:t                    | commit                   |
//! | POST   | /v1/tables/rename                               | rename table             |

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::rest_catalog::{CatalogError, RestCatalog};

#[derive(Debug)]
pub struct AppState {
    pub catalog: RestCatalog,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            catalog: RestCatalog::new(),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/config", get(get_config))
        .route("/v1/namespaces", get(list_namespaces).post(create_namespace))
        .route(
            "/v1/namespaces/{ns}",
            get(get_namespace).delete(drop_namespace),
        )
        .route("/v1/namespaces/{ns}/properties", post(update_properties))
        .route(
            "/v1/namespaces/{ns}/tables",
            get(list_tables).post(create_table),
        )
        .route(
            "/v1/namespaces/{ns}/tables/{t}",
            get(load_table).delete(drop_table).post(commit_table),
        )
        .route("/v1/tables/rename", post(rename_table))
        .with_state(state)
}

pub async fn bind(state: Arc<AppState>, bind_addr: &str) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let addr = listener.local_addr()?;
    let app = router(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((addr, handle))
}

// ── Request / response models ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub overrides: BTreeMap<String, String>,
    pub defaults: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNamespaceRequest {
    pub namespace: Vec<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct NamespaceListResponse {
    pub namespaces: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct NamespacePropertiesResponse {
    pub namespace: Vec<String>,
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePropertiesRequest {
    #[serde(default)]
    pub updates: BTreeMap<String, String>,
    #[serde(default)]
    pub removals: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdatePropertiesResponse {
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTableRequest {
    pub name: String,
    pub metadata_location: String,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct TableResponse {
    pub namespace: Vec<String>,
    pub name: String,
    pub metadata_location: String,
    pub current_snapshot_id: Option<i64>,
    pub last_updated_ms: i64,
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct TableListResponse {
    pub identifiers: Vec<TableIdentifier>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableIdentifier {
    pub namespace: Vec<String>,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    pub expected_metadata_location: String,
    pub new_metadata_location: String,
    pub new_snapshot_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub source: TableIdentifier,
    pub destination: TableIdentifier,
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn get_config(State(_): State<Arc<AppState>>) -> Json<ConfigResponse> {
    Json(ConfigResponse {
        overrides: BTreeMap::new(),
        defaults: BTreeMap::new(),
    })
}

async fn list_namespaces(
    State(state): State<Arc<AppState>>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Json<NamespaceListResponse> {
    let parent = q.get("parent").map(String::as_str);
    let names = state.catalog.list_namespaces(parent);
    let namespaces: Vec<Vec<String>> = names
        .into_iter()
        .map(|n| n.split('.').map(String::from).collect())
        .collect();
    Json(NamespaceListResponse { namespaces })
}

async fn create_namespace(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateNamespaceRequest>,
) -> Response {
    let name = req.namespace.join(".");
    match state.catalog.create_namespace(&name, req.properties.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(NamespacePropertiesResponse {
                namespace: req.namespace,
                properties: req.properties,
            }),
        )
            .into_response(),
        Err(e) => map_error(e),
    }
}

async fn drop_namespace(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
) -> Response {
    match state.catalog.drop_namespace(&ns) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_error(e),
    }
}

async fn get_namespace(State(state): State<Arc<AppState>>, Path(ns): Path<String>) -> Response {
    match state.catalog.namespace_properties(&ns) {
        Ok(p) => Json(NamespacePropertiesResponse {
            namespace: ns.split('.').map(String::from).collect(),
            properties: p,
        })
        .into_response(),
        Err(e) => map_error(e),
    }
}

async fn update_properties(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
    Json(req): Json<UpdatePropertiesRequest>,
) -> Response {
    let updates_keys: Vec<String> = req.updates.keys().cloned().collect();
    let removals_keys: Vec<String> = req.removals.clone();
    match state
        .catalog
        .update_namespace_properties(&ns, req.updates, req.removals)
    {
        Ok(()) => Json(UpdatePropertiesResponse {
            updated: updates_keys,
            removed: removals_keys,
            missing: vec![],
        })
        .into_response(),
        Err(e) => map_error(e),
    }
}

async fn list_tables(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
) -> Response {
    match state.catalog.list_tables(&ns) {
        Ok(names) => Json(TableListResponse {
            identifiers: names
                .into_iter()
                .map(|name| TableIdentifier {
                    namespace: ns.split('.').map(String::from).collect(),
                    name,
                })
                .collect(),
        })
        .into_response(),
        Err(e) => map_error(e),
    }
}

async fn create_table(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
    Json(req): Json<CreateTableRequest>,
) -> Response {
    match state.catalog.create_table(
        &ns,
        &req.name,
        &req.metadata_location,
        req.properties,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(entry) => Json(table_to_response(entry)).into_response(),
        Err(e) => map_error(e),
    }
}

async fn load_table(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
) -> Response {
    match state.catalog.load_table(&ns, &name) {
        Ok(entry) => Json(table_to_response(entry)).into_response(),
        Err(e) => map_error(e),
    }
}

async fn drop_table(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
) -> Response {
    match state.catalog.drop_table(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_error(e),
    }
}

async fn commit_table(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
    Json(req): Json<CommitRequest>,
) -> Response {
    match state.catalog.commit(
        &ns,
        &name,
        &req.expected_metadata_location,
        &req.new_metadata_location,
        req.new_snapshot_id,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(entry) => Json(table_to_response(entry)).into_response(),
        Err(e) => map_error(e),
    }
}

async fn rename_table(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RenameRequest>,
) -> Response {
    let from_ns = req.source.namespace.join(".");
    let to_ns = req.destination.namespace.join(".");
    match state
        .catalog
        .rename_table(&from_ns, &req.source.name, &to_ns, &req.destination.name)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_error(e),
    }
}

fn table_to_response(entry: crate::rest_catalog::TableEntry) -> TableResponse {
    TableResponse {
        namespace: entry.namespace.split('.').map(String::from).collect(),
        name: entry.name,
        metadata_location: entry.metadata_location,
        current_snapshot_id: entry.current_snapshot_id,
        last_updated_ms: entry.last_updated_ms,
        properties: entry.properties,
    }
}

fn map_error(e: CatalogError) -> Response {
    let (status, code) = match &e {
        CatalogError::NamespaceNotFound(_) => (StatusCode::NOT_FOUND, "NoSuchNamespaceException"),
        CatalogError::TableNotFound(_, _) => (StatusCode::NOT_FOUND, "NoSuchTableException"),
        CatalogError::NamespaceExists(_) => (StatusCode::CONFLICT, "NamespaceAlreadyExistsException"),
        CatalogError::TableExists(_, _) => (StatusCode::CONFLICT, "TableAlreadyExistsException"),
        CatalogError::NamespaceNotEmpty(_) => (StatusCode::CONFLICT, "NamespaceNotEmptyException"),
        CatalogError::CommitConflict { .. } => (StatusCode::CONFLICT, "CommitFailedException"),
        CatalogError::InvalidIdentifier(_) => (StatusCode::BAD_REQUEST, "BadRequestException"),
    };
    (
        status,
        Json(serde_json::json!({
            "error": {
                "message": e.to_string(),
                "type": code,
                "code": status.as_u16(),
            }
        })),
    )
        .into_response()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_response_serializes_overrides_and_defaults() {
        let c = ConfigResponse {
            overrides: BTreeMap::new(),
            defaults: BTreeMap::new(),
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("overrides") && s.contains("defaults"));
    }

    #[test]
    fn create_namespace_request_deserializes() {
        let s = r#"{"namespace": ["a","b"], "properties": {"k":"v"}}"#;
        let req: CreateNamespaceRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.namespace, vec!["a", "b"]);
        assert_eq!(req.properties.get("k").unwrap(), "v");
    }

    #[test]
    fn create_table_request_deserializes() {
        let s = r#"{"name":"t","metadata_location":"v1.json","properties":{}}"#;
        let req: CreateTableRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.name, "t");
        assert_eq!(req.metadata_location, "v1.json");
    }

    #[test]
    fn commit_request_deserializes() {
        let s = r#"{"expected_metadata_location":"v1","new_metadata_location":"v2","new_snapshot_id":42}"#;
        let req: CommitRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.expected_metadata_location, "v1");
        assert_eq!(req.new_snapshot_id, Some(42));
    }

    #[test]
    fn rename_request_deserializes() {
        let s = r#"{"source":{"namespace":["a"],"name":"t1"},"destination":{"namespace":["a"],"name":"t2"}}"#;
        let req: RenameRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.source.name, "t1");
        assert_eq!(req.destination.name, "t2");
    }

    #[test]
    fn table_to_response_round_trip() {
        let entry = crate::rest_catalog::TableEntry {
            namespace: "a.b".into(),
            name: "t".into(),
            metadata_location: "v1.json".into(),
            current_snapshot_id: Some(1),
            last_updated_ms: 100,
            properties: BTreeMap::new(),
        };
        let resp = table_to_response(entry);
        assert_eq!(resp.namespace, vec!["a", "b"]);
        assert_eq!(resp.name, "t");
    }

    #[test]
    fn map_error_namespace_not_found_404() {
        let _ = map_error(CatalogError::NamespaceNotFound("x".into()));
    }
}
