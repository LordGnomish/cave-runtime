//! CAVE Flags — Production-grade feature flag service.
//!
//! Full parity with Unleash v6.x, implemented natively in Rust.
//!
//! ## Feature set
//! - **8 strategy types**: default, userWithId, gradualRolloutUserId,
//!   gradualRolloutSessionId, gradualRolloutRandom, flexibleRollout,
//!   applicationHostname, remoteAddress
//! - **Constraints**: IN / NOT_IN / STR_* / NUM_* / DATE_* / SEMVER_* operators
//! - **Segments**: reusable constraint groups shared across strategies
//! - **Variants**: weight-based A/B selection with overrides and payloads
//! - **Projects and tags**
//! - **Toggle lifecycle**: active → potentially-stale → stale → archived
//! - **Impression data and usage metrics**
//! - **Client API** (`/api/client/*`) — SDK-facing endpoints
//! - **Admin API** (`/api/admin/*`) — management endpoints
//! - **Legacy CAVE API** (`/api/flags/*`) — backward-compatible endpoints
//!
//! ## Upstream tracking
//! - Spec: <https://github.com/Unleash/unleash>
//! - Parity target: Unleash v6.x
//!
//! ## Persistence
//! Persistence is handled by the `store::FlagStore` which talks to PostgreSQL
//! through the local `pool::FlagsPool` (deadpool-postgres under the hood).
//! Runtime state is kept in the tokio `RwLock` fields of `FlagsState` and
//! synced to the database at startup / on each mutation.

pub mod engine;
pub mod models;
pub mod pool;
pub mod routes;
pub mod store;
pub mod unleash;

use axum::Router;
use models::{Event, FeatureToggle, Project, Segment};
use pool::FlagsPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Module name used for the PostgreSQL schema (`cave_flags`).
pub const MODULE_NAME: &str = "flags";

// ================================================================
// Metrics entry (in-memory aggregation)
// ================================================================

/// Aggregated usage metrics for a single feature toggle.
#[derive(Debug, Default, Clone)]
pub struct MetricEntry {
    pub toggle_name: String,
    pub yes: u64,
    pub no: u64,
    pub variants: HashMap<String, u64>,
}

// ================================================================
// FlagsState
// ================================================================

/// Shared state injected into every request handler via axum `State`.
///
/// The in-memory stores (`features`, `segments`, `projects`, `events`,
/// `metrics`) act as the working set — populated from the database at
/// startup and kept in sync as mutations arrive.  This mirrors how
/// Unleash Server itself operates (in-memory cache + DB backend).
pub struct FlagsState {
    /// Optional DB pool for persistent storage.
    /// `None` = in-memory-only mode (useful for tests and standalone evaluation).
    pub pool: Option<Arc<FlagsPool>>,
    /// Active feature toggles, keyed by toggle name.
    pub features: RwLock<HashMap<String, FeatureToggle>>,
    /// Global segments (reusable constraint groups).
    pub segments: RwLock<Vec<Segment>>,
    /// Projects.
    pub projects: RwLock<Vec<Project>>,
    /// Audit event log.
    pub events: RwLock<Vec<Event>>,
    /// Aggregated toggle usage metrics.
    pub metrics: RwLock<HashMap<String, MetricEntry>>,
}

impl FlagsState {
    /// Create a `FlagsState` backed by a PostgreSQL pool.
    pub fn new(pool: Arc<FlagsPool>) -> Self {
        Self::with_pool(Some(pool))
    }

    /// Create a `FlagsState` in in-memory-only mode (no database).
    pub fn in_memory() -> Self {
        Self::with_pool(None)
    }

    fn with_pool(pool: Option<Arc<FlagsPool>>) -> Self {
        let now = chrono::Utc::now();
        let default_project = Project {
            id: "default".to_string(),
            name: "Default".to_string(),
            description: None,
            created_at: now,
            updated_at: now,
            health: 100,
            feature_count: 0,
            member_count: 1,
        };
        Self {
            pool,
            features: RwLock::new(HashMap::new()),
            segments: RwLock::new(vec![]),
            projects: RwLock::new(vec![default_project]),
            events: RwLock::new(vec![]),
            metrics: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for FlagsState {
    fn default() -> Self {
        Self::in_memory()
    }
}

// ================================================================
// Public router factory
// ================================================================

/// Create the axum router for the flags module.
///
/// Merges cave-native routes with the Unleash-compatible API so that
/// any Unleash client SDK can use this service as a drop-in replacement.
pub fn router(state: Arc<FlagsState>) -> Router {
    routes::create_router(Arc::clone(&state))
        .merge(unleash::unleash_router(state))
}
