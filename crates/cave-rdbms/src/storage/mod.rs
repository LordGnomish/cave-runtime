//! In-memory storage backend.

pub mod schema;
pub mod catalog;
pub mod transaction;
pub mod vacuum;

pub use schema::{Database, Schema, Table};
pub use catalog::SystemCatalog;
pub use transaction::Transaction;
pub use vacuum::{AutovacuumSettings, VacuumRegistry, VacuumStats};
