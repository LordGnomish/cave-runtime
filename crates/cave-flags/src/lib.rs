//! CAVE Flags — Feature flag evaluation engine.
//!
//! Replaces Unleash with a Rust-native implementation.
//! Supports: boolean flags, gradual rollout, kill switches, A/B variants,
//! environment-scoped flags, and SSE streaming for real-time updates.
//!
//! ## Upstream Tracking: Unleash
//! - GitHub: https://github.com/Unleash/unleash
//! - Tracked: strategy types, client SDK protocol, metrics API
//! - Parity target: Unleash v6.x feature set

pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state shared across request handlers.
pub struct FlagsState {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for the flags module.
pub fn router(state: Arc<FlagsState>) -> Router {
    routes::create_router(state)
}

/// Module name for DB schema.
pub const MODULE_NAME: &str = "flags";
