//! cave-cache — Full Redis parity cache server.
//!
//! Implements the RESP2/RESP3 wire protocol, all Redis data types, pub/sub,
//! Lua scripting, transactions, key expiry, persistence, and more.

pub mod acl;
pub mod cluster;
pub mod codec;
pub mod commands;
pub mod config;
pub mod db;
pub mod error;
pub mod eviction;
pub mod keyspace;
pub mod persistence;
pub mod resp;
pub mod server;
pub mod types;

pub use server::run;
pub use config::Config;
pub use db::ServerState;
