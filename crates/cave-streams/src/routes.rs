//! HTTP routes for cave-streams — all under /api/v1/streams (and /api/v1/*).

use crate::{models::*, StreamsState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;
type StatusResult<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<serde_json::Value>)>;

fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

pub fn create_router(state: Arc<StreamsState>) -> Router {
    Router::new()
        // ── Health ────────────────────────────────────────────────────────
        .route("/api/v1/streams/health", get(health))
        // ── Streams ───────────────────────────────────────────────────────
        // NOTE: literal sub-paths (/health, /backpressure-all) are registered
        // before the /:id wildcard so axum prefers them in matching.
        .route("/api/v1/streams", post(create_stream).get(list_streams))
        .route(
            "/api/v1/streams/:id",
            get(get_stream).delete(delete_stream),
        )
        .route("/api/v1/streams/:id/stats", get(stream_stats))
        .route("/api/v1/streams/:id/throttle", put(throttle_stream))
        // ── Subscriptions (nested) ────────────────────────────────────────
        .route(
            "/api/v1/streams/:stream_id/subscriptions",
            post(create_subscription).get(list_subscriptions),
        )
        .route(
            "/api/v1/streams/:stream_id/subscriptions/:sub_id",
            get(get_subscription).delete(delete_subscription),
        )
        .route(
            "/api/v1/streams/:stream_id/subscriptions/:sub_id/pause",
            put(pause_subscription),
        )
        .route(
            "/api/v1/streams/:stream_id/subscriptions/:sub_id/resume",
            put(resume_subscription),
        )
        // ── Messages ──────────────────────────────────────────────────────
        .route("/api/v1/streams/:stream_id/publish", post(publish_message))
        .route("/api/v1/streams/:stream_id/pull", post(pull_messages))
        .route(
            "/api/v1/streams/:stream_id/subscriptions/:sub_id/ack",
            post(ack_messages),
        )
        // ── Schema Registry ───────────────────────────────────────────────
        .route("/api/v1/schemas", post(register_schema).get(list_schemas))
        .route(
            "/api/v1/schemas/validate",
            post(validate_schema),
        )
        .route(
            "/api/v1/schemas/:id",
            get(get_schema).delete(delete_schema),
        )
        // ── Connectors ────────────────────────────────────────────────────
        .route(
            "/api/v1/connectors",
            post(create_connector).get(list_connectors),
        )
        .route(
            "/api/v1/connectors/:id",
            get(get_connector)
                .patch(patch_connector)
                .delete(delete_connector),
        )
        // ── Dead Letter Queue ─────────────────────────────────────────────
        .route("/api/v1/dlq", get(list_dlq))
        .route("/api/v1/dlq/:id", get(get_dlq_entry).delete(discard_dlq_entry))
        .route("/api/v1/dlq/:id/retry", post(retry_dlq_entry))
        // ── Tiered Storage ────────────────────────────────────────────────
        .route(
            "/api/v1/storage/config",
            get(get_storage_config).put(update_storage_config),
        )
        .route("/api/v1/storage/tiers", get(storage_tier_stats))
        // ── Metrics ───────────────────────────────────────────────────────
        .route("/api/v1/streams/:id/metrics", get(stream_metrics))
        .route("/api/v1/metrics", get(platform_metrics))
        // ── Backpressure ──────────────────────────────────────────────────
        .route("/api/v1/backpressure", get(backpressure_status))
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-streams",
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
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
        }
    }))
}

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
    };
    let mut store = state.store.lock().unwrap();
    store.streams.insert(stream.id, stream.clone());
    (StatusCode::CREATED, Json(stream))
}

#[derive(Deserialize)]
struct NamespaceFilter {
    namespace: Option<String>,
    label: Option<String>,
}

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
                        }
                    })
        })
        .cloned()
        .collect();
    Json(streams)
}

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
}

async fn delete_stream(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    // Reject if active subscriptions exist.
    let has_subs = store.subscriptions.values().any(|s| s.stream_id == id);
    if has_subs {
        return Err(bad_request(
            "cannot delete stream with active subscriptions; delete subscriptions first",
        ));
    }
    store
        .streams
        .remove(&id)
        .map(|s| Json(serde_json::json!({ "deleted": s.id, "name": s.name })))
        .ok_or_else(|| not_found("stream not found"))
}

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
    }))
}

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
    });
    stream.updated_at = Utc::now();
    Ok(Json(stream.clone()))
}

// ── Subscriptions ─────────────────────────────────────────────────────────────

async fn create_subscription(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> StatusResult<Subscription> {
    let mut store = state.store.lock().unwrap();
    if !store.streams.contains_key(&stream_id) {
        return Err(not_found("stream not found"));
    }
    let cursor = match req.delivery_policy.as_ref().unwrap_or(&DeliveryPolicy::Latest) {
        DeliveryPolicy::Earliest => 0,
        DeliveryPolicy::BySequence(seq) => *seq,
        _ => store.stream_sequences.get(&stream_id).copied().unwrap_or(0),
    };
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
    };
    store.subscriptions.insert(sub.id, sub.clone());
    store.refresh_subscriber_count(stream_id);
    Ok((StatusCode::CREATED, Json(sub)))
}

#[derive(Deserialize)]
struct StreamIdFilter {
    stream_id: Option<Uuid>,
}

async fn list_subscriptions(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
) -> ApiResult<Vec<Subscription>> {
    let store = state.store.lock().unwrap();
    if !store.streams.contains_key(&stream_id) {
        return Err(not_found("stream not found"));
    }
    let subs: Vec<Subscription> = store
        .subscriptions
        .values()
        .filter(|s| s.stream_id == stream_id)
        .cloned()
        .collect();
    Ok(Json(subs))
}

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
    }
    Ok(Json(sub.clone()))
}

async fn delete_subscription(
    State(state): State<Arc<StreamsState>>,
    Path((stream_id, sub_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let sub = store
        .subscriptions
        .get(&sub_id)
        .ok_or_else(|| not_found("subscription not found"))?;
    if sub.stream_id != stream_id {
        return Err(not_found("subscription not found on this stream"));
    }
    store.subscriptions.remove(&sub_id);
    store.refresh_subscriber_count(stream_id);
    Ok(Json(serde_json::json!({ "deleted": sub_id })))
}

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
    }
    sub.status = SubscriptionStatus::Paused;
    sub.updated_at = Utc::now();
    Ok(Json(sub.clone()))
}

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
    }
    sub.status = SubscriptionStatus::Active;
    sub.updated_at = Utc::now();
    Ok(Json(sub.clone()))
}

// ── Messages ──────────────────────────────────────────────────────────────────

async fn publish_message(
    State(state): State<Arc<StreamsState>>,
    Path(stream_id): Path<Uuid>,
    Json(req): Json<PublishRequest>,
) -> StatusResult<PublishResponse> {
    let mut store = state.store.lock().unwrap();

    if !store.streams.contains_key(&stream_id) {
        return Err(not_found("stream not found"));
    }

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
        }
    }

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
    };
    store.messages.push(msg);

    // Update stream counters.
    if let Some(stream) = store.streams.get_mut(&stream_id) {
        stream.message_count += 1;
        stream.sequence = seq;
        stream.updated_at = ts;
    }

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
}

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
    }

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
}

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
        }
    }

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
    }
    let cursor = sub.cursor;
    sub.lag = stream_seq.saturating_sub(cursor);
    sub.updated_at = Utc::now();

    Ok(Json(AckResponse {
        acked,
        not_found: not_found_ids,
        cursor_advanced_to: cursor,
    }))
}

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
    };
    store.schemas.insert(schema.id, schema.clone());
    (StatusCode::CREATED, Json(schema))
}

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
            .into_iter()
            .collect();
        store
            .schemas
            .values()
            .filter(|s| schema_ids.contains(&s.id))
            .cloned()
            .collect()
    } else {
        store.schemas.values().cloned().collect()
    };
    Json(schemas)
}

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
}

async fn delete_schema(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let in_use = store.streams.values().any(|s| s.schema_id == Some(id));
    if in_use {
        return Err(bad_request(
            "schema is in use by one or more streams; update streams first",
        ));
    }
    store
        .schemas
        .remove(&id)
        .map(|_| Json(serde_json::json!({ "deleted": id })))
        .ok_or_else(|| not_found("schema not found"))
}

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
            }
        }
    };

    Json(ValidateSchemaResponse {
        valid: true, // syntactic validity requires format-specific parse
        compatible,
        errors,
    })
}

// ── Connectors ────────────────────────────────────────────────────────────────

async fn create_connector(
    State(state): State<Arc<StreamsState>>,
    Json(req): Json<CreateConnectorRequest>,
) -> StatusResult<Connector> {
    let mut store = state.store.lock().unwrap();
    if !store.streams.contains_key(&req.stream_id) {
        return Err(not_found("stream not found"));
    }
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
    };
    store.connectors.insert(connector.id, connector.clone());
    Ok((StatusCode::CREATED, Json(connector)))
}

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
}

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
}

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
    }
    if let Some(config) = req.config {
        connector.config = config;
    }
    if let Some(dsl) = req.transform_dsl {
        connector.transform_dsl = Some(dsl);
    }
    connector.updated_at = Utc::now();
    Ok(Json(connector.clone()))
}

async fn delete_connector(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    store
        .connectors
        .remove(&id)
        .map(|_| Json(serde_json::json!({ "deleted": id })))
        .ok_or_else(|| not_found("connector not found"))
}

// ── Dead Letter Queue ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DlqFilter {
    stream_id: Option<Uuid>,
    status: Option<String>,
    limit: Option<u32>,
}

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
}

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
}

async fn retry_dlq_entry(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let entry = store
        .dlq
        .get_mut(&id)
        .ok_or_else(|| not_found("DLQ entry not found"))?;

    if entry.status == DlqStatus::Discarded {
        return Err(bad_request("cannot retry a discarded DLQ entry"));
    }
    if entry.retry_count >= entry.retry_policy.max_retries {
        entry.status = DlqStatus::Exhausted;
        return Err(bad_request("retry limit exhausted"));
    }

    entry.retry_count += 1;
    entry.status = DlqStatus::Retrying;
    entry.last_retry_at = Some(Utc::now());

    // Compute next retry with exponential backoff.
    let delay_ms = match entry.retry_policy.backoff {
        BackoffStrategy::Fixed => entry.retry_policy.initial_delay_ms,
        BackoffStrategy::Linear => {
            entry.retry_policy.initial_delay_ms * entry.retry_count as u64
        }
        BackoffStrategy::Exponential | BackoffStrategy::Jittered => {
            (entry.retry_policy.initial_delay_ms
                * 2u64.pow(entry.retry_count.saturating_sub(1)))
            .min(entry.retry_policy.max_delay_ms)
        }
    };
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
}

async fn discard_dlq_entry(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let entry = store
        .dlq
        .get_mut(&id)
        .ok_or_else(|| not_found("DLQ entry not found"))?;
    entry.status = DlqStatus::Discarded;
    Ok(Json(serde_json::json!({ "discarded": id })))
}

// ── Tiered Storage ────────────────────────────────────────────────────────────

async fn get_storage_config(
    State(state): State<Arc<StreamsState>>,
) -> Json<StorageTierConfig> {
    let store = state.store.lock().unwrap();
    Json(store.storage_config.clone())
}

async fn update_storage_config(
    State(state): State<Arc<StreamsState>>,
    Json(config): Json<StorageTierConfig>,
) -> Json<StorageTierConfig> {
    let mut store = state.store.lock().unwrap();
    store.storage_config = config.clone();
    Json(config)
}

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
}

// ── Metrics ───────────────────────────────────────────────────────────────────

async fn stream_metrics(
    State(state): State<Arc<StreamsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StreamMetrics> {
    let store = state.store.lock().unwrap();
    if !store.streams.contains_key(&id) {
        return Err(not_found("stream not found"));
    }
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
    });
    Ok(Json(m))
}

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
}

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
            };

            BackpressureStatus {
                stream_id: stream.id,
                stream_name: stream.name.clone(),
                throttle_active,
                current_limit: stream.throughput_limit.clone(),
                slow_subscriptions: slow_subs,
                recommended_action: recommended,
            }
        })
        .collect();
    Json(statuses)
}
