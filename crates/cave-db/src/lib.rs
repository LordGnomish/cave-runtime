// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE DB — shared PostgreSQL connection pool, migration runner, and
//! pluggable persistence layer.
//!
//! Each module gets its own schema (e.g., `cave_flags`, `cave_vulns`).
//! Migrations are embedded in each module crate and run at startup.
//!
//! # Persistence layer
//!
//! Use [`persistence::Storage`] (object-safe trait) and the [`persistence::StorageExt`]
//! extension on `Arc<dyn Storage>` for typed CRUD from module handlers.

pub mod migrate;
pub mod persistence;
pub mod pool;

pub use persistence::{
    DiskStorage, Filter, FilterOp, MemoryStorage, PostgresStorage, Storage, StorageError,
    StorageExt, StorageResult,
};
pub use pool::CavePool;
