// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keycloak Admin REST endpoints — IdP CRUD + Authentication-flow CRUD.
//!
//! - [`idp`] — `/admin/realms/{realm}/identity-provider/...`
//! - [`authflow`] — `/admin/realms/{realm}/authentication/...`

pub mod authflow;
pub mod idp;

use axum::Router;

pub fn router() -> Router {
    Router::new()
        .merge(idp::router(idp::IdpService::new()))
        .merge(authflow::router(authflow::FlowService::new()))
}
