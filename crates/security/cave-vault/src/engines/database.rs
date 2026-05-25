// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::VaultState;
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use axum::{
    Router,
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
};
use chrono::{Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers
        .get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConnection {
    pub name: String,
    pub plugin_name: String, // e.g., "postgresql-database-plugin", "mysql-database-plugin"
    pub connection_url: String,
    pub max_open_connections: i32,
    pub max_idle_connections: i32,
    pub max_connection_lifetime: String,
    pub allowed_roles: Vec<String>,
    pub password_policy: String,
    pub verify_connection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseRole {
    pub name: String,
    pub db_name: String,
    pub creation_statements: Vec<String>,
    pub revocation_statements: Vec<String>,
    pub rollback_statements: Vec<String>,
    pub renew_statements: Vec<String>,
    pub default_ttl: i64,
    pub max_ttl: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticRole {
    pub name: String,
    pub db_name: String,
    pub username: String,
    pub rotation_period: i64,
    pub rotation_statements: Vec<String>,
    pub last_vault_rotation: Option<String>,
    pub next_rotation_time: Option<String>,
}

#[derive(Default)]
pub struct DatabaseStore {
    pub connections: HashMap<String, DatabaseConnection>,
    pub roles: HashMap<String, DatabaseRole>,
    pub static_roles: HashMap<String, StaticRole>,
}

fn random_string(len: usize) -> VaultResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = vec![0u8; len];
    rng.fill(&mut bytes)
        .map_err(|_| VaultError::Crypto("rng failure".into()))?;
    Ok(hex::encode(&bytes[..len / 2]))
}

pub async fn configure_connection(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let conn = DatabaseConnection {
        name: name.clone(),
        plugin_name: body
            .get("plugin_name")
            .and_then(|v| v.as_str())
            .unwrap_or("postgresql-database-plugin")
            .to_string(),
        connection_url: body
            .get("connection_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        max_open_connections: body
            .get("max_open_connections")
            .and_then(|v| v.as_i64())
            .unwrap_or(4) as i32,
        max_idle_connections: body
            .get("max_idle_connections")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        max_connection_lifetime: body
            .get("max_connection_lifetime")
            .and_then(|v| v.as_str())
            .unwrap_or("0s")
            .to_string(),
        allowed_roles: body
            .get("allowed_roles")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        password_policy: body
            .get("password_policy")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        verify_connection: body
            .get("verify_connection")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
    };
    let mut store = state.database_store.write().await;
    store.connections.insert(name, conn);
    Ok(VaultResponse::new())
}

pub async fn read_connection(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.database_store.read().await;
    let conn = store
        .connections
        .get(&name)
        .ok_or_else(|| VaultError::NotFound(format!("connection {} not found", name)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(conn).unwrap_or_default()))
}

pub async fn delete_connection(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.database_store.write().await;
    store.connections.remove(&name);
    Ok(VaultResponse::new())
}

pub async fn list_connections(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.database_store.read().await;
    let keys: Vec<String> = store.connections.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn create_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let role = DatabaseRole {
        name: role_name.clone(),
        db_name: body
            .get("db_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        creation_statements: body
            .get("creation_statements")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        revocation_statements: body
            .get("revocation_statements")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        rollback_statements: Vec::new(),
        renew_statements: Vec::new(),
        default_ttl: body
            .get("default_ttl")
            .and_then(|v| v.as_str())
            .map(crate::token::parse_duration)
            .unwrap_or(3600),
        max_ttl: body
            .get("max_ttl")
            .and_then(|v| v.as_str())
            .map(crate::token::parse_duration)
            .unwrap_or(86400),
    };
    let mut store = state.database_store.write().await;
    store.roles.insert(role_name, role);
    Ok(VaultResponse::new())
}

pub async fn read_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.database_store.read().await;
    let role = store
        .roles
        .get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(role).unwrap_or_default()))
}

pub async fn delete_role(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.database_store.write().await;
    store.roles.remove(&role_name);
    Ok(VaultResponse::new())
}

pub async fn list_roles(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(mount): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.database_store.read().await;
    let keys: Vec<String> = store.roles.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub async fn generate_credentials(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, role_name)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.database_store.read().await;
    let role = store
        .roles
        .get(&role_name)
        .ok_or_else(|| VaultError::RoleNotFound(role_name.clone()))?
        .clone();
    drop(store);

    let username = format!("v-{}--{}", role_name, random_string(8)?);
    let password = random_string(24)?;

    let lease = crate::core::Lease::new(
        &format!("database/creds/{}", role_name),
        "",
        "database",
        role.default_ttl,
        role.max_ttl,
        true,
    );
    let lease_id = lease.id.clone();
    let mut ls = state.lease_store.write().await;
    ls.put(lease);

    Ok(VaultResponse::new()
        .with_data(json!({ "username": username, "password": password }))
        .with_lease(lease_id, role.default_ttl, true))
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    Router::new()
        .route(&format!("/v1/{}/config", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { list_connections(State(state), headers, Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/config/{{name}}", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(name): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { configure_connection(State(state), headers, Path((mount, name)), Json(body)).await }
            }
        }).get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { read_connection(State(state), headers, Path((mount, name))).await }
            }
        }).delete({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { delete_connection(State(state), headers, Path((mount, name))).await }
            }
        }))
        .route(&format!("/v1/{}/roles", mount), get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap| {
                let state = s.clone();
                let mount = mount.clone();
                async move { list_roles(State(state), headers, Path(mount)).await }
            }
        }))
        .route(&format!("/v1/{}/roles/{{role_name}}", mount), post({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>, Json(body): Json<Value>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { create_role(State(state), headers, Path((mount, role_name)), Json(body)).await }
            }
        }).get({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { read_role(State(state), headers, Path((mount, role_name))).await }
            }
        }).delete({
            let s = state.clone();
            let mount = m.clone();
            move |headers: HeaderMap, Path(role_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { delete_role(State(state), headers, Path((mount, role_name))).await }
            }
        }))
        .route(&format!("/v1/{}/creds/{{role_name}}", mount), get({
            let s = state.clone();
            let mount = m;
            move |headers: HeaderMap, Path(role_name): Path<String>| {
                let state = s.clone();
                let mount = mount.clone();
                async move { generate_credentials(State(state), headers, Path((mount, role_name))).await }
            }
        }))
        .with_state(state)
}
