//! HTTP REST API for cave-streams — REST proxy + admin endpoints.
//!
//! All endpoints are prefixed with `/api/v1/streams`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::admin::AdminClient;
use crate::connect::ConnectorRegistry;
use crate::error::{StreamError, StreamResult};
use crate::models::{
    AggregationType, CleanupPolicy, CompatibilityMode, ConnectorConfig, ConnectorDirection,
    ConnectorStatus, Header, PartitionerStrategy, ProducerRecord, Schema, SchemaType,
    StorageTierConfig, StreamOperation, StreamPipelineConfig,
};
use crate::producer::{Producer, ProducerRecordBuilder};
use crate::schema_registry::SchemaRegistry;
use crate::storage::{MemoryStorage, StreamStorage};
use crate::streams_api::{PipelineRegistry, StreamPipelineBuilder};
use crate::topic::TopicConfigPatch;

// ─── App state ────────────────────────────────────────────────────────────────

/// Shared state injected into all route handlers.
#[derive(Clone)]
pub struct StreamsState {
    pub storage: MemoryStorage,
}

impl Default for StreamsState {
    fn default() -> Self {
        Self {
            storage: MemoryStorage::new(),
        }
    }
}

type AppState = Arc<StreamsState>;

// ─── Error response ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    code: u16,
}

struct ApiError(StreamError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status_code())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = Json(ErrorBody {
            error: self.0.to_string(),
            code: status.as_u16(),
        });
        (status, body).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn ok<T: Serialize>(v: T) -> ApiResult<T> {
    Ok(Json(v))
}

fn wrap<T: Serialize>(r: StreamResult<T>) -> ApiResult<T> {
    r.map(Json).map_err(ApiError)
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        // Health
        .route("/health", get(health))
        // Topics
        .route("/topics", post(create_topic).get(list_topics))
        .route("/topics/{name}", get(get_topic).delete(delete_topic))
        .route("/topics/{name}/config", put(alter_topic_config))
        .route("/topics/{name}/partitions", put(add_partitions))
        .route("/topics/{name}/watermarks", get(get_watermarks))
        // Produce / Consume (REST proxy)
        .route("/topics/{name}/records", post(produce_record))
        .route("/topics/{name}/records/{partition}", get(fetch_records))
        // Consumer groups
        .route("/groups", get(list_groups))
        .route("/groups/{group}", get(describe_group).delete(delete_group))
        .route("/groups/{group}/offsets/{topic}", put(reset_offsets))
        .route(
            "/groups/:group/offsets/:topic/:partition",
            get(get_offset).post(commit_offset),
        )
        // Schema registry
        .route("/schemas", post(register_schema).get(list_subjects))
        .route("/schemas/check", post(check_schema_compat))
        .route("/schemas/id/{id}", get(get_schema_by_id))
        .route(
            "/schemas/:subject",
            get(get_latest_schema).delete(delete_subject),
        )
        .route(
            "/schemas/:subject/versions",
            get(list_schema_versions).post(register_schema_for_subject),
        )
        .route(
            "/schemas/:subject/versions/:version",
            get(get_schema_version),
        )
        .route("/schemas/{subject}/compat", put(set_compat).get(get_compat))
        // Connectors
        .route("/connectors", post(create_connector).get(list_connectors))
        .route(
            "/connectors/:name",
            get(get_connector).delete(delete_connector),
        )
        .route("/connectors/{name}/pause", put(pause_connector))
        .route("/connectors/{name}/resume", put(resume_connector))
        // Pipelines (Streams API)
        .route("/pipelines", post(create_pipeline).get(list_pipelines))
        .route(
            "/pipelines/:id",
            get(get_pipeline).delete(delete_pipeline),
        )
        .route("/pipelines/{id}/start", put(start_pipeline))
        .route("/pipelines/{id}/pause", put(pause_pipeline))
        .route("/pipelines/{id}/stop", put(stop_pipeline))
        // Admin
        .route("/admin/cluster", get(cluster_info))
        .route("/admin/compact", post(run_compaction))
        .route("/admin/retention", post(enforce_retention))
        .route("/admin/tiers", get(get_tier_config).put(set_tier_config))
        .with_state(state)
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "cave-streams",
        "version": env!("CARGO_PKG_VERSION"),
        "features": [
            "topics", "partitions", "consumer-groups", "schema-registry",
            "exactly-once", "log-compaction", "tiered-storage",
            "kafka-protocol", "connect-api", "streams-api"
        ]
    }))
}

// ─── Topics ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateTopicRequest {
    name: String,
    partitions: u32,
    replication_factor: Option<u16>,
    retention_ms: Option<i64>,
    retention_bytes: Option<i64>,
    cleanup_policy: Option<CleanupPolicy>,
    max_message_bytes: Option<usize>,
}

async fn create_topic(
    State(state): State<AppState>,
    Json(req): Json<CreateTopicRequest>,
) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let mut config = crate::models::TopicConfig::default();
    if let Some(v) = req.retention_ms {
        config.retention_ms = Some(v);
    }
    if let Some(v) = req.retention_bytes {
        config.retention_bytes = Some(v);
    }
    if let Some(v) = req.cleanup_policy {
        config.cleanup_policy = v;
    }
    if let Some(v) = req.max_message_bytes {
        config.max_message_bytes = v;
    }
    let topic = admin
        .create_topic(
            &req.name,
            req.partitions,
            req.replication_factor.unwrap_or(1),
            Some(config),
        )
        .map_err(ApiError)?;
    ok(serde_json::json!({
        "name": topic.name,
        "partitions": topic.partitions,
        "replication_factor": topic.replication_factor,
        "created_at": topic.created_at,
    }))
}

async fn list_topics(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let topics = state.storage.list_topics().map_err(ApiError)?;
    ok(serde_json::json!({ "topics": topics }))
}

async fn get_topic(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let topic = state
        .storage
        .get_topic(&name)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(StreamError::TopicNotFound(name.clone())))?;
    ok(serde_json::to_value(topic).unwrap_or_default())
}

async fn delete_topic(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    state.storage.delete_topic(&name).map_err(ApiError)?;
    ok(serde_json::json!({ "deleted": name }))
}

async fn alter_topic_config(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(patch): Json<TopicConfigPatch>,
) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let topic = admin.alter_topic_config(&name, patch).map_err(ApiError)?;
    ok(serde_json::to_value(topic).unwrap_or_default())
}

#[derive(Deserialize)]
struct AddPartitionsRequest {
    new_total: u32,
}

async fn add_partitions(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<AddPartitionsRequest>,
) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let topic = admin.add_partitions(&name, req.new_total).map_err(ApiError)?;
    ok(serde_json::json!({ "name": topic.name, "partitions": topic.partitions }))
}

async fn get_watermarks(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let watermarks = admin.topic_watermarks(&name).map_err(ApiError)?;
    ok(serde_json::json!({ "topic": name, "watermarks": watermarks }))
}

// ─── Produce / Consume ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProduceRequest {
    key: Option<String>,
    value: Option<String>,
    headers: Option<Vec<(String, String)>>,
    timestamp_ms: Option<i64>,
    partition: Option<u32>,
}

async fn produce_record(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    Json(req): Json<ProduceRequest>,
) -> ApiResult<serde_json::Value> {
    let producer = Producer::new(state.storage.clone()).map_err(ApiError)?;

    let partitioner = req
        .partition
        .map(PartitionerStrategy::Manual)
        .unwrap_or(PartitionerStrategy::KeyHash);

    let mut builder = ProducerRecordBuilder::new(&topic)
        .partitioner(partitioner);

    if let Some(k) = req.key {
        builder = builder.key(k.into_bytes());
    }
    if let Some(v) = req.value {
        builder = builder.value(v.into_bytes());
    }
    if let Some(ts) = req.timestamp_ms {
        builder = builder.timestamp_ms(ts);
    }
    if let Some(hdrs) = req.headers {
        for (k, v) in hdrs {
            builder = builder.header(k, v.into_bytes());
        }
    }

    let meta = producer.send(builder.build()).map_err(ApiError)?;
    ok(serde_json::json!({
        "topic": meta.topic,
        "partition": meta.partition,
        "offset": meta.offset,
        "timestamp_ms": meta.timestamp_ms,
    }))
}

#[derive(Deserialize)]
struct FetchQuery {
    offset: Option<i64>,
    max_records: Option<usize>,
}

async fn fetch_records(
    State(state): State<AppState>,
    Path((topic, partition)): Path<(String, u32)>,
    Query(q): Query<FetchQuery>,
) -> ApiResult<serde_json::Value> {
    let offset = q.offset.unwrap_or(0);
    let max = q.max_records.unwrap_or(100);
    let records = state
        .storage
        .fetch_from_partition(&topic, partition, offset, max)
        .map_err(ApiError)?;

    let out: Vec<_> = records
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "offset": r.offset,
                "partition": r.partition,
                "timestamp_ms": r.timestamp_ms,
                "key": r.key.as_deref().map(|k| String::from_utf8_lossy(k).to_string()),
                "value": r.value.as_deref().map(|v| String::from_utf8_lossy(v).to_string()),
                "headers": r.headers.iter().map(|h| serde_json::json!({
                    "key": h.key,
                    "value": String::from_utf8_lossy(&h.value),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    ok(serde_json::json!({ "records": out }))
}

// ─── Consumer groups ──────────────────────────────────────────────────────────

async fn list_groups(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let groups = state.storage.list_groups().map_err(ApiError)?;
    ok(serde_json::json!({ "groups": groups }))
}

async fn describe_group(
    State(state): State<AppState>,
    Path(group): Path<String>,
) -> ApiResult<serde_json::Value> {
    let g = state
        .storage
        .get_group(&group)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(StreamError::GroupNotFound(group.clone())))?;
    ok(serde_json::to_value(g).unwrap_or_default())
}

async fn delete_group(
    State(state): State<AppState>,
    Path(group): Path<String>,
) -> ApiResult<serde_json::Value> {
    state.storage.delete_group(&group).map_err(ApiError)?;
    ok(serde_json::json!({ "deleted": group }))
}

#[derive(Deserialize)]
struct ResetOffsetsQuery {
    position: Option<String>,
}

async fn reset_offsets(
    State(state): State<AppState>,
    Path((group, topic)): Path<(String, String)>,
    Query(q): Query<ResetOffsetsQuery>,
) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    match q.position.as_deref().unwrap_or("earliest") {
        "latest" => admin.reset_offsets_latest(&group, &topic).map_err(ApiError)?,
        _ => admin.reset_offsets_earliest(&group, &topic).map_err(ApiError)?,
    }
    ok(serde_json::json!({ "group": group, "topic": topic, "reset": "ok" }))
}

async fn get_offset(
    State(state): State<AppState>,
    Path((group, topic, partition)): Path<(String, String, u32)>,
) -> ApiResult<serde_json::Value> {
    let offset = state
        .storage
        .get_offset(&group, &topic, partition)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "group": group, "topic": topic, "partition": partition, "offset": offset }))
}

#[derive(Deserialize)]
struct CommitOffsetRequest {
    offset: i64,
}

async fn commit_offset(
    State(state): State<AppState>,
    Path((group, topic, partition)): Path<(String, String, u32)>,
    Json(req): Json<CommitOffsetRequest>,
) -> ApiResult<serde_json::Value> {
    state
        .storage
        .commit_offset(&group, &topic, partition, req.offset)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "committed": req.offset }))
}

// ─── Schema registry ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterSchemaRequest {
    subject: String,
    schema_type: SchemaType,
    definition: String,
}

async fn register_schema(
    State(state): State<AppState>,
    Json(req): Json<RegisterSchemaRequest>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let id = registry
        .register(req.subject, req.schema_type, req.definition)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "id": id }))
}

#[derive(Deserialize)]
struct RegisterSchemaForSubjectRequest {
    schema_type: SchemaType,
    definition: String,
}

async fn register_schema_for_subject(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Json(req): Json<RegisterSchemaForSubjectRequest>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let id = registry
        .register(subject, req.schema_type, req.definition)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "id": id }))
}

async fn list_subjects(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let subjects = registry.list_subjects().map_err(ApiError)?;
    ok(serde_json::json!({ "subjects": subjects }))
}

async fn get_schema_by_id(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let schema = registry.get_by_id(id).map_err(ApiError)?;
    ok(serde_json::to_value(schema).unwrap_or_default())
}

async fn get_latest_schema(
    State(state): State<AppState>,
    Path(subject): Path<String>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let schema = registry.get_latest(&subject).map_err(ApiError)?;
    ok(serde_json::to_value(schema).unwrap_or_default())
}

async fn delete_subject(
    State(state): State<AppState>,
    Path(subject): Path<String>,
) -> ApiResult<serde_json::Value> {
    let versions = state
        .storage
        .list_subject_versions(&subject)
        .map_err(ApiError)?;
    let count = versions.len();
    for v in versions {
        state.storage.delete_schema(&subject, v).map_err(ApiError)?;
    }
    ok(serde_json::json!({ "subject": subject, "versions_deleted": count }))
}

async fn list_schema_versions(
    State(state): State<AppState>,
    Path(subject): Path<String>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let versions = registry.list_versions(&subject).map_err(ApiError)?;
    ok(serde_json::json!({ "subject": subject, "versions": versions }))
}

async fn get_schema_version(
    State(state): State<AppState>,
    Path((subject, version)): Path<(String, u32)>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let schema = registry.get_version(&subject, version).map_err(ApiError)?;
    ok(serde_json::to_value(schema).unwrap_or_default())
}

#[derive(Deserialize)]
struct CheckCompatRequest {
    schema_type: SchemaType,
    definition: String,
}

async fn check_schema_compat(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Json(req): Json<CheckCompatRequest>,
) -> ApiResult<serde_json::Value> {
    let registry = SchemaRegistry::new(state.storage.clone());
    let result = registry
        .check_compatibility(&subject, &req.schema_type, &req.definition)
        .map_err(ApiError)?;
    ok(serde_json::json!({
        "compatible": result.compatible,
        "messages": result.messages,
    }))
}

#[derive(Deserialize)]
struct SetCompatRequest {
    mode: CompatibilityMode,
}

async fn set_compat(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Json(req): Json<SetCompatRequest>,
) -> ApiResult<serde_json::Value> {
    state
        .storage
        .set_subject_compat(&subject, req.mode)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "subject": subject, "compat": "updated" }))
}

async fn get_compat(
    State(state): State<AppState>,
    Path(subject): Path<String>,
) -> ApiResult<serde_json::Value> {
    let mode = state
        .storage
        .get_subject_compat(&subject)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "subject": subject, "compatibility": mode }))
}

// ─── Connectors ───────────────────────────────────────────────────────────────

async fn create_connector(
    State(state): State<AppState>,
    Json(cfg): Json<ConnectorConfig>,
) -> ApiResult<serde_json::Value> {
    let registry = ConnectorRegistry::new(state.storage.clone());
    let created = registry.create(cfg).map_err(ApiError)?;
    ok(serde_json::to_value(created).unwrap_or_default())
}

async fn list_connectors(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let connectors = state.storage.list_connectors().map_err(ApiError)?;
    ok(serde_json::json!({ "connectors": connectors }))
}

async fn get_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let cfg = state
        .storage
        .get_connector(&name)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(StreamError::ConnectorNotFound(name.clone())))?;
    ok(serde_json::to_value(cfg).unwrap_or_default())
}

async fn delete_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    state.storage.delete_connector(&name).map_err(ApiError)?;
    ok(serde_json::json!({ "deleted": name }))
}

async fn pause_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let registry = ConnectorRegistry::new(state.storage.clone());
    let cfg = registry.pause(&name).map_err(ApiError)?;
    ok(serde_json::json!({ "name": cfg.name, "status": cfg.status }))
}

async fn resume_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let registry = ConnectorRegistry::new(state.storage.clone());
    let cfg = registry.resume(&name).map_err(ApiError)?;
    ok(serde_json::json!({ "name": cfg.name, "status": cfg.status }))
}

// ─── Pipelines ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreatePipelineRequest {
    name: Option<String>,
    source_topic: String,
    sink_topic: Option<String>,
    operations: Vec<StreamOperation>,
}

async fn create_pipeline(
    State(state): State<AppState>,
    Json(req): Json<CreatePipelineRequest>,
) -> ApiResult<serde_json::Value> {
    let registry = PipelineRegistry::new(state.storage.clone());
    let mut cfg = StreamPipelineConfig {
        id: Uuid::new_v4(),
        name: req.name.unwrap_or_else(|| format!("pipeline-{}", Uuid::new_v4())),
        source_topic: req.source_topic,
        sink_topic: req.sink_topic,
        operations: req.operations,
        state: crate::models::PipelineState::Created,
    };
    let created = registry.create(cfg).map_err(ApiError)?;
    ok(serde_json::to_value(created).unwrap_or_default())
}

async fn list_pipelines(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let pipelines = state.storage.list_pipelines().map_err(ApiError)?;
    ok(serde_json::json!({ "pipelines": pipelines }))
}

async fn get_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let pipeline = state
        .storage
        .get_pipeline(id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(StreamError::PipelineNotFound(id.to_string())))?;
    ok(serde_json::to_value(pipeline).unwrap_or_default())
}

async fn delete_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    state.storage.delete_pipeline(id).map_err(ApiError)?;
    ok(serde_json::json!({ "deleted": id }))
}

async fn start_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    PipelineRegistry::new(state.storage.clone())
        .start(id)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "id": id, "state": "running" }))
}

async fn pause_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    PipelineRegistry::new(state.storage.clone())
        .pause(id)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "id": id, "state": "paused" }))
}

async fn stop_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    PipelineRegistry::new(state.storage.clone())
        .stop(id)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "id": id, "state": "stopped" }))
}

// ─── Admin ────────────────────────────────────────────────────────────────────

async fn cluster_info(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let info = admin.cluster_info();
    ok(serde_json::to_value(info).unwrap_or_default())
}

async fn run_compaction(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let stats = admin.run_compaction().map_err(ApiError)?;
    ok(serde_json::json!({
        "partitions_compacted": stats.partitions_compacted,
        "records_before": stats.records_before,
        "records_after": stats.records_after,
        "bytes_reclaimed": stats.bytes_reclaimed,
    }))
}

async fn enforce_retention(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let admin = AdminClient::new(state.storage.clone());
    let stats = admin.enforce_retention().map_err(ApiError)?;
    ok(serde_json::json!({
        "partitions_trimmed": stats.partitions_trimmed,
        "records_deleted": stats.records_deleted,
    }))
}

async fn get_tier_config(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let cfg = state.storage.get_tier_config().map_err(ApiError)?;
    ok(serde_json::to_value(cfg).unwrap_or_default())
}

async fn set_tier_config(
    State(state): State<AppState>,
    Json(cfg): Json<StorageTierConfig>,
) -> ApiResult<serde_json::Value> {
    state.storage.set_tier_config(cfg).map_err(ApiError)?;
    ok(serde_json::json!({ "updated": true }))
}

// ─── Compatibility shim for missing subject in check endpoint ─────────────────

// The /schemas/check route uses Path(subject) but the JSON body contains
// everything; route the handler with subject from body.
#[allow(dead_code)]
async fn check_compat_no_path(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> ApiResult<serde_json::Value> {
    let subject = req["subject"].as_str().unwrap_or("").to_string();
    let definition = req["definition"].as_str().unwrap_or("").to_string();
    let registry = SchemaRegistry::new(state.storage.clone());
    let result = registry
        .check_compatibility(&subject, &SchemaType::JsonSchema, &definition)
        .map_err(ApiError)?;
    ok(serde_json::json!({ "compatible": result.compatible }))
}
