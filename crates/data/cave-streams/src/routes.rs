// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST management routes: Schema Registry, Kafka Connect, admin endpoints.

use crate::StreamsState;
use crate::connect::{ConnectCluster, Connector};
use crate::schema_registry::{CompatibilityConfig, RegisterSchemaRequest, SchemaFormat};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

pub fn create_router(state: Arc<StreamsState>) -> Router {
    // Separate state for connect cluster
    let connect = Arc::new(crate::connect::ConnectCluster::new());

    Router::new()
        // ── Health ─────────────────────────────────────────────────────────
        .route("/api/streams/health", get(health))
        .route("/api/streams/metrics", get(metrics))

        // ── Schema Registry ────────────────────────────────────────────────
        .route("/subjects", get(list_subjects))
        .route("/subjects/{subject}/versions", post(register_schema))
        .route("/subjects/{subject}/versions", get(list_versions))
        .route("/subjects/{subject}/versions/{version}", get(get_schema_version))
        .route("/subjects/{subject}", delete(delete_subject))
        .route("/schemas/ids/{id}", get(get_schema_by_id))
        .route(
            "/compatibility/subjects/{subject}/versions/{version}",
            post(check_compatibility),
        )
        .route("/config", get(get_global_config).put(set_global_config))
        .route(
            "/config/{subject}",
            get(get_subject_config).put(set_subject_config),
        )

        // ── Kafka Connect ──────────────────────────────────────────────────
        .route("/connectors", get(list_connectors).post(create_connector))
        .route("/connectors/{name}", get(get_connector).delete(delete_connector))
        .route("/connectors/{name}/config", put(update_connector_config))
        .route("/connectors/{name}/status", get(get_connector_status))
        .route("/connectors/{name}/restart", post(restart_connector))
        .route("/connectors/{name}/pause", put(pause_connector))
        .route("/connectors/{name}/resume", put(resume_connector))
        .route("/connectors/{name}/tasks", get(list_tasks))
        .route("/connectors/{name}/tasks/{task_id}/restart", post(restart_task))
        .route("/connector-plugins", get(list_plugins))

        // ── Broker management (Kafka) ──────────────────────────────────────
        .route("/api/streams/topics", get(list_topics).post(create_topic_rest))
        .route("/api/streams/topics/{name}", delete(delete_topic_rest))
        .route("/api/streams/topics/{name}/config", get(get_topic_config).put(alter_topic_config))
        .route("/api/streams/groups", get(list_groups))
        .route("/api/streams/acls", get(list_acls).post(create_acl))
        .route("/api/streams/quotas", get(list_quotas).post(set_quota_rest))
        .route("/api/streams/mirror", get(list_mirror_flows))

        // ── Pulsar admin (pulsar-admin REST parity) ────────────────────────
        .route("/api/streams/pulsar/tenants",
               get(pulsar_list_tenants).post(pulsar_create_tenant))
        .route("/api/streams/pulsar/tenants/{tenant}",
               delete(pulsar_delete_tenant))
        .route("/api/streams/pulsar/namespaces/{tenant}",
               get(pulsar_list_namespaces).post(pulsar_create_namespace))
        .route("/api/streams/pulsar/namespaces/{tenant}/{namespace}",
               delete(pulsar_delete_namespace))
        .route("/api/streams/pulsar/namespaces/{tenant}/{namespace}/retention",
               post(pulsar_set_retention))
        .route("/api/streams/pulsar/namespaces/{tenant}/{namespace}/messageTTL",
               post(pulsar_set_ttl))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}",
               get(pulsar_list_topics))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}",
               post(pulsar_create_topic).delete(pulsar_delete_topic))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/stats",
               get(pulsar_topic_stats))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscriptions",
               get(pulsar_list_subscriptions))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{sub}",
               post(pulsar_create_subscription).delete(pulsar_delete_subscription))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{sub}/skipAll",
               post(pulsar_skip_all))
        .route("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{sub}/resetCursor",
               post(pulsar_reset_cursor))

        .with_state((state, connect))
}

type AppState = (Arc<StreamsState>, Arc<ConnectCluster>);

async fn health(State((s, _)): State<AppState>) -> Json<serde_json::Value> {
    let kafka_topics = s.broker.list_topics().len();
    let kafka_groups = s.broker.groups.list_groups().len();
    let pulsar_tenants = s.pulsar_admin.list_tenants().len();
    Json(json!({
        "module": "cave-streams",
        "status": "ok",
        "upstreams": ["apache-kafka", "apache-pulsar"],
        "kafka": {
            "port": crate::KAFKA_PORT,
            "topics": kafka_topics,
            "consumer_groups": kafka_groups,
        },
        "pulsar": {
            "port": crate::PULSAR_PORT,
            "tenants": pulsar_tenants,
        },
    }))
}

/// `GET /api/streams/metrics` — Prometheus text exposition (version 0.0.4):
/// live Kafka/Pulsar gauges plus the streaming-ray-2 preview counters.
async fn metrics(State((s, _)): State<AppState>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        crate::metrics::render_prometheus(&s),
    )
}

// ── Schema Registry handlers ──────────────────────────────────────────────────

async fn list_subjects(State((s, _)): State<AppState>) -> Json<Vec<String>> {
    let _ = Json(
        s.broker
            .transactions
            .list_transactions()
            .into_iter()
            .map(|t| t.transactional_id)
            .collect::<Vec<_>>(),
    );
    // Actually list schema subjects
    Json(vec![]) // Placeholder — real impl routes to schema_registry
}

async fn register_schema(
    State((_s, _)): State<AppState>,
    Path(subject): Path<String>,
    Json(req): Json<RegisterSchemaRequest>,
) -> impl IntoResponse {
    // Schema registry is on the broker for this demo
    let format = SchemaFormat::from_str(&req.schema_type);
    let refs = req
        .references
        .into_iter()
        .map(|r| crate::schema_registry::SchemaReference {
            name: r.name,
            subject: r.subject,
            version: r.version,
        })
        .collect();
    match crate::schema_registry::SchemaRegistry::new()
        .register_schema(&subject, req.schema, format, refs)
    {
        Ok(id) => (StatusCode::OK, Json(json!({"id": id}))).into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error_code": 42201, "message": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_versions(Path(_subject): Path<String>) -> Json<Vec<i32>> {
    Json(vec![1]) // placeholder
}

async fn get_schema_version(
    Path((_subject, _version)): Path<(String, String)>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error_code": 40401, "message": "Subject not found"})),
    )
        .into_response()
}

async fn delete_subject(Path(_subject): Path<String>) -> Json<Vec<i32>> {
    Json(vec![])
}

async fn get_schema_by_id(Path(_id): Path<i32>) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error_code": 40403, "message": "Schema not found"})),
    )
        .into_response()
}

async fn check_compatibility(
    Path((_subject, _version)): Path<(String, String)>,
    Json(_req): Json<RegisterSchemaRequest>,
) -> Json<serde_json::Value> {
    Json(json!({"is_compatible": true}))
}

async fn get_global_config() -> Json<CompatibilityConfig> {
    Json(CompatibilityConfig {
        compatibility: "BACKWARD".into(),
    })
}

async fn set_global_config(Json(req): Json<CompatibilityConfig>) -> Json<CompatibilityConfig> {
    Json(req)
}

async fn get_subject_config(Path(_subject): Path<String>) -> Json<CompatibilityConfig> {
    Json(CompatibilityConfig {
        compatibility: "BACKWARD".into(),
    })
}

async fn set_subject_config(
    Path(_subject): Path<String>,
    Json(req): Json<CompatibilityConfig>,
) -> Json<CompatibilityConfig> {
    Json(req)
}

// ── Kafka Connect handlers ────────────────────────────────────────────────────

async fn list_connectors(State((_, connect)): State<AppState>) -> Json<Vec<String>> {
    Json(connect.list_connectors())
}

async fn create_connector(
    State((_, connect)): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = req["name"].as_str().unwrap_or("").to_string();
    let config: HashMap<String, String> = req["config"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    match connect.create_connector(name, config) {
        Ok(c) => (StatusCode::CREATED, Json(connector_to_json(&c))).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(json!({"message": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.get_connector(&name) {
        Ok(c) => (StatusCode::OK, Json(connector_to_json(&c))).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": "connector not found"})),
        )
            .into_response(),
    }
}

async fn delete_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.delete_connector(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": "connector not found"})),
        )
            .into_response(),
    }
}

async fn update_connector_config(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
    Json(config): Json<HashMap<String, String>>,
) -> impl IntoResponse {
    match connect.update_connector_config(&name, config) {
        Ok(c) => Json(connector_to_json(&c)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(json!({"message": "not found"}))).into_response(),
    }
}

async fn get_connector_status(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.get_connector(&name) {
        Ok(c) => Json(json!({
            "name": c.name,
            "connector": {"state": format!("{:?}", c.state), "worker_id": "worker-1"},
            "tasks": c.tasks.iter().map(|t| json!({
                "id": t.id.task,
                "state": format!("{:?}", t.state),
                "worker_id": t.worker_id,
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(json!({"message": "not found"}))).into_response(),
    }
}

async fn restart_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.restart_connector(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pause_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.pause_connector(&name) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn resume_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.resume_connector(&name) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn list_tasks(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.get_tasks(&name) {
        Ok(tasks) => Json(
            tasks
                .iter()
                .map(|t| {
                    json!({
                        "id": {"connector": t.id.connector, "task": t.id.task},
                        "config": t.config,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn restart_task(
    State((_, connect)): State<AppState>,
    Path((name, task_id)): Path<(String, usize)>,
) -> impl IntoResponse {
    match connect.restart_task(&name, task_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn list_plugins(State((_, connect)): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(
        connect
            .list_plugins()
            .into_iter()
            .map(|p| {
                json!({
                    "class": p.class,
                    "type": p.plugin_type,
                    "version": p.version,
                })
            })
            .collect(),
    )
}

// ── Broker management handlers ────────────────────────────────────────────────

async fn list_topics(State((s, _)): State<AppState>) -> Json<Vec<String>> {
    Json(s.broker.list_topics())
}

async fn create_topic_rest(
    State((s, _)): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = req["name"].as_str().unwrap_or("").to_string();
    let partitions = req["partitions"].as_i64().unwrap_or(1) as i32;
    let replication = req["replication_factor"].as_i64().unwrap_or(1) as i16;
    match s.broker.create_topic(name, partitions, replication, vec![]) {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn delete_topic_rest(
    State((s, _)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.broker.delete_topic(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_topic_config(
    State((s, _)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.broker.get_topic_configs(&name) {
        Ok(configs) => Json(configs).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn alter_topic_config(
    State((s, _)): State<AppState>,
    Path(name): Path<String>,
    Json(configs): Json<HashMap<String, Option<String>>>,
) -> impl IntoResponse {
    let pairs: Vec<(String, Option<String>)> = configs.into_iter().collect();
    match s.broker.alter_topic_configs(&name, pairs) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn list_groups(State((s, _)): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(
        s.broker
            .groups
            .list_groups()
            .into_iter()
            .map(|g| {
                json!({
                    "group_id": g.group_id,
                    "protocol_type": g.protocol_type,
                    "state": g.state,
                })
            })
            .collect(),
    )
}

async fn list_acls(State((s, _)): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let filter = crate::acl::AclFilter {
        resource_type: None,
        resource_name: None,
        pattern_type: None,
        principal: None,
        host: None,
        operation: None,
        permission: None,
    };
    let acls = s.broker.acls.describe_acls(&filter);
    Json(
        acls.iter()
            .map(|a| {
                json!({
                    "resource_type": format!("{:?}", a.resource_type),
                    "resource_name": a.resource_name,
                    "principal": a.principal,
                    "operation": format!("{:?}", a.operation),
                    "permission": format!("{:?}", a.permission),
                })
            })
            .collect(),
    )
}

async fn create_acl(
    State((s, _)): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let acl = crate::acl::AclBinding {
        resource_type: crate::acl::ResourceType::Topic,
        resource_name: body["resource_name"].as_str().unwrap_or("").to_string(),
        pattern_type: crate::acl::PatternType::Literal,
        principal: body["principal"].as_str().unwrap_or("").to_string(),
        host: "*".into(),
        operation: crate::acl::Operation::Read,
        permission: crate::acl::PermissionType::Allow,
    };
    s.broker.acls.create_acl(acl);
    StatusCode::CREATED
}

async fn list_quotas(State((s, _)): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(
        s.broker
            .quotas
            .list_quotas()
            .into_iter()
            .map(|(e, q)| {
                json!({
                    "entity_type": format!("{:?}", e.entity_type),
                    "entity_name": e.entity_name,
                    "producer_byte_rate": q.producer_byte_rate,
                    "consumer_byte_rate": q.consumer_byte_rate,
                })
            })
            .collect(),
    )
}

async fn set_quota_rest(
    State((s, _)): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let entity = crate::quota::QuotaEntity::user(body["user"].as_str().unwrap_or("default"));
    let quota = crate::quota::Quota {
        producer_byte_rate: body["producer_byte_rate"].as_f64(),
        consumer_byte_rate: body["consumer_byte_rate"].as_f64(),
        request_percentage: body["request_percentage"].as_f64(),
        controller_mutation_rate: None,
    };
    s.broker.quotas.set_quota(entity, quota);
    StatusCode::OK
}

async fn list_mirror_flows() -> Json<Vec<serde_json::Value>> {
    Json(vec![]) // Mirror flows are managed separately
}

// ── Pulsar admin handlers ─────────────────────────────────────────────────────

async fn pulsar_list_tenants(State((s, _)): State<AppState>) -> Json<Vec<serde_json::Value>> {
    Json(
        s.pulsar_admin
            .list_tenants()
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "admin_roles": t.admin_roles,
                    "allowed_clusters": t.allowed_clusters,
                })
            })
            .collect(),
    )
}

async fn pulsar_create_tenant(
    State((s, _)): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = body["name"].as_str().unwrap_or("").to_string();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "name required"})),
        )
            .into_response();
    }
    let t = s.pulsar_admin.create_tenant(&name);
    (StatusCode::CREATED, Json(json!({"name": t.name}))).into_response()
}

async fn pulsar_delete_tenant(
    State((s, _)): State<AppState>,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    match s.pulsar_admin.delete_tenant(&tenant) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_list_namespaces(
    State((s, _)): State<AppState>,
    Path(tenant): Path<String>,
) -> Json<Vec<String>> {
    Json(s.pulsar_admin.list_namespaces(&tenant))
}

async fn pulsar_create_namespace(
    State((s, _)): State<AppState>,
    Path(tenant): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = body["namespace"].as_str().unwrap_or("").to_string();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "namespace required"})),
        )
            .into_response();
    }
    match s.pulsar_admin.create_namespace(&tenant, &name) {
        Ok(ns) => (StatusCode::CREATED, Json(json!({"fqn": ns.fqn()}))).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn pulsar_delete_namespace(
    State((s, _)): State<AppState>,
    Path((tenant, namespace)): Path<(String, String)>,
) -> impl IntoResponse {
    let fqn = format!("{tenant}/{namespace}");
    match s.pulsar_admin.delete_namespace(&fqn) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_set_retention(
    State((s, _)): State<AppState>,
    Path((tenant, namespace)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let fqn = format!("{tenant}/{namespace}");
    let minutes = body["retentionTimeInMinutes"].as_u64().unwrap_or(0);
    let size_mb = body["retentionSizeInMB"].as_u64().unwrap_or(0);
    match s
        .pulsar_admin
        .set_namespace_retention(&fqn, minutes, size_mb)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_set_ttl(
    State((s, _)): State<AppState>,
    Path((tenant, namespace)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let fqn = format!("{tenant}/{namespace}");
    let ttl = body["messageTTL"].as_u64().unwrap_or(0);
    match s.pulsar_admin.set_namespace_ttl(&fqn, ttl) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_list_topics(
    State((s, _)): State<AppState>,
    Path((tenant, namespace)): Path<(String, String)>,
) -> Json<Vec<String>> {
    let fqn = format!("{tenant}/{namespace}");
    Json(s.pulsar_admin.list_topics(&fqn))
}

#[derive(serde::Deserialize, Default)]
struct PulsarTopicQuery {
    domain: Option<String>,
    partitions: Option<u32>,
}

async fn pulsar_create_topic(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic)): Path<(String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    let partitions = q.partitions.unwrap_or(0);
    match s.pulsar_admin.create_topic(&fqn, partitions) {
        Ok(t) => (StatusCode::CREATED, Json(json!({"fqn": t.fqn()}))).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn pulsar_delete_topic(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic)): Path<(String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    match s.pulsar_admin.delete_topic(&fqn) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_topic_stats(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic)): Path<(String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    match s.pulsar_admin.topic_stats(&fqn) {
        Ok(st) => Json(st).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_list_subscriptions(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic)): Path<(String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    match s.pulsar_admin.list_subscriptions(&fqn) {
        Ok(subs) => Json(subs.into_iter().map(|s| s.name).collect::<Vec<_>>()).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
struct PulsarSubBody {
    #[serde(default)]
    sub_type: Option<String>,
    #[serde(default)]
    initial_position: Option<String>,
}

fn parse_sub_type(s: Option<&str>) -> crate::pulsar_admin::SubscriptionType {
    match s.unwrap_or("Exclusive") {
        "Shared" | "shared" => crate::pulsar_admin::SubscriptionType::Shared,
        "Failover" | "failover" => crate::pulsar_admin::SubscriptionType::Failover,
        "KeyShared" | "key_shared" => crate::pulsar_admin::SubscriptionType::KeyShared,
        _ => crate::pulsar_admin::SubscriptionType::Exclusive,
    }
}

fn parse_initial_position(s: Option<&str>) -> crate::pulsar_admin::InitialPosition {
    match s.unwrap_or("earliest") {
        "latest" => crate::pulsar_admin::InitialPosition::Latest,
        _ => crate::pulsar_admin::InitialPosition::Earliest,
    }
}

async fn pulsar_create_subscription(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic, sub)): Path<(String, String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
    Json(body): Json<PulsarSubBody>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    let sub_type = parse_sub_type(body.sub_type.as_deref());
    let pos = parse_initial_position(body.initial_position.as_deref());
    match s
        .pulsar_admin
        .create_subscription(&fqn, &sub, sub_type, pos)
    {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn pulsar_delete_subscription(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic, sub)): Path<(String, String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    match s.pulsar_admin.delete_subscription(&fqn, &sub) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pulsar_skip_all(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic, sub)): Path<(String, String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    match s.pulsar_admin.skip_all(&fqn, &sub) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
struct ResetCursorBody {
    position: Option<String>, // "earliest", "latest", or numeric offset string
}

async fn pulsar_reset_cursor(
    State((s, _)): State<AppState>,
    Path((tenant, namespace, topic, sub)): Path<(String, String, String, String)>,
    axum::extract::Query(q): axum::extract::Query<PulsarTopicQuery>,
    Json(body): Json<ResetCursorBody>,
) -> impl IntoResponse {
    let scheme = q.domain.as_deref().unwrap_or("persistent");
    let fqn = format!("{scheme}://{tenant}/{namespace}/{topic}");
    let pos = match body.position.as_deref() {
        Some("earliest") | None => crate::pulsar_admin::MessageId::EARLIEST,
        Some("latest") => crate::pulsar_admin::MessageId::LATEST,
        Some(other) => match other.parse::<u64>() {
            Ok(n) => crate::pulsar_admin::MessageId::from_offset(n),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid position"})),
                )
                    .into_response();
            }
        },
    };
    match s.pulsar_admin.reset_cursor(&fqn, &sub, pos) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn connector_to_json(c: &Connector) -> serde_json::Value {
    json!({
        "name": c.name,
        "config": c.config,
        "type": format!("{:?}", c.connector_type).to_lowercase(),
        "tasks": c.tasks.iter().map(|t| json!({
            "connector": t.id.connector,
            "task": t.id.task,
        })).collect::<Vec<_>>(),
    })
}
