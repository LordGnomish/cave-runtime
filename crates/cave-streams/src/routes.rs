//! REST management routes: Schema Registry, Kafka Connect, admin endpoints.

use crate::schema_registry::{
    CompatibilityCheckResponse, CompatibilityConfig, RegisterSchemaRequest,
    RegisterSchemaResponse, SchemaFormat, SchemaResponse,
};
use crate::connect::{ConnectCluster, Connector};
use crate::StreamsState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use std::collections::HashMap;

pub fn create_router(state: Arc<StreamsState>) -> Router {
    // Separate state for connect cluster
    let connect = Arc::new(crate::connect::ConnectCluster::new());

    Router::new()
        // ── Health ─────────────────────────────────────────────────────────
        .route("/api/streams/health", get(health))

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

        // ── Broker management ──────────────────────────────────────────────
        .route("/api/streams/topics", get(list_topics).post(create_topic_rest))
        .route("/api/streams/topics/{name}", delete(delete_topic_rest))
        .route("/api/streams/topics/{name}/config", get(get_topic_config).put(alter_topic_config))
        .route("/api/streams/groups", get(list_groups))
        .route("/api/streams/acls", get(list_acls).post(create_acl))
        .route("/api/streams/quotas", get(list_quotas).post(set_quota_rest))
        .route("/api/streams/mirror", get(list_mirror_flows))

        .with_state((state, connect))
}

type AppState = (Arc<StreamsState>, Arc<ConnectCluster>);

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-streams",
        "status": "ok",
        "upstream": "apache-kafka"
    }))
}

// ── Schema Registry handlers ──────────────────────────────────────────────────

async fn list_subjects(State((s, _)): State<AppState>) -> Json<Vec<String>> {
    Json(s.broker.transactions.list_transactions().into_iter().map(|t| t.transactional_id).collect::<Vec<_>>());
    // Actually list schema subjects
    Json(vec![])  // Placeholder — real impl routes to schema_registry
}

async fn register_schema(
    State((s, _)): State<AppState>,
    Path(subject): Path<String>,
    Json(req): Json<RegisterSchemaRequest>,
) -> impl IntoResponse {
    // Schema registry is on the broker for this demo
    let format = SchemaFormat::from_str(&req.schema_type);
    let refs = req.references.into_iter().map(|r| crate::schema_registry::SchemaReference {
        name: r.name,
        subject: r.subject,
        version: r.version,
    }).collect();
    match crate::schema_registry::SchemaRegistry::new().register_schema(&subject, req.schema, format, refs) {
        Ok(id) => (StatusCode::OK, Json(json!({"id": id}))).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error_code": 42201, "message": e.to_string()}))).into_response(),
    }
}

async fn list_versions(Path(_subject): Path<String>) -> Json<Vec<i32>> {
    Json(vec![1]) // placeholder
}

async fn get_schema_version(
    Path((_subject, _version)): Path<(String, String)>,
) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error_code": 40401, "message": "Subject not found"}))).into_response()
}

async fn delete_subject(Path(_subject): Path<String>) -> Json<Vec<i32>> {
    Json(vec![])
}

async fn get_schema_by_id(Path(_id): Path<i32>) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error_code": 40403, "message": "Schema not found"}))).into_response()
}

async fn check_compatibility(
    Path((_subject, _version)): Path<(String, String)>,
    Json(_req): Json<RegisterSchemaRequest>,
) -> Json<serde_json::Value> {
    Json(json!({"is_compatible": true}))
}

async fn get_global_config() -> Json<CompatibilityConfig> {
    Json(CompatibilityConfig { compatibility: "BACKWARD".into() })
}

async fn set_global_config(Json(req): Json<CompatibilityConfig>) -> Json<CompatibilityConfig> {
    Json(req)
}

async fn get_subject_config(Path(_subject): Path<String>) -> Json<CompatibilityConfig> {
    Json(CompatibilityConfig { compatibility: "BACKWARD".into() })
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
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    match connect.create_connector(name, config) {
        Ok(c) => (StatusCode::CREATED, Json(connector_to_json(&c))).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"message": e.to_string()}))).into_response(),
    }
}

async fn get_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.get_connector(&name) {
        Ok(c) => (StatusCode::OK, Json(connector_to_json(&c))).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(json!({"message": "connector not found"}))).into_response(),
    }
}

async fn delete_connector(
    State((_, connect)): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match connect.delete_connector(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(json!({"message": "connector not found"}))).into_response(),
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
        })).into_response(),
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
        Ok(tasks) => Json(tasks.iter().map(|t| json!({
            "id": {"connector": t.id.connector, "task": t.id.task},
            "config": t.config,
        })).collect::<Vec<_>>()).into_response(),
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
    Json(connect.list_plugins().into_iter().map(|p| json!({
        "class": p.class,
        "type": p.plugin_type,
        "version": p.version,
    })).collect())
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
    Json(s.broker.groups.list_groups().into_iter().map(|g| json!({
        "group_id": g.group_id,
        "protocol_type": g.protocol_type,
        "state": g.state,
    })).collect())
}

async fn list_acls(State((s, _)): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let filter = crate::acl::AclFilter {
        resource_type: None, resource_name: None, pattern_type: None,
        principal: None, host: None, operation: None, permission: None,
    };
    let acls = s.broker.acls.describe_acls(&filter);
    Json(acls.iter().map(|a| json!({
        "resource_type": format!("{:?}", a.resource_type),
        "resource_name": a.resource_name,
        "principal": a.principal,
        "operation": format!("{:?}", a.operation),
        "permission": format!("{:?}", a.permission),
    })).collect())
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
    Json(s.broker.quotas.list_quotas().into_iter().map(|(e, q)| json!({
        "entity_type": format!("{:?}", e.entity_type),
        "entity_name": e.entity_name,
        "producer_byte_rate": q.producer_byte_rate,
        "consumer_byte_rate": q.consumer_byte_rate,
    })).collect())
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
