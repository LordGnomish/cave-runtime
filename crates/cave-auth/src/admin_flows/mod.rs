// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/AuthenticationManagementResource.java
//
//! `/admin/realms/{realm}/authentication/` REST surface.
//!
//! Sub-modules:
//! - [`flows`]   — flows CRUD
//! - [`executions`] — executions within a flow
//! - [`required_actions`] — required-action providers (TOTP, configure-otp, terms-and-conditions, …)

pub mod executions;
pub mod flows;
pub mod required_actions;

#[cfg(test)]
pub mod tests;

use axum::Router;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

use flows::AuthenticationFlow;
use executions::AuthenticationExecution;

#[derive(Clone, Default)]
pub struct FlowStore {
    inner: Arc<RwLock<HashMap<String, HashMap<String, AuthenticationFlow>>>>,
}

impl FlowStore {
    pub fn new() -> Self { Self::default() }
    pub async fn list(&self, realm: &str) -> Vec<AuthenticationFlow> {
        self.inner.read().await.get(realm).map(|m| m.values().cloned().collect()).unwrap_or_default()
    }
    pub async fn get(&self, realm: &str, alias: &str) -> Option<AuthenticationFlow> {
        self.inner.read().await.get(realm).and_then(|m| m.get(alias).cloned())
    }
    pub async fn create(&self, realm: &str, flow: AuthenticationFlow) -> Result<AuthenticationFlow, &'static str> {
        let mut w = self.inner.write().await;
        let m = w.entry(realm.to_string()).or_default();
        if m.contains_key(&flow.alias) { return Err("conflict"); }
        m.insert(flow.alias.clone(), flow.clone());
        Ok(flow)
    }
    pub async fn update(&self, realm: &str, alias: &str, flow: AuthenticationFlow) -> Result<AuthenticationFlow, &'static str> {
        let mut w = self.inner.write().await;
        let m = w.get_mut(realm).ok_or("not_found")?;
        if !m.contains_key(alias) { return Err("not_found"); }
        m.insert(alias.to_string(), flow.clone());
        Ok(flow)
    }
    pub async fn delete(&self, realm: &str, alias: &str) -> Result<(), &'static str> {
        let mut w = self.inner.write().await;
        let m = w.get_mut(realm).ok_or("not_found")?;
        m.remove(alias).ok_or("not_found")?;
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct ExecutionStore {
    // realm -> flow_alias -> execution_id -> execution
    inner: Arc<RwLock<HashMap<String, HashMap<String, HashMap<String, AuthenticationExecution>>>>>,
}

impl ExecutionStore {
    pub fn new() -> Self { Self::default() }
    pub async fn list(&self, realm: &str, flow_alias: &str) -> Vec<AuthenticationExecution> {
        let r = self.inner.read().await;
        let mut v: Vec<AuthenticationExecution> = r.get(realm)
            .and_then(|m| m.get(flow_alias))
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        v.sort_by_key(|e| e.priority);
        v
    }
    pub async fn create(&self, realm: &str, flow_alias: &str, exec: AuthenticationExecution) -> Result<AuthenticationExecution, &'static str> {
        let mut w = self.inner.write().await;
        let entry = w.entry(realm.to_string()).or_default().entry(flow_alias.to_string()).or_default();
        if entry.contains_key(&exec.id) { return Err("conflict"); }
        entry.insert(exec.id.clone(), exec.clone());
        Ok(exec)
    }
    pub async fn update(&self, realm: &str, flow_alias: &str, id: &str, exec: AuthenticationExecution) -> Result<AuthenticationExecution, &'static str> {
        let mut w = self.inner.write().await;
        let entry = w.get_mut(realm).and_then(|m| m.get_mut(flow_alias)).ok_or("not_found")?;
        if !entry.contains_key(id) { return Err("not_found"); }
        entry.insert(id.to_string(), exec.clone());
        Ok(exec)
    }
    pub async fn delete(&self, realm: &str, flow_alias: &str, id: &str) -> Result<(), &'static str> {
        let mut w = self.inner.write().await;
        let entry = w.get_mut(realm).and_then(|m| m.get_mut(flow_alias)).ok_or("not_found")?;
        entry.remove(id).ok_or("not_found")?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct AdminFlowsState {
    pub realms: crate::keycloak::realm::RealmStore,
    pub flows: FlowStore,
    pub executions: ExecutionStore,
}

impl AdminFlowsState {
    pub fn new(realms: crate::keycloak::realm::RealmStore) -> Self {
        Self { realms, flows: FlowStore::new(), executions: ExecutionStore::new() }
    }
}

pub fn admin_flows_router(state: AdminFlowsState) -> Router {
    Router::new()
        .merge(flows::router(state.clone()))
        .merge(executions::router(state.clone()))
        .merge(required_actions::router(state))
}
