//! CAVE Core — shared types, configuration, and utilities for the Unified Runtime.

pub mod config;
pub mod error;
pub mod types;

pub use config::{CaveConfig, StorageBackend, StorageConfig};
pub use error::{CaveError, CaveResult};
