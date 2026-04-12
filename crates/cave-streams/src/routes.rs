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
        .route("/topics/:name", get(get_topic).delete(delete_topic))
        .route("/topics/:name/config", put(alter_topic_config))
        .route("/topics/:name/partitions", put(add_partitions))
        .route("/topics/:name/watermarks", get(get_watermarks))
        // Produce / Consume (REST proxy)
        .route("/topics/:name/records", post(produce_record))
        .route("/topics/:name/records/:partition", get(fetch_records))
        // Consumer groups
        .route("/groups", get(list_groups))
        .route("/groups/:group", get(describe_group).delete(delete_group))
        .route("/groups/:group/offsets/:topic", put(reset_offsets))
        .route(
            "/groups/:group/offsets/:topic/:partition",
            get(get_offset).post(commit_offset),
        )
        // Schema registry
        .route("/schemas", post(register_schema).get(list_subjects))
        .route("/schemas/check", post(check_schema_compat))
        .route("/schemas/id/:id", get(get_schema_by_id))
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
        .route("/schemas/:subject/compat", put(set_compat).get(get_compat))
        // Connectors
        .route("/connectors", post(create_connector).get(list_connectors))
        .route(
            "/connectors/:name",
            get(get_connector).delete(delete_connector),
        )
        .route("/connectors/:name/pause", put(pause_connector))
        .route("/connectors/:name/resume", put(resume_connector))
        // Pipelines (Streams API)
        .route("/pipelines", post(create_pipeline).get(list_pipelines))
        .route(
            "/pipelines/:id",
            get(get_pipeline).delete(delete_pipeline),
        )
        .route("/pipelines/:id/start", put(start_pipeline))
        .route("/pipelines/:id/pause", put(pause_pipeline))
        .route("/pipelines/:id/stop", put(stop_pipeline))
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
//! HTTP routes for cave-streams — all under /api/v1/streams (and /api/v1/*).
use crate::{models::*, StreamsState};
    routing::{get, post, put},
use chrono::Utc;
use serde::Deserialize;
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;
type StatusResult<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<serde_json::Value>)>;
fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
fn bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
pub fn create_router(state: Arc<StreamsState>) -> Router {
        // ── Health ────────────────────────────────────────────────────────
        .route("/api/v1/streams/health", get(health))
        // ── Streams ───────────────────────────────────────────────────────
        // NOTE: literal sub-paths (/health, /backpressure-all) are registered
        // before the /:id wildcard so axum prefers them in matching.
        .route("/api/v1/streams", post(create_stream).get(list_streams))
            "/api/v1/streams/:id",
            get(get_stream).delete(delete_stream),
        .route("/api/v1/streams/:id/stats", get(stream_stats))
        .route("/api/v1/streams/:id/throttle", put(throttle_stream))
        // ── Subscriptions (nested) ────────────────────────────────────────
            "/api/v1/streams/:stream_id/subscriptions",
            post(create_subscription).get(list_subscriptions),
            "/api/v1/streams/:stream_id/subscriptions/:sub_id",
            get(get_subscription).delete(delete_subscription),
            "/api/v1/streams/:stream_id/subscriptions/:sub_id/pause",
            put(pause_subscription),
            "/api/v1/streams/:stream_id/subscriptions/:sub_id/resume",
            put(resume_subscription),
        // ── Messages ──────────────────────────────────────────────────────
        .route("/api/v1/streams/:stream_id/publish", post(publish_message))
        .route("/api/v1/streams/:stream_id/pull", post(pull_messages))
            "/api/v1/streams/:stream_id/subscriptions/:sub_id/ack",
            post(ack_messages),
        // ── Schema Registry ───────────────────────────────────────────────
        .route("/api/v1/schemas", post(register_schema).get(list_schemas))
            "/api/v1/schemas/validate",
            post(validate_schema),
            "/api/v1/schemas/:id",
            get(get_schema).delete(delete_schema),
        // ── Connectors ────────────────────────────────────────────────────
            "/api/v1/connectors",
            post(create_connector).get(list_connectors),
            "/api/v1/connectors/:id",
            get(get_connector)
                .patch(patch_connector)
                .delete(delete_connector),
        // ── Dead Letter Queue ─────────────────────────────────────────────
        .route("/api/v1/dlq", get(list_dlq))
        .route("/api/v1/dlq/:id", get(get_dlq_entry).delete(discard_dlq_entry))
        .route("/api/v1/dlq/:id/retry", post(retry_dlq_entry))
        // ── Tiered Storage ────────────────────────────────────────────────
            "/api/v1/storage/config",
            get(get_storage_config).put(update_storage_config),
        .route("/api/v1/storage/tiers", get(storage_tier_stats))
        // ── Metrics ───────────────────────────────────────────────────────
        .route("/api/v1/streams/:id/metrics", get(stream_metrics))
        .route("/api/v1/metrics", get(platform_metrics))
        // ── Backpressure ──────────────────────────────────────────────────
        .route("/api/v1/backpressure", get(backpressure_status))
// ── Health ────────────────────────────────────────────────────────────────────
        "module": "cave-streams",
        "replaces": [
            "Apache Kafka",
            "Confluent Platform",
            "Schema Registry",
            "Kafka Connect",
            "Kafka Streams",
            "NATS JetStream",
            "Pulsar"
        ],
        "features": {
            "tiered_storage": true,
            "exactly_once": true,
            "built_in_dlq": true,
            "built_in_schema_registry": true,
            "partition_less": true,
            "jvm_free": true
// ── Streams ───────────────────────────────────────────────────────────────────
async fn create_stream(
    State(state): State<Arc<StreamsState>>,
    Json(req): Json<CreateStreamRequest>,
) -> (StatusCode, Json<Stream>) {
    let stream = Stream {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace.unwrap_or_else(|| "default".to_string()),
        description: req.description,
        storage_tier: req.storage_tier.unwrap_or_default(),
        retention: req.retention.unwrap_or_default(),
        schema_id: req.schema_id,
        max_message_size_bytes: req.max_message_size_bytes.unwrap_or(1024 * 1024),
        throughput_limit: req.throughput_limit,
        subscriber_count: 0,
        message_count: 0,
        sequence: 0,
        labels: req.labels.unwrap_or_default(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    let mut store = state.store.lock().unwrap();
    store.streams.insert(stream.id, stream.clone());
    (StatusCode::CREATED, Json(stream))
struct NamespaceFilter {
    namespace: Option<String>,
    label: Option<String>,
async fn list_streams(
    State(state): State<Arc<StreamsState>>,
    Query(filter): Query<NamespaceFilter>,
) -> Json<Vec<Stream>> {
    let store = state.store.lock().unwrap();
    let streams: Vec<Stream> = store
        .streams
        .values()
        .filter(|s| {
            filter.namespace.as_ref().map_or(true, |ns| &s.namespace == ns)
                && filter
                    .label
                    .as_ref()
                    .map_or(true, |kv| {
                        // accept "key=value" or just "key" (presence check)
                        if let Some((k, v)) = kv.split_once('=') {
                            s.labels.get(k).map(|val| val == v).unwrap_or(false)
                        } else {
                            s.labels.contains_key(kv.as_str())
                    })
        })
        .cloned()
        .collect();
    Json(streams)
async fn get_stream(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Stream> {
    let store = state.store.lock().unwrap();
    store
        .streams
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("stream not found"))
async fn delete_stream(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    let mut store = state.store.lock().unwrap();
    // Reject if active subscriptions exist.
    let has_subs = store.subscriptions.values().any(|s| s.stream_id == id);
    if has_subs {
        return Err(bad_request(
            "cannot delete stream with active subscriptions; delete subscriptions first",
        ));
    store
        .streams
        .remove(&id)
        .map(|s| Json(serde_json::json!({ "deleted": s.id, "name": s.name })))
        .ok_or_else(|| not_found("stream not found"))
async fn stream_stats(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StreamStats> {
    let store = state.store.lock().unwrap();
    let s = store
        .streams
        .get(&id)
        .ok_or_else(|| not_found("stream not found"))?;
    Ok(Json(StreamStats {
        stream_id: s.id,
        name: s.name.clone(),
        namespace: s.namespace.clone(),
        message_count: s.message_count,
        subscriber_count: s.subscriber_count,
        sequence: s.sequence,
        throughput_limit: s.throughput_limit.clone(),
        storage_tier: s.storage_tier.clone(),
        retention: s.retention.clone(),
async fn throttle_stream(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ThrottleRequest>,
) -> ApiResult<Stream> {
    let mut store = state.store.lock().unwrap();
    let stream = store
        .streams
        .get_mut(&id)
        .ok_or_else(|| not_found("stream not found"))?;
    stream.throughput_limit = Some(ThroughputLimit {
        messages_per_second: req.messages_per_second,
        bytes_per_second: req.bytes_per_second,
    stream.updated_at = Utc::now();
    Ok(Json(stream.clone()))
// ── Subscriptions ─────────────────────────────────────────────────────────────
async fn create_subscription(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> StatusResult<Subscription> {
    let mut store = state.store.lock().unwrap();
    if !store.streams.contains_key(&stream_id) {
        return Err(not_found("stream not found"));
    let cursor = match req.delivery_policy.as_ref().unwrap_or(&DeliveryPolicy::Latest) {
        DeliveryPolicy::Earliest => 0,
        DeliveryPolicy::BySequence(seq) => *seq,
        _ => store.stream_sequences.get(&stream_id).copied().unwrap_or(0),
    let sub = Subscription {
        id: Uuid::new_v4(),
        stream_id,
        name: req.name,
        subscription_type: req.subscription_type,
        delivery_policy: req.delivery_policy.unwrap_or_default(),
        retry_policy: req.retry_policy.unwrap_or_default(),
        dead_letter_stream: req.dead_letter_stream,
        filter_expression: req.filter_expression,
        ack_deadline_seconds: req.ack_deadline_seconds.unwrap_or(30),
        exactly_once: req.exactly_once.unwrap_or(false),
        consumer_count: 0,
        cursor,
        lag: 0,
        status: SubscriptionStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    store.subscriptions.insert(sub.id, sub.clone());
    store.refresh_subscriber_count(stream_id);
    Ok((StatusCode::CREATED, Json(sub)))
struct StreamIdFilter {
    stream_id: Option<Uuid>,
async fn list_subscriptions(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
) -> ApiResult<Vec<Subscription>> {
    let store = state.store.lock().unwrap();
    if !store.streams.contains_key(&stream_id) {
        return Err(not_found("stream not found"));
    let subs: Vec<Subscription> = store
        .subscriptions
        .values()
        .filter(|s| s.stream_id == stream_id)
        .cloned()
        .collect();
    Ok(Json(subs))
async fn get_subscription(
    State(state): State<Arc<StreamsState>>,
    Path((stream_id, sub_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Subscription> {
    let store = state.store.lock().unwrap();
    let sub = store
        .subscriptions
        .get(&sub_id)
        .ok_or_else(|| not_found("subscription not found"))?;
    if sub.stream_id != stream_id {
        return Err(not_found("subscription not found on this stream"));
    Ok(Json(sub.clone()))
async fn delete_subscription(
    State(state): State<Arc<StreamsState>>,
    Path((stream_id, sub_id)): Path<(Uuid, Uuid)>,
    let mut store = state.store.lock().unwrap();
    let sub = store
        .subscriptions
        .get(&sub_id)
        .ok_or_else(|| not_found("subscription not found"))?;
    if sub.stream_id != stream_id {
        return Err(not_found("subscription not found on this stream"));
    store.subscriptions.remove(&sub_id);
    store.refresh_subscriber_count(stream_id);
    Ok(Json(serde_json::json!({ "deleted": sub_id })))
async fn pause_subscription(
    State(state): State<Arc<StreamsState>>,
    Path((stream_id, sub_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Subscription> {
    let mut store = state.store.lock().unwrap();
    let sub = store
        .subscriptions
        .get_mut(&sub_id)
        .ok_or_else(|| not_found("subscription not found"))?;
    if sub.stream_id != stream_id {
        return Err(not_found("subscription not found on this stream"));
    sub.status = SubscriptionStatus::Paused;
    sub.updated_at = Utc::now();
    Ok(Json(sub.clone()))
async fn resume_subscription(
    State(state): State<Arc<StreamsState>>,
    Path((stream_id, sub_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Subscription> {
    let mut store = state.store.lock().unwrap();
    let sub = store
        .subscriptions
        .get_mut(&sub_id)
        .ok_or_else(|| not_found("subscription not found"))?;
    if sub.stream_id != stream_id {
        return Err(not_found("subscription not found on this stream"));
    sub.status = SubscriptionStatus::Active;
    sub.updated_at = Utc::now();
    Ok(Json(sub.clone()))
// ── Messages ──────────────────────────────────────────────────────────────────
async fn publish_message(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
    Json(req): Json<PublishRequest>,
) -> StatusResult<PublishResponse> {
    let mut store = state.store.lock().unwrap();
    if !store.streams.contains_key(&stream_id) {
        return Err(not_found("stream not found"));
    // Exactly-once deduplication check.
    if let Some(dedup_id) = req.deduplication_id {
        if store.is_duplicate(dedup_id) {
            let ts = Utc::now();
            let seq = store.stream_sequences.get(&stream_id).copied().unwrap_or(0);
            return Ok((
                StatusCode::OK,
                Json(PublishResponse {
                    message_id: dedup_id,
                    stream_id,
                    sequence: seq,
                    timestamp: ts,
                    storage_tier: StorageTierHint::Hot,
                    deduplicated: true,
                }),
            ));
    let seq = store.next_sequence(stream_id);
    let ts = Utc::now();
    let msg_id = Uuid::new_v4();
    let msg = Message {
        id: msg_id,
        stream_id,
        sequence: seq,
        timestamp: ts,
        key: req.key,
        payload: req.payload,
        headers: req.headers.unwrap_or_default(),
        schema_id: req.schema_id,
        storage_tier: StorageTierHint::Hot,
        delivery_count: 0,
    store.messages.push(msg);
    // Update stream counters.
    if let Some(stream) = store.streams.get_mut(&stream_id) {
        stream.message_count += 1;
        stream.sequence = seq;
        stream.updated_at = ts;
    Ok((
        StatusCode::CREATED,
        Json(PublishResponse {
            message_id: msg_id,
            stream_id,
            sequence: seq,
            timestamp: ts,
            storage_tier: StorageTierHint::Hot,
            deduplicated: false,
        }),
    ))
async fn pull_messages(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
    Json(req): Json<PullRequest>,
) -> ApiResult<Vec<Message>> {
    let store = state.store.lock().unwrap();
    let sub = store
        .subscriptions
        .get(&req.subscription_id)
        .ok_or_else(|| not_found("subscription not found"))?;
    if sub.stream_id != stream_id {
        return Err(bad_request("subscription does not belong to this stream"));
    let limit = req.max_messages.unwrap_or(100) as usize;
    let cursor = sub.cursor;
    let messages: Vec<Message> = store
        .messages
        .iter()
        .filter(|m| m.stream_id == stream_id && m.sequence > cursor)
        .take(limit)
        .cloned()
        .collect();
    Ok(Json(messages))
async fn ack_messages(
    State(state): State<Arc<StreamsState>>,
    Path((stream_id, sub_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<AckRequest>,
) -> ApiResult<AckResponse> {
    let mut store = state.store.lock().unwrap();
    // Validate ownership immutably first.
    {
        let sub = store
            .subscriptions
            .get(&sub_id)
            .ok_or_else(|| not_found("subscription not found"))?;
        if sub.stream_id != stream_id {
            return Err(not_found("subscription not found on this stream"));
    // Collect all immutable data before any mutation.
    let known_ids: std::collections::HashSet<Uuid> =
        store.messages.iter().map(|m| m.id).collect();
    let (acked, not_found_ids): (Vec<Uuid>, Vec<Uuid>) =
        req.message_ids.into_iter().partition(|id| known_ids.contains(id));
    let max_seq = store
        .messages
        .iter()
        .filter(|m| acked.contains(&m.id))
        .map(|m| m.sequence)
        .max()
        .unwrap_or(0);
    let stream_seq = store
        .stream_sequences
        .get(&stream_id)
        .copied()
        .unwrap_or(0);
    // Now mutate subscription in a single borrow.
    let sub = store.subscriptions.get_mut(&sub_id).unwrap();
    if max_seq > sub.cursor {
        sub.cursor = max_seq;
    let cursor = sub.cursor;
    sub.lag = stream_seq.saturating_sub(cursor);
    sub.updated_at = Utc::now();
    Ok(Json(AckResponse {
        acked,
        not_found: not_found_ids,
        cursor_advanced_to: cursor,
// ── Schema Registry ───────────────────────────────────────────────────────────
async fn register_schema(
    State(state): State<Arc<StreamsState>>,
    Json(req): Json<RegisterSchemaRequest>,
) -> (StatusCode, Json<Schema>) {
    let mut store = state.store.lock().unwrap();
    let fingerprint = crate::store::StreamsStore::fingerprint(&req.definition);
    let version = store.next_schema_version(&req.subject);
    let schema = Schema {
        id: Uuid::new_v4(),
        subject: req.subject,
        format: req.format,
        version,
        definition: req.definition,
        compatibility: req.compatibility.unwrap_or(CompatibilityMode::Backward),
        fingerprint,
        stream_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    store.schemas.insert(schema.id, schema.clone());
    (StatusCode::CREATED, Json(schema))
async fn list_schemas(
    State(state): State<Arc<StreamsState>>,
    Query(filter): Query<StreamIdFilter>,
) -> Json<Vec<Schema>> {
    let store = state.store.lock().unwrap();
    // If stream_id filter present, return only schemas used by that stream.
    let schemas: Vec<Schema> = if let Some(sid) = filter.stream_id {
        let schema_ids: std::collections::HashSet<Uuid> = store
            .streams
            .get(&sid)
            .and_then(|s| s.schema_id)
            .collect();
        store
            .schemas
            .values()
            .filter(|s| schema_ids.contains(&s.id))
            .cloned()
            .collect()
    } else {
        store.schemas.values().cloned().collect()
    Json(schemas)
async fn get_schema(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Schema> {
    let store = state.store.lock().unwrap();
    store
        .schemas
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("schema not found"))
async fn delete_schema(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    let mut store = state.store.lock().unwrap();
    let in_use = store.streams.values().any(|s| s.schema_id == Some(id));
    if in_use {
        return Err(bad_request(
            "schema is in use by one or more streams; update streams first",
        ));
    store
        .schemas
        .remove(&id)
        .map(|_| Json(serde_json::json!({ "deleted": id })))
        .ok_or_else(|| not_found("schema not found"))
async fn validate_schema(
    State(state): State<Arc<StreamsState>>,
    Json(req): Json<ValidateSchemaRequest>,
) -> Json<ValidateSchemaResponse> {
    let store = state.store.lock().unwrap();
    let existing = store.schemas.values().find(|s| s.subject == req.subject);
    let (compatible, errors) = match existing {
        None => (true, vec![]),
        Some(schema) => {
            let fp = crate::store::StreamsStore::fingerprint(&req.definition);
            if fp == schema.fingerprint {
                (true, vec![])
            } else {
                // Structural compatibility is a deep concern; surface a hint.
                let msg = format!(
                    "Definition differs from subject '{}' v{} (compatibility: {:?}). \
                     Full schema diffing requires format-specific tooling.",
                    schema.subject, schema.version, schema.compatibility
                );
                (false, vec![msg])
    Json(ValidateSchemaResponse {
        valid: true, // syntactic validity requires format-specific parse
        compatible,
        errors,
    })
// ── Connectors ────────────────────────────────────────────────────────────────
async fn create_connector(
    State(state): State<Arc<StreamsState>>,
    Json(req): Json<CreateConnectorRequest>,
) -> StatusResult<Connector> {
    let mut store = state.store.lock().unwrap();
    if !store.streams.contains_key(&req.stream_id) {
        return Err(not_found("stream not found"));
    let connector = Connector {
        id: Uuid::new_v4(),
        name: req.name,
        connector_class: req.connector_class,
        direction: req.direction,
        stream_id: req.stream_id,
        status: ConnectorStatus::Provisioning,
        config: req.config,
        transform_dsl: req.transform_dsl,
        error_msg: None,
        messages_processed: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    store.connectors.insert(connector.id, connector.clone());
    Ok((StatusCode::CREATED, Json(connector)))
async fn list_connectors(
    State(state): State<Arc<StreamsState>>,
    Query(filter): Query<StreamIdFilter>,
) -> Json<Vec<Connector>> {
    let store = state.store.lock().unwrap();
    let connectors: Vec<Connector> = store
        .connectors
        .values()
        .filter(|c| filter.stream_id.map_or(true, |id| c.stream_id == id))
        .cloned()
        .collect();
    Json(connectors)
async fn get_connector(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Connector> {
    let store = state.store.lock().unwrap();
    store
        .connectors
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("connector not found"))
async fn patch_connector(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchConnectorRequest>,
) -> ApiResult<Connector> {
    let mut store = state.store.lock().unwrap();
    let connector = store
        .connectors
        .get_mut(&id)
        .ok_or_else(|| not_found("connector not found"))?;
    if let Some(status) = req.status {
        connector.status = status;
    if let Some(config) = req.config {
        connector.config = config;
    if let Some(dsl) = req.transform_dsl {
        connector.transform_dsl = Some(dsl);
    connector.updated_at = Utc::now();
    Ok(Json(connector.clone()))
async fn delete_connector(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    let mut store = state.store.lock().unwrap();
    store
        .connectors
        .remove(&id)
        .map(|_| Json(serde_json::json!({ "deleted": id })))
        .ok_or_else(|| not_found("connector not found"))
// ── Dead Letter Queue ─────────────────────────────────────────────────────────
struct DlqFilter {
    stream_id: Option<Uuid>,
    status: Option<String>,
    limit: Option<u32>,
async fn list_dlq(
    State(state): State<Arc<StreamsState>>,
    Query(filter): Query<DlqFilter>,
) -> Json<Vec<DeadLetterEntry>> {
    let store = state.store.lock().unwrap();
    let limit = filter.limit.unwrap_or(100) as usize;
    let entries: Vec<DeadLetterEntry> = store
        .dlq
        .values()
        .filter(|e| {
            filter.stream_id.map_or(true, |id| e.stream_id == id)
                && filter.status.as_ref().map_or(true, |s| {
                    format!("{:?}", e.status).to_lowercase() == s.to_lowercase()
                })
        })
        .take(limit)
        .cloned()
        .collect();
    Json(entries)
async fn get_dlq_entry(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<DeadLetterEntry> {
    let store = state.store.lock().unwrap();
    store
        .dlq
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("DLQ entry not found"))
async fn retry_dlq_entry(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    let mut store = state.store.lock().unwrap();
    let entry = store
        .dlq
        .get_mut(&id)
        .ok_or_else(|| not_found("DLQ entry not found"))?;
    if entry.status == DlqStatus::Discarded {
        return Err(bad_request("cannot retry a discarded DLQ entry"));
    if entry.retry_count >= entry.retry_policy.max_retries {
        entry.status = DlqStatus::Exhausted;
        return Err(bad_request("retry limit exhausted"));
    entry.retry_count += 1;
    entry.status = DlqStatus::Retrying;
    entry.last_retry_at = Some(Utc::now());
    // Compute next retry with exponential backoff.
    let delay_ms = match entry.retry_policy.backoff {
        BackoffStrategy::Fixed => entry.retry_policy.initial_delay_ms,
        BackoffStrategy::Linear => {
            entry.retry_policy.initial_delay_ms * entry.retry_count as u64
        BackoffStrategy::Exponential | BackoffStrategy::Jittered => {
            (entry.retry_policy.initial_delay_ms
                * 2u64.pow(entry.retry_count.saturating_sub(1)))
            .min(entry.retry_policy.max_delay_ms)
    entry.next_retry_at = Some(
        Utc::now()
            + chrono::Duration::milliseconds(delay_ms as i64),
    );
    Ok(Json(serde_json::json!({
        "id": id,
        "retry_count": entry.retry_count,
        "next_retry_at": entry.next_retry_at,
        "status": entry.status,
    })))
async fn discard_dlq_entry(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
    let mut store = state.store.lock().unwrap();
    let entry = store
        .dlq
        .get_mut(&id)
        .ok_or_else(|| not_found("DLQ entry not found"))?;
    entry.status = DlqStatus::Discarded;
    Ok(Json(serde_json::json!({ "discarded": id })))
// ── Tiered Storage ────────────────────────────────────────────────────────────
async fn get_storage_config(
    State(state): State<Arc<StreamsState>>,
) -> Json<StorageTierConfig> {
    let store = state.store.lock().unwrap();
    Json(store.storage_config.clone())
async fn update_storage_config(
    State(state): State<Arc<StreamsState>>,
    Json(config): Json<StorageTierConfig>,
) -> Json<StorageTierConfig> {
    let mut store = state.store.lock().unwrap();
    store.storage_config = config.clone();
    Json(config)
async fn storage_tier_stats(
    State(state): State<Arc<StreamsState>>,
) -> Json<StorageTierStats> {
    let store = state.store.lock().unwrap();
    // Estimate tier distribution: first ~20% of messages = cold, middle = warm, rest = hot.
    let total = store.messages.len() as u64;
    let cold_count = total / 5;
    let warm_count = total * 3 / 10;
    let hot_count = total - cold_count - warm_count;
    // Rough size estimate: 1 KB average per message.
    let avg_bytes = 1024u64;
    Json(StorageTierStats {
        hot_message_count: hot_count,
        warm_message_count: warm_count,
        cold_message_count: cold_count,
        hot_estimated_bytes: hot_count * avg_bytes,
        warm_estimated_bytes: warm_count * avg_bytes,
        cold_estimated_bytes: cold_count * avg_bytes,
        auto_tier_enabled: store.storage_config.auto_tier_enabled,
        config: store.storage_config.clone(),
    })
// ── Metrics ───────────────────────────────────────────────────────────────────
async fn stream_metrics(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StreamMetrics> {
    let store = state.store.lock().unwrap();
    if !store.streams.contains_key(&id) {
        return Err(not_found("stream not found"));
    // Return stored metrics or a zero snapshot.
    let m = store.metrics.get(&id).cloned().unwrap_or(StreamMetrics {
        stream_id: id,
        timestamp: Utc::now(),
        messages_in_per_sec: 0.0,
        messages_out_per_sec: 0.0,
        bytes_in_per_sec: 0.0,
        bytes_out_per_sec: 0.0,
        subscriber_count: store
            .subscriptions
            .values()
            .filter(|s| s.stream_id == id)
            .count() as u32,
        dlq_count: store.dlq.values().filter(|e| e.stream_id == id).count() as u64,
        hot_bytes: 0,
        warm_bytes: 0,
        cold_bytes: 0,
        publish_latency_ms_p50: 0.0,
        publish_latency_ms_p99: 0.0,
        end_to_end_latency_ms_p99: 0.0,
        exactly_once_dedup_count: store.dedup_hit_count,
    Ok(Json(m))
async fn platform_metrics(
    State(state): State<Arc<StreamsState>>,
) -> Json<PlatformMetrics> {
    let store = state.store.lock().unwrap();
    let per_stream: Vec<StreamMetrics> = store
        .streams
        .keys()
        .map(|&id| {
            store.metrics.get(&id).cloned().unwrap_or(StreamMetrics {
                stream_id: id,
                timestamp: Utc::now(),
                messages_in_per_sec: 0.0,
                messages_out_per_sec: 0.0,
                bytes_in_per_sec: 0.0,
                bytes_out_per_sec: 0.0,
                subscriber_count: store
                    .subscriptions
                    .values()
                    .filter(|s| s.stream_id == id)
                    .count() as u32,
                dlq_count: store.dlq.values().filter(|e| e.stream_id == id).count() as u64,
                hot_bytes: 0,
                warm_bytes: 0,
                cold_bytes: 0,
                publish_latency_ms_p50: 0.0,
                publish_latency_ms_p99: 0.0,
                end_to_end_latency_ms_p99: 0.0,
                exactly_once_dedup_count: store.dedup_hit_count,
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
    Json(PlatformMetrics {
        timestamp: Utc::now(),
        total_streams: store.streams.len() as u64,
        total_subscriptions: store.subscriptions.len() as u64,
        total_messages: store.messages.len() as u64,
        total_connectors: store.connectors.len() as u64,
        total_dlq_entries: store.dlq.len() as u64,
        active_schemas: store.schemas.len() as u64,
        dedup_cache_size: store.dedup_ids.len() as u64,
        per_stream,
    })
// ── Backpressure ──────────────────────────────────────────────────────────────
async fn backpressure_status(
    State(state): State<Arc<StreamsState>>,
) -> Json<Vec<BackpressureStatus>> {
    let store = state.store.lock().unwrap();
    let statuses: Vec<BackpressureStatus> = store
        .streams
        .values()
        .map(|stream| {
            let slow_subs: Vec<SlowSubscription> = store
                .subscriptions
                .values()
                .filter(|s| s.stream_id == stream.id && s.lag > 1000)
                .map(|s| SlowSubscription {
                    subscription_id: s.id,
                    name: s.name.clone(),
                    lag: s.lag,
                    status: s.status.clone(),
                })
                .collect();
            let throttle_active = stream.throughput_limit.is_some();
            let recommended = if slow_subs.len() > 3 {
                BackpressureAction::ThrottlePublishers
            } else if !slow_subs.is_empty() {
                BackpressureAction::ScaleConsumers
            } else if stream.message_count > 1_000_000 {
                BackpressureAction::MoveToColdTier
            } else {
                BackpressureAction::None
            BackpressureStatus {
                stream_id: stream.id,
                stream_name: stream.name.clone(),
                throttle_active,
                current_limit: stream.throughput_limit.clone(),
                slow_subscriptions: slow_subs,
                recommended_action: recommended,
        })
        .collect();
    Json(statuses)
}
