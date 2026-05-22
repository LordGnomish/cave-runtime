// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/IdentityProvidersResource.java
//
// Mounted at `/admin/realms/{realm}/identity-provider`.
//
//   /instances                          -> instances::*
//   /instances/{alias}/mappers          -> mappers::*

pub mod instances;
pub mod mappers;

use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;

pub use instances::{IdentityProvider, IdentityProviderStore};
pub use mappers::{IdentityProviderMapper, IdentityProviderMapperStore};

/// Composite state — both stores. Routers fan out via `with_state`.
#[derive(Clone)]
pub struct AdminIdpState {
    pub providers: Arc<IdentityProviderStore>,
    pub mappers: Arc<IdentityProviderMapperStore>,
}

impl AdminIdpState {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(IdentityProviderStore::new()),
            mappers: Arc::new(IdentityProviderMapperStore::new()),
        }
    }
}

impl Default for AdminIdpState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a fully wired admin-IdP router. Mount with
/// `app.merge(cave_auth::admin_idp::router(state))`.
pub fn router(state: AdminIdpState) -> Router {
    let providers = Router::new()
        .route(
            "/admin/realms/{realm}/identity-provider/instances",
            get(instances::list_instances).post(instances::create_instance),
        )
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}",
            get(instances::get_instance)
                .put(instances::update_instance)
                .delete(instances::delete_instance),
        )
        .with_state(state.providers);

    let mappers = Router::new()
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}/mappers",
            get(mappers::list_mappers).post(mappers::create_mapper),
        )
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}",
            get(mappers::get_mapper)
                .put(mappers::update_mapper)
                .delete(mappers::delete_mapper),
        )
        .with_state(state.mappers);

    Router::new().merge(providers).merge(mappers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_default_constructs() {
        let s = AdminIdpState::default();
        assert!(s.providers.list("master").is_empty());
        assert!(s.mappers.list("master", "google").is_empty());
    }

    #[test]
    fn router_builds_without_panic() {
        let _ = router(AdminIdpState::new());
    }
}
