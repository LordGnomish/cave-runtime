// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::VaultState;
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use axum::{
    Router,
    extract::{Json, Path, Query, State},
    http::HeaderMap,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
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

/// Per-token private storage, namespaced by token accessor
#[derive(Default)]
pub struct CubbyholeStore {
    /// accessor -> path -> data
    pub data: HashMap<String, HashMap<String, HashMap<String, Value>>>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub list: Option<String>,
}

pub async fn read_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    let accessor = token.accessor.clone();
    drop(ts);

    if q.list.as_deref() == Some("true") {
        let store = state.cubbyhole_store.read().await;
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{}/", path)
        };
        let keys: Vec<String> = store
            .data
            .get(&accessor)
            .map(|m| {
                let mut seen = std::collections::BTreeSet::new();
                for k in m.keys() {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        let part = rest.split('/').next().unwrap_or(rest);
                        if rest.contains('/') {
                            seen.insert(format!("{}/", part));
                        } else {
                            seen.insert(part.to_string());
                        }
                    } else if prefix.is_empty() {
                        let part = k.split('/').next().unwrap_or(k.as_str());
                        if k.contains('/') {
                            seen.insert(format!("{}/", part));
                        } else {
                            seen.insert(part.to_string());
                        }
                    }
                }
                seen.into_iter().collect()
            })
            .unwrap_or_default();
        return Ok(VaultResponse::new().with_data(json!({ "keys": keys })));
    }

    let store = state.cubbyhole_store.read().await;
    let secret = store
        .data
        .get(&accessor)
        .and_then(|m| m.get(&path))
        .ok_or(VaultError::SecretNotFound)?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(secret).unwrap_or_default()))
}

pub async fn write_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Json(body): Json<HashMap<String, Value>>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    let accessor = token.accessor.clone();
    drop(ts);

    let mut store = state.cubbyhole_store.write().await;
    store.data.entry(accessor).or_default().insert(path, body);
    Ok(VaultResponse::new())
}

pub async fn delete_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    let accessor = token.accessor.clone();
    drop(ts);

    let mut store = state.cubbyhole_store.write().await;
    if let Some(m) = store.data.get_mut(&accessor) {
        m.remove(&path);
    }
    Ok(VaultResponse::new())
}

pub async fn list_secrets(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    let accessor = token.accessor.clone();
    drop(ts);

    let store = state.cubbyhole_store.read().await;
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{}/", path)
    };
    let keys: Vec<String> = store
        .data
        .get(&accessor)
        .map(|m| {
            let mut seen = std::collections::BTreeSet::new();
            for k in m.keys() {
                if let Some(rest) = k.strip_prefix(&prefix) {
                    let part = rest.split('/').next().unwrap_or(rest);
                    if rest.contains('/') {
                        seen.insert(format!("{}/", part));
                    } else {
                        seen.insert(part.to_string());
                    }
                } else if prefix.is_empty() {
                    let part = k.split('/').next().unwrap_or(k.as_str());
                    if k.contains('/') {
                        seen.insert(format!("{}/", part));
                    } else {
                        seen.insert(part.to_string());
                    }
                }
            }
            seen.into_iter().collect()
        })
        .unwrap_or_default();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route(
            "/v1/cubbyhole/{*path}",
            get({
                let s = state.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Query(q): Query<ListQuery>| {
                    let state = s.clone();
                    async move { read_secret(State(state), headers, Path(path), Query(q)).await }
                }
            })
            .put({
                let s = state.clone();
                move |headers: HeaderMap,
                      Path(path): Path<String>,
                      Json(body): Json<HashMap<String, Value>>| {
                    let state = s.clone();
                    async move { write_secret(State(state), headers, Path(path), Json(body)).await }
                }
            })
            .post({
                let s = state.clone();
                move |headers: HeaderMap,
                      Path(path): Path<String>,
                      Json(body): Json<HashMap<String, Value>>| {
                    let state = s.clone();
                    async move { write_secret(State(state), headers, Path(path), Json(body)).await }
                }
            })
            .delete({
                let s = state.clone();
                move |headers: HeaderMap, Path(path): Path<String>| {
                    let state = s.clone();
                    async move { delete_secret(State(state), headers, Path(path)).await }
                }
            }),
        )
        .with_state(state)
}
