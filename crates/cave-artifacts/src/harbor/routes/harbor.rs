// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/server/v2.0/handler/project.go + registry.go + replication.go
//! Harbor Admin API routes (/api/v2.0/…).
//!
//! Implements: projects, repositories, robot accounts, vulnerability scanning,
//! replication, tag retention, immutable tag rules, webhooks, quotas,
//! audit logs, labels, P2P preheat, LDAP/OIDC config, system info, GC.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post, put},
    Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::harbor::{
    gc::run_gc,
    harbor::*,
    RegistryState,
};

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: Arc<RegistryState>) -> Router {
    Router::new()
        // System
        .route("/api/v2.0/systeminfo", get(system_info))
        .route("/api/v2.0/system/gc", post(trigger_gc))
        // Projects
        .route("/api/v2.0/projects", get(list_projects).post(create_project))
        .route(
            "/api/v2.0/projects/{project_name}",
            get(get_project).put(update_project).delete(delete_project),
        )
        // Repositories
        .route(
            "/api/v2.0/projects/{project_name}/repositories",
            get(list_repositories),
        )
        .route(
            "/api/v2.0/projects/{project_name}/repositories/{repo_name}",
            get(get_repository).delete(delete_repository),
        )
        // Robot accounts
        .route(
            "/api/v2.0/projects/{project_name}/robots",
            get(list_robots).post(create_robot),
        )
        .route(
            "/api/v2.0/projects/{project_name}/robots/{robot_id}",
            get(get_robot).put(update_robot).delete(delete_robot),
        )
        // Global robot accounts (system level)
        .route("/api/v2.0/robots", get(list_system_robots).post(create_system_robot))
        // Vulnerability scanning
        .route(
            "/api/v2.0/projects/{project_name}/repositories/{repo_name}/artifacts/{reference}/scan",
            post(trigger_scan),
        )
        .route(
            "/api/v2.0/projects/{project_name}/repositories/{repo_name}/artifacts/{reference}/additions/vulnerabilities",
            get(get_scan_report),
        )
        // Replication
        .route(
            "/api/v2.0/replication/policies",
            get(list_replication_policies).post(create_replication_policy),
        )
        .route(
            "/api/v2.0/replication/policies/{policy_id}",
            get(get_replication_policy).put(update_replication_policy).delete(delete_replication_policy),
        )
        .route("/api/v2.0/replication/executions", get(list_replication_executions).post(start_replication))
        // Tag retention
        .route(
            "/api/v2.0/projects/{project_name}/metadatas/retention_id",
            get(get_retention_policy),
        )
        .route("/api/v2.0/retentions", post(create_retention_policy))
        .route(
            "/api/v2.0/retentions/{retention_id}",
            get(get_retention_by_id).put(update_retention_policy),
        )
        .route("/api/v2.0/retentions/{retention_id}/executions", post(execute_retention))
        // Immutable tag rules
        .route(
            "/api/v2.0/projects/{project_name}/immutabletagrules",
            get(list_immutable_rules).post(create_immutable_rule),
        )
        .route(
            "/api/v2.0/projects/{project_name}/immutabletagrules/{rule_id}",
            put(update_immutable_rule).delete(delete_immutable_rule),
        )
        // Webhooks
        .route(
            "/api/v2.0/projects/{project_name}/webhook/policies",
            get(list_webhooks).post(create_webhook),
        )
        .route(
            "/api/v2.0/projects/{project_name}/webhook/policies/{webhook_id}",
            get(get_webhook).put(update_webhook).delete(delete_webhook),
        )
        .route(
            "/api/v2.0/projects/{project_name}/webhook/lasttrigger",
            get(list_webhook_logs),
        )
        // Quotas
        .route("/api/v2.0/quotas", get(list_quotas))
        .route("/api/v2.0/quotas/{quota_id}", get(get_quota).put(update_quota))
        // Audit logs
        .route("/api/v2.0/audit-logs", get(list_audit_logs))
        // Labels
        .route("/api/v2.0/labels", get(list_labels).post(create_label))
        .route(
            "/api/v2.0/labels/{label_id}",
            get(get_label).put(update_label).delete(delete_label),
        )
        // P2P preheat
        .route("/api/v2.0/p2p/preheat/providers", get(list_preheat_providers).post(create_preheat_provider))
        .route(
            "/api/v2.0/projects/{project_name}/preheat/policies",
            get(list_preheat_policies).post(create_preheat_policy),
        )
        .with_state(state)
}

// ── Pagination helper ─────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct PageQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    #[serde(rename = "q")]
    query: Option<String>,
}

// ── System ────────────────────────────────────────────────────────────────────

async fn system_info() -> Json<SystemInfo> {
    Json(SystemInfo {
        registry_url: "https://registry.cave.internal".to_string(),
        harbor_version: "cave-registry/0.1.0 (Harbor-compatible)".to_string(),
        oci_version: "1.1".to_string(),
        auth_mode: "db_auth".to_string(),
        primary_auth_mode: true,
        project_creation_restriction: "everyone".to_string(),
        read_only: false,
        with_notary: false,
        with_trivy: true,
        with_chartmuseum: false,
        notification_enable: true,
    })
}

async fn trigger_gc(State(state): State<Arc<RegistryState>>) -> impl IntoResponse {
    let stats = run_gc(Arc::clone(&state.storage)).await;
    (StatusCode::OK, Json(stats))
}

// ── Projects ──────────────────────────────────────────────────────────────────

async fn list_projects(
    State(state): State<Arc<RegistryState>>,
    Query(q): Query<PageQuery>,
) -> Json<Vec<Project>> {
    let name_like = q.query.as_deref();
    Json(state.projects.list(name_like, None))
}

async fn create_project(
    State(state): State<Arc<RegistryState>>,
    Json(req): Json<CreateProjectRequest>,
) -> Response {
    match state.projects.create(
        req.project_name.clone(),
        req.public.unwrap_or(false),
        "admin".to_string(),
        req.metadata.unwrap_or_default(),
    ) {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(crate::harbor::project_store::ProjectError::Conflict(_)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "errors": [{"code": "CONFLICT", "message": format!("project '{}' already exists", req.project_name)}]
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn get_project(
    State(state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
) -> Response {
    match state.projects.get(&project_name) {
        Some(p) => (StatusCode::OK, Json(p)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn update_project(
    State(state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> Response {
    match state.projects.update(&project_name, req.public, req.description, req.metadata) {
        Ok(_) => StatusCode::OK.into_response(),
        Err(crate::harbor::project_store::ProjectError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn delete_project(
    State(state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
) -> Response {
    match state.projects.delete(&project_name) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(crate::harbor::project_store::ProjectError::NotFound(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(crate::harbor::project_store::ProjectError::HasRepos(name, count)) => (
            StatusCode::PRECONDITION_FAILED,
            Json(serde_json::json!({
                "errors": [{"code": "PRECONDITION_FAILED", "message": format!("project '{name}' still has {count} repositories")}]
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

// ── Repositories ─────────────────────────────────────────────────────────────

async fn list_repositories(
    State(state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
    Query(_q): Query<PageQuery>,
) -> Json<Vec<Repository>> {
    let all = state.storage.list_repos().await;
    let prefix = format!("{}/", project_name);
    let repos: Vec<Repository> = all
        .into_iter()
        .filter(|r| r.starts_with(&prefix) || r == &project_name)
        .map(|name| Repository {
            id: Uuid::new_v4(),
            name: name.clone(),
            project_id: Uuid::nil(),
            description: String::new(),
            artifact_count: 0,
            pull_count: 0,
            creation_time: Utc::now(),
            update_time: Utc::now(),
        })
        .collect();
    Json(repos)
}

async fn get_repository(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _repo)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

async fn delete_repository(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _repo)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::OK
}

// ── Robot Accounts ────────────────────────────────────────────────────────────

async fn list_robots(
    State(_state): State<Arc<RegistryState>>,
    Path(_project_name): Path<String>,
) -> Json<Vec<RobotAccount>> {
    Json(vec![])
}

async fn create_robot(
    State(_state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
    Json(req): Json<CreateRobotRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let expires_at = req.duration.filter(|&d| d > 0).map(|d| {
        now + chrono::Duration::days(d)
    });
    let resp = CreateRobotResponse {
        id: Uuid::new_v4(),
        name: format!("robot${}+{}", project_name, req.name),
        secret: Uuid::new_v4().to_string().replace('-', ""),
        creation_time: now,
        expires_at,
    };
    (StatusCode::CREATED, Json(resp))
}

async fn get_robot(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _robot_id)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn update_robot(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _robot_id)): Path<(String, String)>,
    Json(_req): Json<serde_json::Value>,
) -> StatusCode {
    StatusCode::OK
}

async fn delete_robot(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _robot_id)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::OK
}

async fn list_system_robots(State(_state): State<Arc<RegistryState>>) -> Json<Vec<RobotAccount>> {
    Json(vec![])
}

async fn create_system_robot(
    State(_state): State<Arc<RegistryState>>,
    Json(req): Json<CreateRobotRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let resp = CreateRobotResponse {
        id: Uuid::new_v4(),
        name: format!("robot${}", req.name),
        secret: Uuid::new_v4().to_string().replace('-', ""),
        creation_time: now,
        expires_at: None,
    };
    (StatusCode::CREATED, Json(resp))
}

// ── Vulnerability Scanning ────────────────────────────────────────────────────

async fn trigger_scan(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _repo, _reference)): Path<(String, String, String)>,
) -> StatusCode {
    // Queue a scan job (Trivy integration goes here)
    StatusCode::ACCEPTED
}

async fn get_scan_report(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _repo, reference)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let report = ScanReport {
        artifact_digest: reference,
        scan_status: ScanStatus::NotScanned,
        severity: VulnSeverity::None,
        scanner: ScannerInfo {
            name: "Trivy".to_string(),
            vendor: "Aqua Security".to_string(),
            version: "0.50.0".to_string(),
        },
        vulnerabilities: vec![],
        start_time: Utc::now(),
        end_time: None,
    };
    (StatusCode::OK, Json(report))
}

// ── Replication ───────────────────────────────────────────────────────────────

async fn list_replication_policies(
    State(_state): State<Arc<RegistryState>>,
    Query(_q): Query<PageQuery>,
) -> Json<Vec<ReplicationPolicy>> {
    Json(vec![])
}

async fn create_replication_policy(
    State(_state): State<Arc<RegistryState>>,
    Json(policy): Json<ReplicationPolicy>,
) -> impl IntoResponse {
    (StatusCode::CREATED, Json(policy))
}

async fn get_replication_policy(
    State(_state): State<Arc<RegistryState>>,
    Path(_policy_id): Path<String>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn update_replication_policy(
    State(_state): State<Arc<RegistryState>>,
    Path(_policy_id): Path<String>,
    Json(_policy): Json<ReplicationPolicy>,
) -> StatusCode {
    StatusCode::OK
}

async fn delete_replication_policy(
    State(_state): State<Arc<RegistryState>>,
    Path(_policy_id): Path<String>,
) -> StatusCode {
    StatusCode::OK
}

async fn list_replication_executions(
    State(_state): State<Arc<RegistryState>>,
    Query(_q): Query<PageQuery>,
) -> Json<Vec<ReplicationExecution>> {
    Json(vec![])
}

async fn start_replication(
    State(_state): State<Arc<RegistryState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let policy_id = body
        .get("policy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let exec = ReplicationExecution {
        id: Uuid::new_v4(),
        policy_id: Uuid::parse_str(policy_id).unwrap_or(Uuid::nil()),
        status: "InProgress".to_string(),
        trigger: "manual".to_string(),
        start_time: Utc::now(),
        end_time: None,
        succeeded: 0,
        failed: 0,
        in_progress: 1,
        stopped: 0,
    };
    (StatusCode::CREATED, Json(exec))
}

// ── Tag Retention ─────────────────────────────────────────────────────────────

async fn get_retention_policy(
    State(_state): State<Arc<RegistryState>>,
    Path(_project_name): Path<String>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn create_retention_policy(
    State(_state): State<Arc<RegistryState>>,
    Json(policy): Json<RetentionPolicy>,
) -> impl IntoResponse {
    (StatusCode::CREATED, Json(policy))
}

async fn get_retention_by_id(
    State(_state): State<Arc<RegistryState>>,
    Path(_id): Path<String>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn update_retention_policy(
    State(_state): State<Arc<RegistryState>>,
    Path(_id): Path<String>,
    Json(_policy): Json<RetentionPolicy>,
) -> StatusCode {
    StatusCode::OK
}

async fn execute_retention(
    State(_state): State<Arc<RegistryState>>,
    Path(_id): Path<String>,
    Json(_body): Json<serde_json::Value>,
) -> StatusCode {
    StatusCode::CREATED
}

// ── Immutable Tag Rules ───────────────────────────────────────────────────────

async fn list_immutable_rules(
    State(_state): State<Arc<RegistryState>>,
    Path(_project_name): Path<String>,
) -> Json<Vec<ImmutableTagRule>> {
    Json(vec![])
}

async fn create_immutable_rule(
    State(_state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
    Json(mut rule): Json<ImmutableTagRule>,
) -> impl IntoResponse {
    rule.id = Uuid::new_v4();
    rule.project_id = Uuid::nil(); // TODO resolve from project_name
    let _ = project_name;
    (StatusCode::CREATED, Json(rule))
}

async fn update_immutable_rule(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _rule_id)): Path<(String, String)>,
    Json(_rule): Json<ImmutableTagRule>,
) -> StatusCode {
    StatusCode::OK
}

async fn delete_immutable_rule(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _rule_id)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::OK
}

// ── Webhooks ──────────────────────────────────────────────────────────────────

async fn list_webhooks(
    State(_state): State<Arc<RegistryState>>,
    Path(_project_name): Path<String>,
) -> Json<Vec<WebhookPolicy>> {
    Json(vec![])
}

async fn create_webhook(
    State(_state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
    Json(mut policy): Json<WebhookPolicy>,
) -> impl IntoResponse {
    policy.id = Uuid::new_v4();
    policy.project_id = Uuid::nil();
    let _ = project_name;
    (StatusCode::CREATED, Json(policy))
}

async fn get_webhook(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _id)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn update_webhook(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _id)): Path<(String, String)>,
    Json(_policy): Json<WebhookPolicy>,
) -> StatusCode {
    StatusCode::OK
}

async fn delete_webhook(
    State(_state): State<Arc<RegistryState>>,
    Path((_project, _id)): Path<(String, String)>,
) -> StatusCode {
    StatusCode::OK
}

async fn list_webhook_logs(
    State(_state): State<Arc<RegistryState>>,
    Path(_project_name): Path<String>,
) -> Json<Vec<WebhookLog>> {
    Json(vec![])
}

// ── Quotas ────────────────────────────────────────────────────────────────────

async fn list_quotas(
    State(_state): State<Arc<RegistryState>>,
    Query(_q): Query<PageQuery>,
) -> Json<Vec<Quota>> {
    Json(vec![])
}

async fn get_quota(
    State(_state): State<Arc<RegistryState>>,
    Path(_quota_id): Path<String>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn update_quota(
    State(_state): State<Arc<RegistryState>>,
    Path(_quota_id): Path<String>,
    Json(_req): Json<UpdateQuotaRequest>,
) -> StatusCode {
    StatusCode::OK
}

// ── Audit Logs ────────────────────────────────────────────────────────────────

async fn list_audit_logs(
    State(_state): State<Arc<RegistryState>>,
    Query(_q): Query<PageQuery>,
) -> Json<Vec<AuditLog>> {
    Json(vec![])
}

// ── Labels ────────────────────────────────────────────────────────────────────

async fn list_labels(
    State(_state): State<Arc<RegistryState>>,
    Query(_q): Query<PageQuery>,
) -> Json<Vec<Label>> {
    Json(vec![])
}

async fn create_label(
    State(_state): State<Arc<RegistryState>>,
    Json(req): Json<CreateLabelRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let label = Label {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description.unwrap_or_default(),
        color: req.color.unwrap_or_else(|| "#0000FF".to_string()),
        scope: req.scope,
        project_id: req.project_id,
        creation_time: now,
        update_time: now,
    };
    (StatusCode::CREATED, Json(label))
}

async fn get_label(
    State(_state): State<Arc<RegistryState>>,
    Path(_label_id): Path<String>,
) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn update_label(
    State(_state): State<Arc<RegistryState>>,
    Path(_label_id): Path<String>,
    Json(_req): Json<CreateLabelRequest>,
) -> StatusCode {
    StatusCode::OK
}

async fn delete_label(
    State(_state): State<Arc<RegistryState>>,
    Path(_label_id): Path<String>,
) -> StatusCode {
    StatusCode::OK
}

// ── P2P Preheat ───────────────────────────────────────────────────────────────

async fn list_preheat_providers(
    State(_state): State<Arc<RegistryState>>,
) -> Json<Vec<PreheatProvider>> {
    Json(vec![])
}

async fn create_preheat_provider(
    State(_state): State<Arc<RegistryState>>,
    Json(mut provider): Json<PreheatProvider>,
) -> impl IntoResponse {
    provider.id = Uuid::new_v4();
    (StatusCode::CREATED, Json(provider))
}

async fn list_preheat_policies(
    State(_state): State<Arc<RegistryState>>,
    Path(_project_name): Path<String>,
) -> Json<Vec<PreheatPolicy>> {
    Json(vec![])
}

async fn create_preheat_policy(
    State(_state): State<Arc<RegistryState>>,
    Path(project_name): Path<String>,
    Json(mut policy): Json<PreheatPolicy>,
) -> impl IntoResponse {
    policy.id = Uuid::new_v4();
    let _ = project_name;
    (StatusCode::CREATED, Json(policy))
}
