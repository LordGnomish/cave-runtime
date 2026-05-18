// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::VaultState;
use axum::{
    extract::{Json, Path, Query, State},
    http::HeaderMap,
    routing::{delete, get, post, put},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Default)]
pub struct Kv1Store {
    /// mount -> path -> secret data
    pub data: HashMap<String, HashMap<String, HashMap<String, Value>>>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub list: Option<String>,
}

pub async fn read_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;

    if q.list.as_deref() == Some("true") {
        let store = state.kv1_store.read().await;
        let mount_data = store.data.get(&mount);
        let prefix = if path.is_empty() { String::new() } else { format!("{}/", path) };
        let keys: Vec<String> = mount_data
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
                    }
                }
                seen.into_iter().collect()
            })
            .unwrap_or_default();
        return Ok(VaultResponse::new().with_data(json!({ "keys": keys })));
    }

    let store = state.kv1_store.read().await;
    let secret = store.data.get(&mount)
        .and_then(|m| m.get(&path))
        .ok_or(VaultError::SecretNotFound)?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(secret).unwrap_or_default()))
}

pub async fn write_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
    Json(body): Json<HashMap<String, Value>>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv1_store.write().await;
    store.data.entry(mount).or_default().insert(path, body);
    Ok(VaultResponse::new())
}

pub async fn delete_secret(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.kv1_store.write().await;
    if let Some(m) = store.data.get_mut(&mount) {
        m.remove(&path);
    }
    Ok(VaultResponse::new())
}

pub async fn list_secrets(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path((mount, path)): Path<(String, String)>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.kv1_store.read().await;
    let mount_data = store.data.get(&mount);
    let prefix = if path.is_empty() { String::new() } else { format!("{}/", path) };
    let keys: Vec<String> = mount_data
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
                }
            }
            seen.into_iter().collect()
        })
        .unwrap_or_default();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub fn router(state: Arc<VaultState>, mount: &str) -> Router {
    let m = mount.to_string();
    let m2 = mount.to_string();
    let m3 = mount.to_string();
    let m4 = mount.to_string();

    Router::new()
        .route(
            &format!("/v1/{}/{{*path}}", mount),
            get({
                let s = state.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Query(q): Query<ListQuery>| {
                    let state = s.clone();
                    let mount = m.clone();
                    async move {
                        read_secret(
                            State(state),
                            headers,
                            Path((mount, path)),
                            Query(q),
                        ).await
                    }
                }
            })
            .put({
                let s = state.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<HashMap<String, Value>>| {
                    let state = s.clone();
                    let mount = m2.clone();
                    async move {
                        write_secret(State(state), headers, Path((mount, path)), Json(body)).await
                    }
                }
            })
            .post({
                let s = state.clone();
                move |headers: HeaderMap, Path(path): Path<String>, Json(body): Json<HashMap<String, Value>>| {
                    let state = s.clone();
                    let mount = m3.clone();
                    async move {
                        write_secret(State(state), headers, Path((mount, path)), Json(body)).await
                    }
                }
            })
            .delete({
                let s = state.clone();
                move |headers: HeaderMap, Path(path): Path<String>| {
                    let state = s.clone();
                    let mount = m4.clone();
                    async move {
                        delete_secret(State(state), headers, Path((mount, path))).await
                    }
                }
            }),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_store() -> Arc<RwLock<Kv1Store>> {
        Arc::new(RwLock::new(Kv1Store::default()))
    }

    #[tokio::test]
    async fn test_kv1_crud() {
        let store = make_store();
        {
            let mut s = store.write().await;
            let mut data = HashMap::new();
            data.insert("password".to_string(), Value::String("secret123".to_string()));
            s.data.entry("secret".to_string()).or_default().insert("myapp/db".to_string(), data);
        }
        {
            let s = store.read().await;
            let secret = s.data.get("secret").and_then(|m| m.get("myapp/db")).unwrap();
            assert_eq!(secret.get("password").and_then(|v| v.as_str()), Some("secret123"));
        }
        {
            let mut s = store.write().await;
            if let Some(m) = s.data.get_mut("secret") {
                m.remove("myapp/db");
            }
        }
        {
            let s = store.read().await;
            assert!(s.data.get("secret").and_then(|m| m.get("myapp/db")).is_none());
        }
    }
}
