//! CAVE DB — shared PostgreSQL connection pool and migration runner.
//!
//! Each module gets its own schema (e.g., `cave_flags`, `cave_vulns`).
//! Migrations are embedded in each module crate and run at startup.

pub mod pool;
pub mod migrate;

pub use pool::CavePool;
