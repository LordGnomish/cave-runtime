//! Cave Portal HTTP API — attribution / activity / DORA dashboards.
//!
//! Mounting in cave-runtime (or any axum app):
//! ```ignore
//! use cave_portal_api::{admin_router, PortalDataState};
//! let state = PortalDataState::from_env();
//! let app = axum::Router::new().merge(admin_router(state));
//! ```
//!
//! Routes mounted:
//!   - `/api/portal/attribution/{summary,timeline,by-module}`
//!   - `/api/portal/activity/heatmap`
//!   - `/api/portal/dora/{summary,timeline}`
//!   - `/admin/{attribution,activity,dora}`

pub mod routes;

pub use routes::state::PortalDataState;

use axum::Router;

/// Combined admin router: all three dashboards + their JSON endpoints, sharing
/// a single 5-minute commit cache.
pub fn admin_router(state: PortalDataState) -> Router {
    Router::new()
        .merge(routes::attribution::router(state.clone()))
        .merge(routes::activity::router(state.clone()))
        .merge(routes::dora::router(state))
}
