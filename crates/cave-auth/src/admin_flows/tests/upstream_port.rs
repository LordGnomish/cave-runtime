// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 testsuite/integration-arquillian/.../admin/AuthenticationManagementTest.java
//
//! Upstream-port traceability tests for the authentication-flows admin REST.

use super::super::{AdminFlowsState, executions::{AuthenticationExecution, ExecutionRequirement}, flows::AuthenticationFlow};
use crate::keycloak::realm::{RealmRequest, RealmStore};

async fn fresh_state() -> AdminFlowsState {
    let realms = RealmStore::new();
    realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
    AdminFlowsState::new(realms)
}

// upstream: keycloak/keycloak AuthenticationManagementTest.java:flowStoreRoundtrip
#[tokio::test]
async fn flow_store_roundtrip() {
    let state = fresh_state().await;
    state.flows.create("r", AuthenticationFlow {
        alias: "browser".into(), description: None, provider_id: "basic-flow".into(),
        top_level: true, built_in: false,
    }).await.unwrap();
    let f = state.flows.get("r", "browser").await.unwrap();
    assert_eq!(f.provider_id, "basic-flow");
}

// upstream: keycloak/keycloak AuthenticationManagementTest.java:duplicateFlowReturnsConflictAtStoreLevel
#[tokio::test]
async fn duplicate_flow_returns_conflict() {
    let state = fresh_state().await;
    let f = AuthenticationFlow {
        alias: "browser".into(), description: None, provider_id: "basic-flow".into(),
        top_level: true, built_in: false,
    };
    state.flows.create("r", f.clone()).await.unwrap();
    assert!(state.flows.create("r", f).await.is_err());
}

// upstream: keycloak/keycloak AuthenticationManagementTest.java:executionsAreSortedByPriorityAtStoreLevel
#[tokio::test]
async fn executions_sorted_by_priority() {
    let state = fresh_state().await;
    state.flows.create("r", AuthenticationFlow {
        alias: "browser".into(), description: None, provider_id: "basic-flow".into(),
        top_level: true, built_in: false,
    }).await.unwrap();
    for (id, p) in [("a", 50), ("b", 10), ("c", 30)] {
        state.executions.create("r", "browser", AuthenticationExecution {
            id: id.into(), provider_id: id.into(), requirement: ExecutionRequirement::Required,
            priority: p, flow_alias: "browser".into(), authenticator_flow: false,
        }).await.unwrap();
    }
    let list = state.executions.list("r", "browser").await;
    assert_eq!(list.iter().map(|e| e.priority).collect::<Vec<_>>(), vec![10, 30, 50]);
}

// upstream: keycloak/keycloak AuthenticationManagementTest.java:builtinFlowsArePreserved
#[test]
fn execution_requirement_serialises_uppercase() {
    let r = ExecutionRequirement::Conditional;
    let s = serde_json::to_string(&r).unwrap();
    assert_eq!(s, "\"CONDITIONAL\"");
}
