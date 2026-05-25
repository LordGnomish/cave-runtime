// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/AuthenticationManagementResource.java
//
// Mounted at `/admin/realms/{realm}/authentication`.
//
//   /flows                                 -> flows::*
//   /flows/{flowAlias}/executions          -> executions::*
//   /flows/{flowAlias}/executions/execution-> executions::add_execution
//   /required-actions                      -> required_actions::*

pub mod executions;
pub mod flows;
pub mod required_actions;

use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;

pub use executions::{AuthenticationExecution, ExecutionStore, Requirement};
pub use flows::{AuthenticationFlow, AuthenticationFlowStore};
pub use required_actions::{RequiredActionProvider, RequiredActionStore};

#[derive(Clone)]
pub struct AdminFlowsState {
    pub flows: Arc<AuthenticationFlowStore>,
    pub executions: Arc<ExecutionStore>,
    pub required_actions: Arc<RequiredActionStore>,
}

impl AdminFlowsState {
    pub fn new() -> Self {
        Self {
            flows: Arc::new(AuthenticationFlowStore::new()),
            executions: Arc::new(ExecutionStore::new()),
            required_actions: Arc::new(RequiredActionStore::new()),
        }
    }
}

impl Default for AdminFlowsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the admin-flows router. Mount with
/// `app.merge(cave_auth::admin_flows::router(state))`.
pub fn router(state: AdminFlowsState) -> Router {
    let flows_r = Router::new()
        .route(
            "/admin/realms/{realm}/authentication/flows",
            get(flows::list_flows).post(flows::create_flow),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{id}",
            get(flows::get_flow)
                .put(flows::update_flow)
                .delete(flows::delete_flow),
        )
        .with_state(state.flows);

    let exec_r = Router::new()
        .route(
            "/admin/realms/{realm}/authentication/flows/{flow_alias}/executions",
            get(executions::list_executions),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{flow_alias}/executions/execution",
            post(executions::add_execution),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{flow_alias}/executions/{id}",
            axum::routing::delete(executions::delete_execution),
        )
        .with_state(state.executions);

    let ra_r = Router::new()
        .route(
            "/admin/realms/{realm}/authentication/required-actions",
            get(required_actions::list_required_actions),
        )
        .route(
            "/admin/realms/{realm}/authentication/required-actions/{alias}",
            get(required_actions::get_required_action)
                .put(required_actions::update_required_action)
                .delete(required_actions::delete_required_action),
        )
        .with_state(state.required_actions);

    Router::new().merge(flows_r).merge(exec_r).merge(ra_r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_default_constructs() {
        let s = AdminFlowsState::default();
        assert!(s.flows.list("master").is_empty());
        assert!(s.executions.list("master", "browser").is_empty());
        assert!(s.required_actions.list("master").is_empty());
    }

    #[test]
    fn router_builds_without_panic() {
        let _ = router(AdminFlowsState::new());
    }
}
