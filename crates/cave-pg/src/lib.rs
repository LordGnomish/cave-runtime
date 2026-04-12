//! cave-pg — Embedded PostgreSQL-compatible database engine.
//!
//! Provides a full PostgreSQL wire protocol v3 server that can be used as a
//! drop-in replacement for PostgreSQL by libpq, psycopg2, node-postgres, etc.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use cave_pg::{Engine, Server};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() {
//!     let engine = Arc::new(Engine::new());
//!     let server = Server::new(engine, "127.0.0.1:5432").await.unwrap();
//!     server.run().await.unwrap();
//! }
//! ```

pub mod error;
pub mod types;
pub mod protocol;
pub mod auth;
pub mod storage;
pub mod catalog;
pub mod functions;
pub mod executor;
pub mod session;
pub mod server;
pub mod pool;

// ─────────────────────────────────────────────────────────────────────────────
// Top-level re-exports
// ─────────────────────────────────────────────────────────────────────────────

pub use error::{Error, PgError, Result};
pub use types::{ColumnDesc, FormatCode, Interval, Oid, PgValue, ResultSet, CommandResult};
pub use storage::{Engine, Database};
pub use executor::Executor;
pub use server::Server;
pub use pool::ConnectionPool;
