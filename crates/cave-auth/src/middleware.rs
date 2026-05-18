// SPDX-License-Identifier: AGPL-3.0-or-later
//! Axum middleware for JWT authentication.
//! Validates tokens, extracts CaveIdentity, injects into request extensions.

use crate::claims::RawClaims;
use crate::jwks::JwksCache;
use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, warn};

/// Auth layer that validates JWTs and injects CaveIdentity into request extensions.
#[derive(Clone)]
pub struct CaveAuthLayer {
    jwks_cache: Arc<JwksCache>,
    audience: String,
    issuer: String,
}

impl CaveAuthLayer {
    pub fn new(jwks_cache: Arc<JwksCache>, audience: String, issuer: String) -> Self {
        Self {
            jwks_cache,
            audience,
            issuer,
        }
    }
}

/// Middleware function — use with `axum::middleware::from_fn_with_state`.
pub async fn auth_middleware(
    state: Arc<CaveAuthLayer>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    // Extract Bearer token
    let token = match extract_bearer_token(&req) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Missing or invalid Authorization header" })),
            )
                .into_response();
        }
    };

    // Get JWKS
    let jwks = match state.jwks_cache.get_keys().await {
        Ok(keys) => keys,
        Err(e) => {
            warn!(error = %e, "Failed to fetch JWKS");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Authentication service unavailable" })),
            )
                .into_response();
        }
    };

    // Try each key until one works (handles key rotation)
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&state.audience]);
    validation.set_issuer(&[&state.issuer]);

    for key in &jwks.keys {
        if let Ok(decoding_key) = DecodingKey::from_jwk(key) {
            if let Ok(token_data) = decode::<RawClaims>(&token, &decoding_key, &validation) {
                match token_data.claims.to_identity() {
                    Ok(identity) => {
                        debug!(cave_uid = %identity.cave_uid, tenant = %identity.tenant_id, "Authenticated");
                        req.extensions_mut().insert(identity);
                        return next.run(req).await;
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to extract identity from valid token");
                    }
                }
            }
        }
    }

    // No key worked — try refreshing JWKS (key rotation may have happened)
    if let Ok(fresh_jwks) = state.jwks_cache.refresh().await {
        for key in &fresh_jwks.keys {
            if let Ok(decoding_key) = DecodingKey::from_jwk(key) {
                if let Ok(token_data) = decode::<RawClaims>(&token, &decoding_key, &validation) {
                    if let Ok(identity) = token_data.claims.to_identity() {
                        debug!(cave_uid = %identity.cave_uid, "Authenticated after JWKS refresh");
                        req.extensions_mut().insert(identity);
                        return next.run(req).await;
                    }
                }
            }
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Invalid or expired token" })),
    )
        .into_response()
}

fn extract_bearer_token(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}
