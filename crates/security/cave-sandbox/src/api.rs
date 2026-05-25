// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-sandbox HTTP API — axum router for the control plane.
//!
//! Routes:
//!  - `POST /sandbox` create
//!  - `GET  /sandbox/{id}` describe
//!  - `DELETE /sandbox/{id}` remove
//!  - `POST /sandbox/{id}/exec` exec command
//!  - `POST /sandbox/{id}/pause` pause
//!  - `POST /sandbox/{id}/resume` resume

use crate::lifecycle::LifecycleState;
use crate::models::{Runtime, Sandbox, SandboxState};
use crate::store::{SandboxStore, StoredSandbox};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// API state — exposes the store.
#[derive(Clone, Default)]
pub struct ApiState {
    pub store: SandboxStore,
}

impl ApiState {
    pub fn new() -> Self { ApiState::default() }
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub id: Option<String>,
    pub runtime: Runtime,
    pub bundle: String,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateResponse {
    pub id: String,
    pub state: SandboxState,
}

#[derive(Debug, Deserialize)]
pub struct ExecRequest {
    pub argv: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecResponse {
    pub accepted: bool,
    pub exec_id: String,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/sandbox", post(create))
        .route("/sandbox", get(list))
        .route("/sandbox/{id}", get(describe).delete(remove))
        .route("/sandbox/{id}/exec", post(exec))
        .route("/sandbox/{id}/pause", post(pause))
        .route("/sandbox/{id}/resume", post(resume))
        .with_state(state)
}

async fn create(State(s): State<ApiState>, Json(req): Json<CreateRequest>) -> impl IntoResponse {
    let id = req.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let sandbox = Sandbox {
        id: id.clone(),
        runtime: req.runtime,
        state: SandboxState::Created,
        bundle: req.bundle,
        annotations: req.annotations,
    };
    s.store.put(sandbox);
    (StatusCode::CREATED, Json(CreateResponse { id, state: SandboxState::Created }))
}

async fn list(State(s): State<ApiState>) -> impl IntoResponse {
    let items: Vec<StoredSandbox> = s.store.list();
    (StatusCode::OK, Json(items))
}

async fn describe(State(s): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    match s.store.get(&id) {
        Some(s) => (StatusCode::OK, Json(s)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn remove(State(s): State<ApiState>, Path(id): Path<String>) -> StatusCode {
    if s.store.get(&id).is_none() {
        return StatusCode::NOT_FOUND;
    }
    // Drive to terminal first so audit-trail captures it.
    let _ = s.store.transition(&id, LifecycleState::Stopped, Some("api-delete".into()));
    let _ = s.store.transition(&id, LifecycleState::Removed, Some("api-delete".into()));
    s.store.remove(&id);
    StatusCode::NO_CONTENT
}

async fn exec(Path(id): Path<String>, State(s): State<ApiState>, Json(req): Json<ExecRequest>) -> impl IntoResponse {
    if s.store.get(&id).is_none() {
        return (StatusCode::NOT_FOUND, Json(ExecResponse { accepted: false, exec_id: String::new() }));
    }
    if req.argv.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ExecResponse { accepted: false, exec_id: String::new() }));
    }
    // exec is accepted but actual process spawn is OUT OF SCOPE.
    let exec_id = uuid::Uuid::new_v4().to_string();
    (StatusCode::ACCEPTED, Json(ExecResponse { accepted: true, exec_id }))
}

async fn pause(Path(id): Path<String>, State(s): State<ApiState>) -> StatusCode {
    match s.store.transition(&id, LifecycleState::Paused, Some("api-pause".into())) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) if e == "not-found" => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::CONFLICT,
    }
}

async fn resume(Path(id): Path<String>, State(s): State<ApiState>) -> StatusCode {
    match s.store.transition(&id, LifecycleState::Running, Some("api-resume".into())) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) if e == "not-found" => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::CONFLICT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    fn json_body<T: Serialize>(t: &T) -> Body { Body::from(serde_json::to_vec(t).unwrap()) }

    #[tokio::test]
    async fn create_then_describe() {
        let state = ApiState::new();
        let app = router(state.clone());
        let req = Request::builder()
            .method("POST").uri("/sandbox").header("content-type", "application/json")
            .body(json_body(&serde_json::json!({"runtime":"Gvisor","bundle":"/b"}))).unwrap();
        let r = app.clone().oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        let bytes = to_bytes(r.into_body(), 1024).await.unwrap();
        let cr: CreateResponse = serde_json::from_slice(&bytes).unwrap();
        let id = cr.id;

        let req = Request::builder().uri(format!("/sandbox/{id}")).body(Body::empty()).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn delete_removes() {
        let state = ApiState::new();
        let app = router(state.clone());
        state.store.put(Sandbox {
            id: "s".into(), runtime: Runtime::Kata, state: SandboxState::Created,
            bundle: "/b".into(), annotations: Default::default(),
        });
        let req = Request::builder().method("DELETE").uri("/sandbox/s").body(Body::empty()).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(state.store.get("s").is_none());
    }

    #[tokio::test]
    async fn exec_no_argv_400s() {
        let state = ApiState::new();
        state.store.put(Sandbox {
            id: "s".into(), runtime: Runtime::Gvisor, state: SandboxState::Running,
            bundle: "/b".into(), annotations: Default::default(),
        });
        let app = router(state);
        let req = Request::builder().method("POST").uri("/sandbox/s/exec")
            .header("content-type", "application/json")
            .body(json_body(&serde_json::json!({"argv":[]}))).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn exec_accepts() {
        let state = ApiState::new();
        state.store.put(Sandbox {
            id: "s".into(), runtime: Runtime::Gvisor, state: SandboxState::Running,
            bundle: "/b".into(), annotations: Default::default(),
        });
        let app = router(state);
        let req = Request::builder().method("POST").uri("/sandbox/s/exec")
            .header("content-type", "application/json")
            .body(json_body(&serde_json::json!({"argv":["ls"]}))).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn pause_resume_chain() {
        let state = ApiState::new();
        state.store.put(Sandbox {
            id: "s".into(), runtime: Runtime::Kata, state: SandboxState::Created,
            bundle: "/b".into(), annotations: Default::default(),
        });
        // first drive to running:
        state.store.transition("s", LifecycleState::Running, None).unwrap();
        let app = router(state.clone());
        let req = Request::builder().method("POST").uri("/sandbox/s/pause").body(Body::empty()).unwrap();
        let r = app.clone().oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(state.store.get("s").unwrap().sandbox.state, SandboxState::Paused);
        let req = Request::builder().method("POST").uri("/sandbox/s/resume").body(Body::empty()).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(state.store.get("s").unwrap().sandbox.state, SandboxState::Running);
    }

    #[tokio::test]
    async fn describe_missing_404() {
        let app = router(ApiState::new());
        let req = Request::builder().uri("/sandbox/missing").body(Body::empty()).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_initially_empty() {
        let app = router(ApiState::new());
        let req = Request::builder().uri("/sandbox").body(Body::empty()).unwrap();
        let r = app.oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let bytes = to_bytes(r.into_body(), 4096).await.unwrap();
        let v: Vec<StoredSandbox> = serde_json::from_slice(&bytes).unwrap();
        assert!(v.is_empty());
    }
}
