//! CAVE DocDB — MongoDB 6.0-compatible document database.
//!
//! Compatible with: MongoDB Community Edition (SSPL-1.1).
//! Sovereign-safe alternative pattern — wire-protocol compatibility + fully
//! in-memory Rust engine. Backs drivers (pymongo, mongo-rust-driver, Node
//! mongoose, Java driver) without modification on port 27017.

pub mod bson;
pub mod commands;
pub mod cursor;
pub mod engine;
pub mod index;
pub mod models;
pub mod projection;
pub mod query;
pub mod routes;
pub mod server;
pub mod update;
pub mod wire;

pub use engine::Engine;
pub use routes::DocDbState;
pub type State = DocDbState;

use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<DocDbState>) -> Router {
    routes::create_router(state)
}

pub fn new_state() -> Arc<DocDbState> {
    Arc::new(DocDbState::default())
}

pub const MODULE_NAME: &str = "docdb";
