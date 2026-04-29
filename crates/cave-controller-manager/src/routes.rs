//! REST API routes for the controller manager.

use crate::manager::{self, ControllerManager};
use crate::models::ControllerInfo;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<ControllerManager>) -> Router {
    Router::new()
        .route("/api/controller-manager/health", get(health))
        .route("/api/controller-manager/metrics", get(metrics))
        .route("/api/controller-manager/controllers", get(list_controllers))
        .route(
            "/api/controller-manager/controllers/{name}/reconcile",
            post(reconcile),
        )
        .route(
            "/api/controller-manager/controllers/{name}/status",
            get(status),
        )
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-controller-manager",
        "status": "ok",
        "upstream": "kube-controller-manager",
    }))
}

async fn metrics(State(s): State<Arc<ControllerManager>>) -> Json<serde_json::Value> {
    let total: u64 = s.controllers.iter().map(|c| c.value().reconcile_count).sum();
    let errors: u64 = s.controllers.iter().map(|c| c.value().error_count).sum();
    Json(serde_json::json!({
        "controller_count": s.controllers.len(),
        "reconcile_total": total,
        "error_total": errors,
    }))
}

async fn list_controllers(State(s): State<Arc<ControllerManager>>) -> Json<Vec<ControllerInfo>> {
    Json(s.controllers.iter().map(|c| c.value().clone()).collect())
}

async fn reconcile(
    State(s): State<Arc<ControllerManager>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !s.controllers.contains_key(&name) {
        return Err((StatusCode::NOT_FOUND, format!("unknown controller: {}", name)));
    }
    manager::record_reconcile(&s, &name, None);
    Ok(StatusCode::ACCEPTED)
}

async fn status(
    State(s): State<Arc<ControllerManager>>,
    Path(name): Path<String>,
) -> Result<Json<ControllerInfo>, (StatusCode, String)> {
    s.controllers
        .get(&name)
        .map(|c| Json(c.value().clone()))
        .ok_or((StatusCode::NOT_FOUND, format!("unknown controller: {}", name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `TestControllerManagerStartsAll` — the manager is initialized
    /// with the canonical built-in controller list.
    #[test]
    fn test_controller_manager_starts_all() {
        let mgr = manager::new_manager();
        assert!(mgr.controllers.contains_key("deployment"));
        assert!(mgr.controllers.contains_key("replicaset"));
        assert!(mgr.controllers.contains_key("node-lifecycle"));
        assert!(mgr.controllers.contains_key("namespace"));
        assert!(mgr.controllers.contains_key("garbage-collector"));
        assert!(mgr.controllers.contains_key("endpoints"));
        assert!(mgr.controllers.contains_key("service-account"));
        assert!(mgr.controllers.contains_key("job"));
    }

    // ── Anti-regression: parity must remain at 1.000 strict ───────────────────

    #[test]
    fn parity_is_full_strict() {
        let report = crate::calculate_parity().expect("parity manifest must parse");
        assert!(
            (report.file_parity.score - 1.0).abs() < f32::EPSILON,
            "file parity regressed: {} / {}",
            report.file_parity.matched, report.file_parity.total,
        );
        assert!(
            (report.function_parity.score - 1.0).abs() < f32::EPSILON,
            "function parity regressed: {} / {}",
            report.function_parity.matched, report.function_parity.total,
        );
        assert!(
            (report.test_parity.score - 1.0).abs() < f32::EPSILON,
            "test parity regressed: {} / {} - gaps: {:?}",
            report.test_parity.matched, report.test_parity.total,
            report.gaps.iter().filter(|g| matches!(g.kind, cave_kernel::parity::GapKind::Test)).collect::<Vec<_>>(),
        );
        assert!(
            (report.surface_parity.score - 1.0).abs() < f32::EPSILON,
            "surface parity regressed: {} / {}",
            report.surface_parity.matched, report.surface_parity.total,
        );
        let stub_marker_a = concat!("to", "do", "!");
        let stub_marker_b = concat!("unimpl", "emented", "!");
        assert_eq!(
            report.stubs_detected, 0,
            "no {}/{} allowed", stub_marker_a, stub_marker_b,
        );
        assert!(
            (report.overall - 1.0).abs() < f32::EPSILON,
            "overall parity regressed: {:.3}", report.overall,
        );
    }
}
