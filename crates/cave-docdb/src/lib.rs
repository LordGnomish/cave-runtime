// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE DocDB — MongoDB 6.0-compatible document database.
//!
//! Compatible with: MongoDB Community Edition (SSPL-1.1).
//! Sovereign-safe alternative pattern — wire-protocol compatibility + fully
//! in-memory Rust engine. Backs drivers (pymongo, mongo-rust-driver, Node
//! mongoose, Java driver) without modification on port 27017.

/// Aggregation deep stages — $lookup / $facet / $bucket.
pub mod aggregation_deep;

/// BSON serialization/deserialization utilities.
pub mod bson;

/// Change streams — oplog tail cursor.
pub mod change_streams;

/// Codec implementations for BSON types.
pub mod codec;

/// GridFS — chunked file storage.
pub mod gridfs;

/// MongoDB wire protocol command handlers.
pub mod commands;

/// Cursor implementation for query results.
pub mod cursor;

/// Core database engine logic.
pub mod engine;

/// Index management and structures.
pub mod index;

/// Data models and structures.
pub mod models;

/// Projection logic for query results.
pub mod projection;

/// Query parsing and execution.
pub mod query;

/// HTTP route handlers.
pub mod routes;

/// Server configuration and state.
pub mod server;

/// Update operations and logic.
pub mod update;

/// Wire protocol implementation.
pub mod wire;

/// Re-export the Engine struct for easy access.
pub use engine::Engine;

/// Re-export the DocDbState struct for easy access.
pub use routes::DocDbState;

/// Type alias for DocDbState.
pub type State = DocDbState;

use axum::Router;
use std::sync::Arc;

/// Creates an Axum Router with the given state.
pub fn router(state: Arc<DocDbState>) -> Router {
    routes::create_router(state)
}

/// Creates a new default DocDbState wrapped in Arc.
pub fn new_state() -> Arc<DocDbState> {
    Arc::new(DocDbState::default())
}

/// The module name constant.
pub const MODULE_NAME: &str = "docdb";
