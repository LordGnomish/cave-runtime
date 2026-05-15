// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for authentication endpoints.
//!
//! Provides:
//! - `/api/auth/me` — get current user's claims
//! - `/api/auth/token` — dev-only endpoint to create test JWTs

use crate::jwt_middleware::JwtClaims;
use axum::{
    extract::Json,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// Request body for dev token creation.
#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    pub sub: String,
    pub email: String,
    pub roles: Vec<String>,
}

/// Response for token creation.
#[derive(Debug, Serialize)]
pub struct CreateTokenResponse {
    pub token: String,
    pub expires_in: usize,
}

/// Handler: GET /api/auth/me — return current user's claims
pub async fn get_me(claims: JwtClaims) -> impl IntoResponse {
    Json(json!({
        "sub": claims.sub,
        "email": claims.email,
        "roles": claims.roles,
        "exp": claims.exp,
    }))
}

/// Handler: POST /api/auth/token — create a dev JWT (DEV-ONLY)
pub async fn create_token(
    Json(req): Json<CreateTokenRequest>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    // Check if dev mode is enabled
    let dev_mode = std::env::var("CAVE_DEV_MODE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if !dev_mode {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Token creation disabled — set CAVE_DEV_MODE=true" })),
        ));
    }

    // Get JWT secret from env (required)
    let secret = std::env::var("CAVE_JWT_SECRET")
        .expect("CAVE_JWT_SECRET must be set (use any string for dev)");

    // Create claims
    let exp = (Utc::now().timestamp() + 3600) as usize; // 1 hour expiry
    let claims = JwtClaims {
        sub: req.sub,
        email: req.email,
        roles: req.roles,
        exp,
    };

    // Encode JWT
    let key = EncodingKey::from_secret(secret.as_bytes());

    let token = match encode(&Header::new(Algorithm::HS256), &claims, &key) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "Failed to encode JWT");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to create token" })),
            ));
        }
    };

    Ok((
        StatusCode::CREATED,
        Json(CreateTokenResponse {
            token,
            expires_in: 3600,
        }),
    ))
}

/// Mount auth routes — `/api/auth/me` and `/api/auth/token` (dev)
pub fn router() -> Router {
    Router::new()
        .route("/api/auth/me", get(get_me))
        .route("/api/auth/token", post(create_token))
}
