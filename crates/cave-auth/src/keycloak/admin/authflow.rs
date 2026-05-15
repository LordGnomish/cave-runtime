// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/services/resources/admin/AuthenticationManagementResource.java
//
//! Authentication-flow REST CRUD.
//!
//! Routes:
//!   GET   /admin/realms/{realm}/authentication/flows
//!   POST  /admin/realms/{realm}/authentication/flows
//!   GET   /admin/realms/{realm}/authentication/flows/{flowAlias}
//!   PUT   /admin/realms/{realm}/authentication/flows/{flowAlias}
//!   DELETE /admin/realms/{realm}/authentication/flows/{flowAlias}
//!   GET   /admin/realms/{realm}/authentication/flows/{flowAlias}/executions
//!   POST  /admin/realms/{realm}/authentication/flows/{flowAlias}/executions
//!   PUT   /admin/realms/{realm}/authentication/flows/{flowAlias}/executions
//!   POST  /admin/realms/{realm}/authentication/flows/{flowAlias}/copy

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ─── Models ───────────────────────────────────────────────────────────────────

/// Keycloak AuthenticationExecutionModel.Requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Requirement {
    Required,
    Alternative,
    Optional,
    Disabled,
    Conditional,
}

/// AuthenticationExecutionRepresentation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionModel {
    #[serde(default)]
    pub id: Option<String>,
    pub authenticator: String,
    #[serde(default)]
    pub authenticator_config: Option<String>,
    pub requirement: Requirement,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub flow_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Default for Requirement {
    fn default() -> Self { Requirement::Disabled }
}

/// AuthenticationFlowRepresentation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowModel {
    #[serde(default)]
    pub id: Option<String>,
    pub alias: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_provider")]
    pub provider_id: String,
    #[serde(default = "default_true")]
    pub top_level: bool,
    #[serde(default)]
    pub built_in: bool,
    #[serde(default)]
    pub authentication_executions: Vec<ExecutionModel>,
}

fn default_true() -> bool { true }
fn default_provider() -> String { "basic-flow".to_string() }

// ─── Store + service ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FlowService {
    inner: Arc<RwLock<HashMap<(String, String), FlowModel>>>,
}

impl FlowService {
    pub fn new() -> Self {
        let svc = Self { inner: Arc::new(RwLock::new(HashMap::new())) };
        // Seed built-in flows in `master` realm so admin UI has something to render.
        let store = svc.inner.clone();
        tokio::spawn(async move {
            let mut w = store.write().await;
            for flow in builtin_flows() {
                w.insert(("master".to_string(), flow.alias.clone()), flow);
            }
        });
        svc
    }
    pub fn cloned(&self) -> Self { self.clone() }

    pub async fn list(&self, realm: &str) -> Vec<FlowModel> {
        self.inner.read().await.iter()
            .filter(|((r, _), _)| r == realm)
            .map(|(_, v)| v.clone())
            .collect()
    }
    pub async fn get(&self, realm: &str, alias: &str) -> Option<FlowModel> {
        self.inner.read().await.get(&(realm.to_string(), alias.to_string())).cloned()
    }
    pub async fn create(&self, realm: &str, mut flow: FlowModel) -> Result<FlowModel, &'static str> {
        if flow.alias.is_empty() { return Err("alias_required"); }
        let mut store = self.inner.write().await;
        if store.contains_key(&(realm.to_string(), flow.alias.clone())) {
            return Err("alias_exists");
        }
        flow.id = Some(format!("flow-{}", uuid::Uuid::new_v4()));
        for (i, exec) in flow.authentication_executions.iter_mut().enumerate() {
            if exec.id.is_none() {
                exec.id = Some(format!("exec-{}", uuid::Uuid::new_v4()));
            }
            if exec.priority == 0 { exec.priority = (i as i32 + 1) * 10; }
        }
        store.insert((realm.to_string(), flow.alias.clone()), flow.clone());
        Ok(flow)
    }
    pub async fn update(&self, realm: &str, alias: &str, mut flow: FlowModel) -> Result<FlowModel, &'static str> {
        let mut store = self.inner.write().await;
        let key = (realm.to_string(), alias.to_string());
        let prev = store.get(&key).ok_or("not_found")?.clone();
        if prev.built_in { return Err("cannot_edit_builtin"); }
        flow.alias = alias.to_string();
        flow.id = prev.id;
        store.insert(key, flow.clone());
        Ok(flow)
    }
    pub async fn delete(&self, realm: &str, alias: &str) -> Result<(), &'static str> {
        let mut store = self.inner.write().await;
        let key = (realm.to_string(), alias.to_string());
        let prev = store.get(&key).ok_or("not_found")?.clone();
        if prev.built_in { return Err("cannot_delete_builtin"); }
        store.remove(&key);
        Ok(())
    }
    pub async fn copy(&self, realm: &str, src: &str, new_alias: &str) -> Result<FlowModel, &'static str> {
        let mut flow = self.get(realm, src).await.ok_or("not_found")?;
        flow.alias = new_alias.to_string();
        flow.built_in = false;
        flow.id = None;
        // Reassign exec ids so the copy is independent.
        for exec in &mut flow.authentication_executions {
            exec.id = None;
        }
        self.create(realm, flow).await
    }

    pub async fn list_executions(&self, realm: &str, alias: &str) -> Option<Vec<ExecutionModel>> {
        self.get(realm, alias).await.map(|f| f.authentication_executions)
    }

    pub async fn add_execution(&self, realm: &str, alias: &str, mut exec: ExecutionModel) -> Result<ExecutionModel, &'static str> {
        let mut store = self.inner.write().await;
        let key = (realm.to_string(), alias.to_string());
        let flow = store.get_mut(&key).ok_or("not_found")?;
        if flow.built_in { return Err("cannot_edit_builtin"); }
        if exec.id.is_none() { exec.id = Some(format!("exec-{}", uuid::Uuid::new_v4())); }
        if exec.priority == 0 {
            let max = flow.authentication_executions.iter().map(|e| e.priority).max().unwrap_or(0);
            exec.priority = max + 10;
        }
        flow.authentication_executions.push(exec.clone());
        flow.authentication_executions.sort_by_key(|e| e.priority);
        Ok(exec)
    }

    pub async fn replace_executions(
        &self,
        realm: &str,
        alias: &str,
        execs: Vec<ExecutionModel>,
    ) -> Result<Vec<ExecutionModel>, &'static str> {
        let mut store = self.inner.write().await;
        let key = (realm.to_string(), alias.to_string());
        let flow = store.get_mut(&key).ok_or("not_found")?;
        if flow.built_in { return Err("cannot_edit_builtin"); }
        let mut normalised = execs;
        normalised.sort_by_key(|e| e.priority);
        flow.authentication_executions = normalised.clone();
        Ok(normalised)
    }
}

impl Default for FlowService {
    fn default() -> Self { Self::new() }
}

/// Built-in flows shipped by Keycloak (subset — the most-used six).
pub fn builtin_flows() -> Vec<FlowModel> {
    let mk = |alias: &str, description: &str, execs: &[(&str, Requirement)]| FlowModel {
        id: Some(format!("builtin-{alias}")),
        alias: alias.into(),
        description: Some(description.into()),
        provider_id: "basic-flow".into(),
        top_level: true,
        built_in: true,
        authentication_executions: execs.iter().enumerate().map(|(i, (a, r))| ExecutionModel {
            id: Some(format!("builtin-{}-{}", alias, i)),
            authenticator: (*a).into(),
            authenticator_config: None,
            requirement: *r,
            priority: (i as i32 + 1) * 10,
            flow_id: None,
            description: None,
        }).collect(),
    };
    vec![
        mk("browser", "browser based authentication", &[
            ("auth-cookie", Requirement::Alternative),
            ("auth-spnego", Requirement::Disabled),
            ("identity-provider-redirector", Requirement::Alternative),
            ("forms", Requirement::Alternative),
        ]),
        mk("direct grant", "OpenID Connect Resource Owner Grant", &[
            ("direct-grant-validate-username", Requirement::Required),
            ("direct-grant-validate-password", Requirement::Required),
            ("direct-grant-validate-otp", Requirement::Conditional),
        ]),
        mk("registration", "registration flow", &[
            ("registration-page-form", Requirement::Required),
        ]),
        mk("reset credentials", "Reset credentials", &[
            ("reset-credentials-choose-user", Requirement::Required),
            ("reset-credential-email", Requirement::Required),
            ("reset-password", Requirement::Required),
        ]),
        mk("clients", "Base authentication for clients", &[
            ("client-secret", Requirement::Alternative),
            ("client-jwt", Requirement::Alternative),
            ("client-secret-jwt", Requirement::Alternative),
            ("client-x509", Requirement::Alternative),
        ]),
        mk("first broker login", "Actions taken after first broker login", &[
            ("review profile", Requirement::Required),
            ("create or link", Requirement::Required),
        ]),
    ]
}

// ─── HTTP handlers ────────────────────────────────────────────────────────────

pub async fn list_flows(
    State(svc): State<FlowService>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    super::super::metrics::inc_flow_op("list", "ok");
    Json(svc.list(&realm).await)
}

pub async fn create_flow(
    State(svc): State<FlowService>,
    Path(realm): Path<String>,
    Json(flow): Json<FlowModel>,
) -> impl IntoResponse {
    match svc.create(&realm, flow).await {
        Ok(f) => {
            super::super::metrics::inc_flow_op("create", "ok");
            (StatusCode::CREATED, Json(f)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_flow_op("create", "fail");
            (StatusCode::CONFLICT, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub async fn get_flow(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match svc.get(&realm, &alias).await {
        Some(f) => {
            super::super::metrics::inc_flow_op("get", "ok");
            (StatusCode::OK, Json(f)).into_response()
        }
        None => {
            super::super::metrics::inc_flow_op("get", "not_found");
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response()
        }
    }
}

pub async fn update_flow(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
    Json(flow): Json<FlowModel>,
) -> impl IntoResponse {
    match svc.update(&realm, &alias, flow).await {
        Ok(f) => {
            super::super::metrics::inc_flow_op("update", "ok");
            (StatusCode::OK, Json(f)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_flow_op("update", "fail");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub async fn delete_flow(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match svc.delete(&realm, &alias).await {
        Ok(()) => {
            super::super::metrics::inc_flow_op("delete", "ok");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            super::super::metrics::inc_flow_op("delete", "fail");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct CopyRequest {
    pub new_name: String,
}

pub async fn copy_flow(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
    Json(req): Json<CopyRequest>,
) -> impl IntoResponse {
    match svc.copy(&realm, &alias, &req.new_name).await {
        Ok(f) => {
            super::super::metrics::inc_flow_op("copy", "ok");
            (StatusCode::CREATED, Json(f)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_flow_op("copy", "fail");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub async fn list_executions(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match svc.list_executions(&realm, &alias).await {
        Some(execs) => {
            super::super::metrics::inc_flow_op("list_execs", "ok");
            (StatusCode::OK, Json(execs)).into_response()
        }
        None => {
            super::super::metrics::inc_flow_op("list_execs", "not_found");
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response()
        }
    }
}

pub async fn add_execution(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
    Json(exec): Json<ExecutionModel>,
) -> impl IntoResponse {
    match svc.add_execution(&realm, &alias, exec).await {
        Ok(e) => {
            super::super::metrics::inc_flow_op("add_exec", "ok");
            (StatusCode::CREATED, Json(e)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_flow_op("add_exec", "fail");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub async fn replace_executions(
    State(svc): State<FlowService>,
    Path((realm, alias)): Path<(String, String)>,
    Json(execs): Json<Vec<ExecutionModel>>,
) -> impl IntoResponse {
    match svc.replace_executions(&realm, &alias, execs).await {
        Ok(out) => {
            super::super::metrics::inc_flow_op("reorder", "ok");
            (StatusCode::OK, Json(out)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_flow_op("reorder", "fail");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub fn router(svc: FlowService) -> Router {
    Router::new()
        .route("/admin/realms/{realm}/authentication/flows",
               get(list_flows).post(create_flow))
        .route("/admin/realms/{realm}/authentication/flows/{flowAlias}",
               get(get_flow).put(update_flow).delete(delete_flow))
        .route("/admin/realms/{realm}/authentication/flows/{flowAlias}/copy",
               post(copy_flow))
        .route("/admin/realms/{realm}/authentication/flows/{flowAlias}/executions",
               get(list_executions).post(add_execution).put(replace_executions))
        .with_state(svc)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_flow_assigns_id() {
        let svc = FlowService::new();
        let f = svc.create("r", FlowModel {
            alias: "custom".into(), ..Default::default()
        }).await.unwrap();
        assert!(f.id.is_some());
    }

    #[tokio::test]
    async fn duplicate_alias_errors() {
        let svc = FlowService::new();
        svc.create("r", FlowModel { alias: "x".into(), ..Default::default() }).await.unwrap();
        let err = svc.create("r", FlowModel { alias: "x".into(), ..Default::default() }).await.unwrap_err();
        assert_eq!(err, "alias_exists");
    }

    #[tokio::test]
    async fn cannot_delete_builtin() {
        let svc = FlowService::new();
        let _ = svc.create("master", FlowModel {
            alias: "browser".into(),
            built_in: true,
            ..Default::default()
        }).await;
        // Built-in flow registered manually for this test (the spawn-task seed is async).
        let err = svc.delete("master", "browser").await.unwrap_err();
        assert_eq!(err, "cannot_delete_builtin");
    }

    #[tokio::test]
    async fn copy_flow_creates_independent_clone() {
        let svc = FlowService::new();
        svc.create("r", FlowModel {
            alias: "src".into(),
            authentication_executions: vec![ExecutionModel {
                authenticator: "auth-cookie".into(),
                requirement: Requirement::Required,
                priority: 10, ..Default::default()
            }],
            ..Default::default()
        }).await.unwrap();
        let copy = svc.copy("r", "src", "dst").await.unwrap();
        assert_eq!(copy.alias, "dst");
        assert!(!copy.built_in);
        assert_eq!(copy.authentication_executions.len(), 1);
    }

    #[tokio::test]
    async fn add_and_reorder_executions() {
        let svc = FlowService::new();
        svc.create("r", FlowModel { alias: "f".into(), ..Default::default() }).await.unwrap();
        svc.add_execution("r", "f", ExecutionModel {
            authenticator: "auth-cookie".into(),
            requirement: Requirement::Alternative, priority: 10, ..Default::default()
        }).await.unwrap();
        svc.add_execution("r", "f", ExecutionModel {
            authenticator: "forms".into(),
            requirement: Requirement::Alternative, priority: 20, ..Default::default()
        }).await.unwrap();
        let execs = svc.list_executions("r", "f").await.unwrap();
        assert_eq!(execs[0].priority, 10);
        assert_eq!(execs[1].priority, 20);

        // Reorder.
        let reordered = svc.replace_executions("r", "f", vec![
            ExecutionModel { authenticator: "forms".into(), requirement: Requirement::Required, priority: 5, ..Default::default() },
            ExecutionModel { authenticator: "auth-cookie".into(), requirement: Requirement::Alternative, priority: 25, ..Default::default() },
        ]).await.unwrap();
        assert_eq!(reordered[0].priority, 5);
        assert_eq!(reordered[0].authenticator, "forms");
    }

    #[test]
    fn builtin_flow_set_includes_six_flows() {
        let bf = builtin_flows();
        let names: Vec<&str> = bf.iter().map(|f| f.alias.as_str()).collect();
        assert!(names.contains(&"browser"));
        assert!(names.contains(&"direct grant"));
        assert!(names.contains(&"registration"));
        assert!(names.contains(&"reset credentials"));
        assert!(names.contains(&"clients"));
        assert!(names.contains(&"first broker login"));
    }

    #[test]
    fn requirement_serde_uppercase() {
        let j = serde_json::to_string(&Requirement::Alternative).unwrap();
        assert_eq!(j, "\"ALTERNATIVE\"");
        let r: Requirement = serde_json::from_str("\"REQUIRED\"").unwrap();
        assert_eq!(r, Requirement::Required);
    }
}
