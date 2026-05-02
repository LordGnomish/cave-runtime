//! cave-registry — Harbor / Docker / OCI container registry.
//!
//! As of the multi-upstream consolidation, the implementation lives in
//! `cave_artifacts::harbor`. This crate is preserved as an alias so existing
//! dependents (`cave-runtime`, `cave-auth`, `cave-upstream`, `cave-scaffold`)
//! continue to compile against the `cave_registry::*` paths.
//!
//! New code should depend on `cave-artifacts` directly and reach for
//! `cave_artifacts::harbor::*`.

pub use cave_artifacts::harbor::{RegistryState, router};

pub const MODULE_NAME: &str = "registry";
