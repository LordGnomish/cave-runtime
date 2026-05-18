// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Core — shared types, configuration, and utilities for the Unified Runtime.
//!
//! This crate defines the foundational types used across all CAVE modules:
//! - **Profile system** (Principle 3): 7 deployment profiles (3 envs × 2 providers + local)
//! - **Tenant model** (Principle 8): Three-tier isolation (soft/hard/dedicated)
//! - **Identity types** (ADR-129): Canonical `cave_uid` user identity
//! - **Error types**: Unified error handling with HTTP status codes
//! - **Configuration**: YAML-based runtime config with profile-aware defaults

pub mod config;
pub mod error;
pub mod labels;
pub mod profile;
pub mod tenant;
pub mod types;
pub mod wal;

pub use labels::Labels;

pub use config::CaveConfig;
pub use error::{CaveError, CaveResult};
pub use profile::{
    DatabaseBackend, DeploymentProfile, Environment, ProfileConfig, ProfileModules,
    ProfileResources, Provider,
};
pub use tenant::{
    DataClassification, Tenant, TenantEnvironment, TenantRateLimits, TenantState, TenantTier,
    WorkloadCriticality,
};
