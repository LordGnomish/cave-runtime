//! CAVE RDBMS — PostgreSQL-wire-protocol-compatible SQL database.
//!
//! Replaces: PostgreSQL (management + query surface). Standard Postgres
//! clients (psql, libpq, pgjdbc, psycopg, pgx, sqlx, node-pg) connect on
//! port 5432 unmodified.
//!
//! Sovereign-safe reimplementation: Apache-2.0, pure-Rust engine, no FFI
//! to Postgres. Not feature-complete vs. upstream PostgreSQL; targets a
//! broad, practical SQL subset plus compatibility shim for catalog
//! queries that psql/Postgres drivers require on connect.

pub mod engine;
pub mod executor;
pub mod models;
pub mod protocol;
pub mod routes;
pub mod server;
pub mod sql;
pub mod storage;
pub mod types;

pub use engine::Engine;
pub use routes::RdbmsState;
pub type State = RdbmsState;

use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<RdbmsState>) -> Router {
    routes::create_router(state)
}

pub fn new_state() -> Arc<RdbmsState> {
    Arc::new(RdbmsState::default())
}

pub const MODULE_NAME: &str = "rdbms";
