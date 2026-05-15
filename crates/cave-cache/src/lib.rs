//! cave-cache — Full Redis parity cache server.
//!
//! Implements the RESP2/RESP3 wire protocol, all Redis data types, pub/sub,
//! Lua scripting, transactions, key expiry, persistence, and more.

/// The Access Control List module.
///
/// Handles user authentication, permissions, and command restrictions.
pub mod acl;

/// The cluster module.
///
/// Manages cluster topology, node communication, and data sharding.
pub mod cluster;

/// The codec module.
///
/// Handles serialization and deserialization of data types.
pub mod codec;

/// The commands module.
///
/// Implements all Redis commands and their logic.
pub mod commands;

/// The configuration module.
///
/// Parses and manages server configuration options.
pub mod config;

/// The database module.
///
/// Manages server state, key storage, and expiration logic.
pub mod db;

/// The error module.
///
/// Defines error types and handling for the server.
pub mod error;

/// The eviction module.
///
/// Implements eviction policies like LRU and LFU.
pub mod eviction;

/// The keyspace module.
///
/// Manages keyspace notifications and indexing.
pub mod keyspace;

/// The persistence module.
///
/// Handles RDB and AOF persistence mechanisms.
pub mod persistence;

/// The RESP protocol module.
///
/// Implements RESP2 and RESP3 wire protocol parsing and serialization.
pub mod resp;

/// The server module.
///
/// Handles network connections, event loops, and request dispatching.
pub mod server;

/// The types module.
///
/// Defines core data structures and value types.
pub mod types;

/// Sentinel HA — monitor/failover state machine.
pub mod sentinel;

/// Master/replica replication — repl id, offset, backlog, PSYNC.
pub mod replication;

/// Native TLS listener config + PEM loader.
pub mod tls_listener;

/// Re-exports the main server run function.
pub use server::run;

/// Re-exports the configuration struct.
pub use config::Config;

/// Re-exports the server state struct.
pub use db::ServerState;
