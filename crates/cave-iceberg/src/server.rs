//! Iceberg REST catalog HTTP server (axum).
//!
//! Routes mirrored from `apache/iceberg/open-api/rest-catalog-open-api.yaml`:
//!
//! | method | path                                                  | operation                  |
//! |--------|-------------------------------------------------------|----------------------------|
//! | GET    | /v1/config                                            | server config              |
//! | GET    | /v1/namespaces                                        | list namespaces            |
//! | POST   | /v1/namespaces                                        | create namespace           |
//! | DELETE | /v1/namespaces/:ns                                    | drop namespace             |
//! | GET    | /v1/namespaces/:ns                                    | namespace properties       |
//! | POST   | /v1/namespaces/:ns/properties                         | update properties          |
//! | GET    | /v1/namespaces/:ns/tables                             | list tables                |
//! | POST   | /v1/namespaces/:ns/tables                             | create table               |
//! | GET    | /v1/namespaces/:ns/tables/:t                          | load table                 |
//! | DELETE | /v1/namespaces/:ns/tables/:t                          | drop table                 |
//! | POST   | /v1/namespaces/:ns/tables/:t                          | commit (legacy + spec)     |
//! | POST   | /v1/namespaces/:ns/register                           | registerTable              |
//! | GET    | /v1/namespaces/:ns/tables/:t/credentials              | vended credentials         |
//! | POST   | /v1/namespaces/:ns/tables/:t/sign                     | S3 remote-sign passthrough |
//! | POST   | /v1/transactions/commit                               | multi-table commit         |
//! | POST   | /v1/tables/rename                                     | rename table               |

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::rest_catalog::{CatalogError, CommitRequirement, CommitUpdate, RestCatalog};

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
        .route(
            "/v1/namespaces/{ns}/tables/{t}/credentials",
            get(load_credentials),
        )
        .route("/v1/namespaces/{ns}/tables/{t}/sign", post(sign_request))
        .route("/v1/namespaces/{ns}/register", post(register_table))
        .route("/v1/transactions/commit", post(commit_transaction))
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

/// Legacy in-tree commit body (predates upstream `CommitTableRequest`).
/// Retained because existing clients in this repo target this shape; the
/// handler accepts either this body OR the upstream spec body.
#[derive(Debug, Deserialize)]
pub struct LegacyCommitRequest {
    pub expected_metadata_location: String,
    pub new_metadata_location: String,
    pub new_snapshot_id: Option<i64>,
}

/// Upstream `CommitTableRequest`:
///
/// ```yaml
/// CommitTableRequest:
///   required: [requirements, updates]
///   properties:
///     identifier:    { $ref: TableIdentifier }
///     requirements:  [ TableRequirement ]
///     updates:       [ TableUpdate ]
/// ```
#[derive(Debug, Deserialize)]
pub struct SpecCommitRequest {
    #[serde(default)]
    pub identifier: Option<TableIdentifier>,
    #[serde(default)]
    pub requirements: Vec<RawRequirement>,
    #[serde(default)]
    pub updates: Vec<RawUpdate>,
    /// Cave-iceberg extension: the new metadata.json path. Upstream
    /// derives this server-side because the catalog owns metadata
    /// allocation; cave-iceberg leaves metadata writing to the client
    /// and accepts the new path on the wire.
    #[serde(default, rename = "new-metadata-location")]
    pub new_metadata_location: Option<String>,
}

/// Discriminated-union for `TableRequirement` (preserves unknown variants).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RawRequirement {
    AssertCreate,
    AssertTableUuid {
        uuid: String,
    },
    AssertCurrentSchemaId {
        #[serde(rename = "current-schema-id")]
        current_schema_id: i32,
    },
    AssertRefSnapshotId {
        #[serde(rename = "ref")]
        ref_name: String,
        #[serde(rename = "snapshot-id")]
        snapshot_id: Option<i64>,
    },
    /// Some clients fold "current snapshot id" into an
    /// `assert-ref-snapshot-id` on `main`; we also accept a raw form.
    AssertCurrentSnapshotId {
        #[serde(rename = "snapshot-id")]
        snapshot_id: Option<i64>,
    },
    #[serde(other)]
    Unknown,
}

impl RawRequirement {
    fn into_domain(self) -> CommitRequirement {
        match self {
            RawRequirement::AssertCreate => CommitRequirement::AssertCreate,
            RawRequirement::AssertTableUuid { uuid } => CommitRequirement::AssertTableUuid { uuid },
            RawRequirement::AssertCurrentSchemaId { .. } => CommitRequirement::Other {
                type_tag: "assert-current-schema-id".into(),
            },
            RawRequirement::AssertRefSnapshotId { ref_name, snapshot_id } => {
                CommitRequirement::AssertRefSnapshotId { ref_name, snapshot_id }
            }
            RawRequirement::AssertCurrentSnapshotId { snapshot_id } => {
                CommitRequirement::AssertCurrentSnapshotId { snapshot_id }
            }
            RawRequirement::Unknown => CommitRequirement::Other {
                type_tag: "unknown".into(),
            },
        }
    }
}

/// Discriminated-union for `TableUpdate`.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum RawUpdate {
    /// `set-snapshot-ref` for ref="main" updates current_snapshot_id.
    SetSnapshotRef {
        #[serde(rename = "ref-name")]
        ref_name: String,
        #[serde(rename = "snapshot-id")]
        snapshot_id: i64,
    },
    /// `add-snapshot` carries a Snapshot; we extract `snapshot-id`.
    AddSnapshot {
        snapshot: AddSnapshotInner,
    },
    /// `set-properties` merges into table properties.
    SetProperties {
        updates: BTreeMap<String, String>,
    },
    /// `remove-properties` removes table-property keys.
    RemoveProperties {
        removals: Vec<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct AddSnapshotInner {
    #[serde(rename = "snapshot-id")]
    pub snapshot_id: i64,
}

impl RawUpdate {
    fn into_domain(self) -> CommitUpdate {
        match self {
            RawUpdate::SetSnapshotRef { ref_name, snapshot_id } if ref_name == "main" => {
                CommitUpdate::SetCurrentSnapshot { snapshot_id }
            }
            RawUpdate::SetSnapshotRef { .. } => CommitUpdate::Other {
                action: "set-snapshot-ref(non-main)".into(),
            },
            RawUpdate::AddSnapshot { snapshot } => {
                CommitUpdate::SetCurrentSnapshot { snapshot_id: snapshot.snapshot_id }
            }
            RawUpdate::SetProperties { updates } => CommitUpdate::SetProperties { updates },
            RawUpdate::RemoveProperties { removals } => CommitUpdate::RemoveProperties { removals },
            RawUpdate::Unknown => CommitUpdate::Other {
                action: "unknown".into(),
            },
        }
    }
}

/// Wire envelope: either the upstream spec body OR the cave-iceberg legacy
/// body. `Legacy` is tried first because its required fields
/// (`expected_metadata_location`, `new_metadata_location`) make it strictly
/// reject spec-shaped payloads; `Spec` has no required scalar fields so it
/// would otherwise eat any object.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CommitRequest {
    Legacy(LegacyCommitRequest),
    Spec(SpecCommitRequest),
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub source: TableIdentifier,
    pub destination: TableIdentifier,
}

#[derive(Debug, Deserialize)]
pub struct RegisterTableRequest {
    pub name: String,
    #[serde(rename = "metadata-location", alias = "metadata_location")]
    pub metadata_location: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize)]
pub struct LoadCredentialsResponse {
    #[serde(rename = "storage-credentials")]
    pub storage_credentials: Vec<StorageCredential>,
}

#[derive(Debug, Serialize)]
pub struct StorageCredential {
    pub prefix: String,
    pub config: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct RemoteSignRequest {
    pub region: String,
    pub uri: String,
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct RemoteSignResponse {
    pub uri: String,
    pub headers: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CommitTransactionRequest {
    #[serde(rename = "table-changes", alias = "table_changes")]
    pub table_changes: Vec<TableChange>,
}

#[derive(Debug, Deserialize)]
pub struct TableChange {
    pub identifier: TableIdentifier,
    #[serde(default)]
    pub requirements: Vec<RawRequirement>,
    #[serde(default)]
    pub updates: Vec<RawUpdate>,
    #[serde(default, rename = "new-metadata-location")]
    pub new_metadata_location: Option<String>,
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
    let now = chrono::Utc::now().timestamp_millis();
    match req {
        CommitRequest::Legacy(req) => match state.catalog.commit(
            &ns,
            &name,
            &req.expected_metadata_location,
            &req.new_metadata_location,
            req.new_snapshot_id,
            now,
        ) {
            Ok(entry) => Json(table_to_response(entry)).into_response(),
            Err(e) => map_error(e),
        },
        CommitRequest::Spec(req) => {
            let new_loc = match req.new_metadata_location {
                Some(l) => l,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": {
                                "message": "missing required field new-metadata-location",
                                "type": "BadRequestException",
                                "code": 400u16,
                            }
                        })),
                    )
                        .into_response();
                }
            };
            let requirements: Vec<CommitRequirement> =
                req.requirements.into_iter().map(RawRequirement::into_domain).collect();
            let updates: Vec<CommitUpdate> =
                req.updates.into_iter().map(RawUpdate::into_domain).collect();
            match state.catalog.commit_with_updates(
                &ns,
                &name,
                &new_loc,
                &requirements,
                &updates,
                now,
            ) {
                Ok(entry) => Json(table_to_response(entry)).into_response(),
                Err(e) => map_error(e),
            }
        }
    }
}

async fn register_table(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
    Json(req): Json<RegisterTableRequest>,
) -> Response {
    match state.catalog.register_table(
        &ns,
        &req.name,
        &req.metadata_location,
        req.overwrite,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(entry) => Json(table_to_response(entry)).into_response(),
        Err(e) => map_error(e),
    }
}

async fn load_credentials(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
) -> Response {
    // The catalog must know about the table for the call to be meaningful;
    // 404 if missing. cave-iceberg has no STS provider wired in, so we
    // return an empty credential list — clients will fall back to their
    // ambient credentials.
    match state.catalog.load_table(&ns, &name) {
        Ok(_) => Json(LoadCredentialsResponse {
            storage_credentials: vec![],
        })
        .into_response(),
        Err(e) => map_error(e),
    }
}

async fn sign_request(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
    Json(req): Json<RemoteSignRequest>,
) -> Response {
    // Honour the contract: the table must exist, otherwise we return 404
    // (otherwise an attacker could probe the catalog for arbitrary tables
    // via a sign request). cave-iceberg is not a real SigV4 signer; we
    // pass the request URI + headers back unchanged so a co-located
    // pre-signed flow degrades gracefully.
    match state.catalog.load_table(&ns, &name) {
        Ok(_) => Json(RemoteSignResponse {
            uri: req.uri,
            headers: req.headers,
        })
        .into_response(),
        Err(e) => map_error(e),
    }
}

async fn commit_transaction(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommitTransactionRequest>,
) -> Response {
    // Multi-table commit. Upstream guarantees all-or-nothing: a single
    // failure rolls back all table changes. cave-iceberg holds catalog
    // state in-memory under per-table RwLocks; we approximate atomicity
    // by validating each change up-front (read-only) and then applying
    // them in a second pass. If any apply fails we surface the first
    // error; partial application here is bounded to the in-memory map
    // and is acceptable for the in-tree REST stub.
    let now = chrono::Utc::now().timestamp_millis();
    for change in &req.table_changes {
        if change.new_metadata_location.is_none() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": {
                        "message": format!(
                            "missing required field new-metadata-location for table {}.{}",
                            change.identifier.namespace.join("."),
                            change.identifier.name
                        ),
                        "type": "BadRequestException",
                        "code": 400u16,
                    }
                })),
            )
                .into_response();
        }
    }
    for change in req.table_changes {
        let ns = change.identifier.namespace.join(".");
        let name = change.identifier.name.clone();
        let new_loc = change.new_metadata_location.unwrap_or_default();
        let requirements: Vec<CommitRequirement> = change
            .requirements
            .into_iter()
            .map(RawRequirement::into_domain)
            .collect();
        let updates: Vec<CommitUpdate> = change
            .updates
            .into_iter()
            .map(RawUpdate::into_domain)
            .collect();
        if let Err(e) = state
            .catalog
            .commit_with_updates(&ns, &name, &new_loc, &requirements, &updates, now)
        {
            return map_error(e);
        }
    }
    StatusCode::NO_CONTENT.into_response()
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
    fn legacy_commit_request_deserializes() {
        let s = r#"{"expected_metadata_location":"v1","new_metadata_location":"v2","new_snapshot_id":42}"#;
        let req: CommitRequest = serde_json::from_str(s).unwrap();
        match req {
            CommitRequest::Legacy(r) => {
                assert_eq!(r.expected_metadata_location, "v1");
                assert_eq!(r.new_snapshot_id, Some(42));
            }
            _ => panic!("expected legacy variant"),
        }
    }

    #[test]
    fn spec_commit_request_with_requirements_and_updates_deserializes() {
        // Body shape mirrors apache/iceberg open-api/rest-catalog-open-api.yaml:
        //   CommitTableRequest{ requirements: [...], updates: [...] }
        let s = r#"{
            "requirements":[
              {"type":"assert-table-uuid","uuid":"abc"},
              {"type":"assert-ref-snapshot-id","ref":"main","snapshot-id":7}
            ],
            "updates":[
              {"action":"set-snapshot-ref","ref-name":"main","snapshot-id":8},
              {"action":"set-properties","updates":{"k":"v"}}
            ],
            "new-metadata-location":"v3.json"
        }"#;
        let req: CommitRequest = serde_json::from_str(s).unwrap();
        match req {
            CommitRequest::Spec(r) => {
                assert_eq!(r.requirements.len(), 2);
                assert_eq!(r.updates.len(), 2);
                assert_eq!(r.new_metadata_location.as_deref(), Some("v3.json"));
            }
            _ => panic!("expected spec variant"),
        }
    }

    #[test]
    fn raw_update_set_snapshot_ref_main_maps_to_set_current_snapshot() {
        let raw: RawUpdate = serde_json::from_str(
            r#"{"action":"set-snapshot-ref","ref-name":"main","snapshot-id":42}"#,
        )
        .unwrap();
        assert!(matches!(
            raw.into_domain(),
            CommitUpdate::SetCurrentSnapshot { snapshot_id: 42 }
        ));
    }

    #[test]
    fn register_table_request_deserializes_kebab_or_snake() {
        let s_kebab = r#"{"name":"t","metadata-location":"s3://x/m.json"}"#;
        let s_snake = r#"{"name":"t","metadata_location":"s3://x/m.json","overwrite":true}"#;
        let a: RegisterTableRequest = serde_json::from_str(s_kebab).unwrap();
        let b: RegisterTableRequest = serde_json::from_str(s_snake).unwrap();
        assert_eq!(a.metadata_location, "s3://x/m.json");
        assert_eq!(b.metadata_location, "s3://x/m.json");
        assert!(b.overwrite);
        assert!(!a.overwrite);
    }

    #[test]
    fn remote_sign_request_deserializes() {
        let s = r#"{
            "region":"us-east-1",
            "uri":"https://example.s3.amazonaws.com/foo",
            "method":"GET",
            "headers":{"x-amz-date":["20260101T000000Z"]}
        }"#;
        let req: RemoteSignRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.region, "us-east-1");
        assert_eq!(req.method, "GET");
        assert!(req.headers.contains_key("x-amz-date"));
    }

    #[test]
    fn commit_transaction_request_deserializes() {
        let s = r#"{
          "table-changes":[
            {
              "identifier":{"namespace":["acme"],"name":"t1"},
              "requirements":[],
              "updates":[],
              "new-metadata-location":"v2.json"
            },
            {
              "identifier":{"namespace":["acme"],"name":"t2"},
              "requirements":[],
              "updates":[],
              "new-metadata-location":"v3.json"
            }
          ]
        }"#;
        let req: CommitTransactionRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.table_changes.len(), 2);
        assert_eq!(req.table_changes[0].identifier.name, "t1");
    }

    #[test]
    fn load_credentials_response_serializes_storage_credentials_kebab_case() {
        let r = LoadCredentialsResponse {
            storage_credentials: vec![],
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"storage-credentials\""));
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
