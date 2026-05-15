// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::token::CreateTokenParams;
use crate::VaultState;
use axum::{
    extract::{Extension, Json, State},
    http::HeaderMap,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

pub async fn create_token(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(params): Json<CreateTokenParams>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let caller = ts.lookup(&token_str).ok_or(VaultError::BadToken)?.clone();
    drop(ts);

    let mut ts = state.token_store.write().await;
    let new_token = ts.create(&params, Some(&caller))?;
    Ok(VaultResponse::new().with_auth(new_token.to_auth_info()))
}

pub async fn create_orphan_token(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(mut params): Json<CreateTokenParams>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    {
        let ts = state.token_store.read().await;
        let _ = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    }
    params.no_parent = Some(true);
    let mut ts = state.token_store.write().await;
    let new_token = ts.create(&params, None)?;
    Ok(VaultResponse::new().with_auth(new_token.to_auth_info()))
}

pub async fn lookup_self(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&token_str).ok_or(VaultError::BadToken)?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(token).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct LookupRequest {
    pub token: Option<String>,
}

pub async fn lookup_token(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<LookupRequest>,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let target = body.token.ok_or_else(|| VaultError::InvalidRequest("token required".into()))?;
    let ts = state.token_store.read().await;
    let token = ts.lookup(&target).ok_or(VaultError::TokenNotFound)?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(token).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct LookupAccessorRequest {
    pub accessor: Option<String>,
}

pub async fn lookup_accessor(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<LookupAccessorRequest>,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let accessor = body.accessor.ok_or_else(|| VaultError::InvalidRequest("accessor required".into()))?;
    let ts = state.token_store.read().await;
    let token = ts.lookup_by_accessor(&accessor).ok_or(VaultError::TokenNotFound)?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(token).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct RenewRequest {
    pub token: Option<String>,
    pub increment: Option<String>,
}

pub async fn renew_token(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<RenewRequest>,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let target = body.token.ok_or_else(|| VaultError::InvalidRequest("token required".into()))?;
    let increment = body.increment.as_deref()
        .map(crate::token::parse_duration)
        .unwrap_or(3600);
    let mut ts = state.token_store.write().await;
    let token = ts.renew(&target, increment)?;
    Ok(VaultResponse::new().with_auth(token.to_auth_info()))
}

pub async fn renew_self(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<RenewRequest>,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let increment = body.increment.as_deref()
        .map(crate::token::parse_duration)
        .unwrap_or(3600);
    let mut ts = state.token_store.write().await;
    let token = ts.renew(&token_str, increment)?;
    Ok(VaultResponse::new().with_auth(token.to_auth_info()))
}

#[derive(Deserialize)]
pub struct RevokeRequest {
    pub token: Option<String>,
}

pub async fn revoke_token(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<RevokeRequest>,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let target = body.token.ok_or_else(|| VaultError::InvalidRequest("token required".into()))?;
    let mut ts = state.token_store.write().await;
    ts.revoke_tree(&target);
    Ok(VaultResponse::new())
}

pub async fn revoke_self(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let token_str = extract_token(&headers)?;
    let mut ts = state.token_store.write().await;
    ts.revoke(&token_str);
    Ok(VaultResponse::new())
}

#[derive(Deserialize)]
pub struct RevokeAccessorRequest {
    pub accessor: Option<String>,
}

pub async fn revoke_accessor(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<RevokeAccessorRequest>,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let accessor = body.accessor.ok_or_else(|| VaultError::InvalidRequest("accessor required".into()))?;
    let mut ts = state.token_store.write().await;
    let token_id = {
        ts.lookup_by_accessor(&accessor)
            .map(|t| t.id.clone())
            .ok_or(VaultError::TokenNotFound)?
    };
    ts.revoke_tree(&token_id);
    Ok(VaultResponse::new())
}

pub async fn revoke_orphan(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<RevokeRequest>,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let target = body.token.ok_or_else(|| VaultError::InvalidRequest("token required".into()))?;
    let mut ts = state.token_store.write().await;
    ts.revoke(&target);
    Ok(VaultResponse::new())
}

pub async fn list_accessors(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _caller_token = extract_token(&headers)?;
    let ts = state.token_store.read().await;
    let accessors = ts.list_accessors();
    Ok(VaultResponse::new().with_data(json!({ "keys": accessors })))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .route("/v1/auth/token/create", post(create_token))
        .route("/v1/auth/token/create-orphan", post(create_orphan_token))
        .route("/v1/auth/token/lookup-self", get(lookup_self))
        .route("/v1/auth/token/lookup", post(lookup_token))
        .route("/v1/auth/token/lookup-accessor", post(lookup_accessor))
        .route("/v1/auth/token/renew", post(renew_token))
        .route("/v1/auth/token/renew-self", post(renew_self))
        .route("/v1/auth/token/revoke", post(revoke_token))
        .route("/v1/auth/token/revoke-self", post(revoke_self))
        .route("/v1/auth/token/revoke-accessor", post(revoke_accessor))
        .route("/v1/auth/token/revoke-orphan", post(revoke_orphan))
        .route("/v1/auth/token/accessors", get(list_accessors))
        .with_state(state)
}
