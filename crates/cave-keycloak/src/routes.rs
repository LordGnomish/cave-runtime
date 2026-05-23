// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Axum HTTP surface — wires the controllers to URLs the cave-cli and
//! cave-portal-ui hit. The routes here only expose the safe read-only
//! shape (discovery + JWKS + health). The full admin write surface lives
//! behind `cave-auth`'s authenticated middleware in production wiring
//! and is added by the embedding crate (`cave-runtime`).

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use std::sync::Arc;

use crate::discovery::{discovery_for, DiscoveryDocument};
use crate::jwks::{jwks_for, JwksDocument};

pub fn create_router(state: Arc<crate::State>) -> Router {
    Router::new()
        .route("/api/iam/health", axum::routing::get(health))
        .route(
            "/api/iam/realms/{realm}/.well-known/openid-configuration",
            axum::routing::get(discovery_handler),
        )
        .route(
            "/api/iam/realms/{realm}/protocol/openid-connect/certs",
            axum::routing::get(jwks_handler),
        )
        .with_state(state)
}

#[derive(serde::Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub module: &'static str,
    pub realm_count: usize,
    pub user_count: usize,
    pub source_sha: &'static str,
    pub upstream_version: &'static str,
}

pub const UPSTREAM_SOURCE_SHA: &str = "0a402f777f8985eccbb07556e96d9b386275e048";
pub const UPSTREAM_VERSION: &str = "26.6.2";

async fn health(State(state): State<Arc<crate::State>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        module: crate::MODULE_NAME,
        realm_count: state.store.realm_count(),
        user_count: state.store.user_count(),
        source_sha: UPSTREAM_SOURCE_SHA,
        upstream_version: UPSTREAM_VERSION,
    })
}

async fn discovery_handler(
    Path(realm): Path<String>,
    State(_state): State<Arc<crate::State>>,
) -> Result<Json<DiscoveryDocument>, StatusCode> {
    Ok(Json(discovery_for(&realm, "https://iam.cave.svc")))
}

async fn jwks_handler(
    Path(realm): Path<String>,
    State(state): State<Arc<crate::State>>,
) -> Json<JwksDocument> {
    Json(jwks_for(&realm, &state.signer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Realm;

    #[tokio::test]
    async fn health_handler_returns_zero_counts_when_empty() {
        let s = Arc::new(crate::State::default());
        let resp = health(State(s.clone())).await;
        assert_eq!(resp.0.status, "ok");
        assert_eq!(resp.0.module, "keycloak");
        assert_eq!(resp.0.realm_count, 0);
        assert_eq!(resp.0.upstream_version, "26.6.2");
    }

    #[tokio::test]
    async fn health_handler_counts_realms() {
        let s = Arc::new(crate::State::default());
        s.store.put_realm(Realm::new("r1", "t1", "R1")).unwrap();
        let resp = health(State(s.clone())).await;
        assert_eq!(resp.0.realm_count, 1);
    }

    #[tokio::test]
    async fn discovery_handler_returns_realm_scoped_doc() {
        let s = Arc::new(crate::State::default());
        let resp = discovery_handler(Path("master".into()), State(s)).await.unwrap();
        assert!(resp.0.issuer.ends_with("/realms/master"));
    }

    #[tokio::test]
    async fn jwks_handler_returns_keys_for_realm() {
        let s = Arc::new(crate::State::default());
        s.signer.install(
            "r1",
            crate::signer::SigningKeyEntry::es256_from_seed("k1", &[7u8; 32]).unwrap(),
            true,
        );
        let resp = jwks_handler(Path("r1".into()), State(s)).await;
        assert_eq!(resp.0.keys.len(), 1);
    }
}
