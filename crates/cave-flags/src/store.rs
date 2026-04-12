//! PostgreSQL persistence layer for cave-flags.
//!
//! All tables live in the `cave_flags` schema (created via `FlagsPool::ensure_schema`).
//! The schema evolves through versioned migrations applied idempotently at startup.
//!
//! ## Schema summary
//! | Table                 | Purpose                                              |
//! |-----------------------|------------------------------------------------------|
//! | `features`            | Feature toggle definitions (strategies/variants JSONB) |
//! | `feature_environments`| Per-environment enable/disable + strategy overrides  |
//! | `segments`            | Reusable constraint groups                          |
//! | `projects`            | Project metadata                                    |
//! | `events`              | Append-only audit log (feeds Sovereign Ledger)      |
//! | `client_metrics`      | SDK usage counters (yes/no/variant per toggle)      |
//! | `client_instances`    | Registered SDK client instances                     |

use crate::pool::FlagsPool;

// ================================================================
// Migrations
// ================================================================

/// V1: Core feature toggle tables.
pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS features (
    name             TEXT PRIMARY KEY,
    description      TEXT NOT NULL DEFAULT '',
    project          TEXT NOT NULL DEFAULT 'default',
    enabled          BOOLEAN NOT NULL DEFAULT true,
    archived         BOOLEAN NOT NULL DEFAULT false,
    stale            BOOLEAN NOT NULL DEFAULT false,
    impression_data  BOOLEAN NOT NULL DEFAULT false,
    toggle_type      TEXT NOT NULL DEFAULT 'release',
    strategies       JSONB NOT NULL DEFAULT '[]',
    variants         JSONB NOT NULL DEFAULT '[]',
    tags             JSONB NOT NULL DEFAULT '[]',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by       TEXT NOT NULL DEFAULT 'system',
    last_seen_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_features_project  ON features(project);
CREATE INDEX IF NOT EXISTS idx_features_archived ON features(archived);

CREATE TABLE IF NOT EXISTS feature_environments (
    feature_name TEXT NOT NULL REFERENCES features(name) ON DELETE CASCADE,
    environment  TEXT NOT NULL,
    enabled      BOOLEAN NOT NULL DEFAULT true,
    strategies   JSONB NOT NULL DEFAULT '[]',
    variants     JSONB NOT NULL DEFAULT '[]',
    PRIMARY KEY (feature_name, environment)
);

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO projects (id, name, description) VALUES ('default', 'Default', 'Default project')
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS segments (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    constraints JSONB NOT NULL DEFAULT '[]',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by  TEXT NOT NULL DEFAULT 'system'
);

CREATE TABLE IF NOT EXISTS events (
    id           BIGSERIAL PRIMARY KEY,
    event_type   TEXT NOT NULL,
    created_by   TEXT NOT NULL,
    data         JSONB,
    pre_data     JSONB,
    feature_name TEXT,
    project      TEXT,
    environment  TEXT,
    tags         JSONB NOT NULL DEFAULT '[]',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_events_feature ON events(feature_name);
CREATE INDEX IF NOT EXISTS idx_events_type    ON events(event_type);
"#;

/// V2: Metrics and client instance tracking.
pub const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS client_metrics (
    id             BIGSERIAL PRIMARY KEY,
    feature_name   TEXT NOT NULL,
    app_name       TEXT NOT NULL,
    environment    TEXT NOT NULL DEFAULT 'default',
    timestamp      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    yes_count      BIGINT NOT NULL DEFAULT 0,
    no_count       BIGINT NOT NULL DEFAULT 0,
    variant_counts JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_metrics_feature
    ON client_metrics(feature_name, timestamp DESC);

CREATE TABLE IF NOT EXISTS client_instances (
    id            BIGSERIAL PRIMARY KEY,
    app_name      TEXT NOT NULL,
    instance_id   TEXT NOT NULL,
    sdk_version   TEXT,
    strategies    TEXT[] NOT NULL DEFAULT '{}',
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(app_name, instance_id)
);
"#;

/// All migrations in version order.
pub const MIGRATIONS: &[(i32, &str)] = &[(1, MIGRATION_V1), (2, MIGRATION_V2)];

// ================================================================
// Flag Store
// ================================================================

/// Persistence layer for the flags module.
///
/// All async CRUD methods use `FlagsPool` to acquire connections from
/// the deadpool-postgres pool.  The public methods are fully typed and
/// ready for implementation; they return `Ok(…)` stubs today and will
/// be wired to real SQL as `cave-flags` matures.
pub struct FlagStore<'a> {
    pub pool: &'a FlagsPool,
}

impl<'a> FlagStore<'a> {
    pub fn new(pool: &'a FlagsPool) -> Self {
        Self { pool }
    }

    // ── Startup ──────────────────────────────────────────────────

    /// Apply all pending schema migrations.
    pub async fn migrate(&self) -> Result<(), String> {
        self.pool.run_migrations(MIGRATIONS).await
    }

    // ── Features ─────────────────────────────────────────────────

    /// List all non-archived feature toggles, optionally filtered by project.
    ///
    /// ```sql
    /// SELECT * FROM cave_flags.features
    /// WHERE archived = false [AND project = $1]
    /// ORDER BY name
    /// ```
    pub async fn list_features(
        &self,
        project: Option<&str>,
    ) -> Result<Vec<crate::models::FeatureToggle>, String> {
        let _ = project;
        Ok(vec![])
    }

    /// Fetch a single toggle by name.
    pub async fn get_feature(
        &self,
        name: &str,
    ) -> Result<Option<crate::models::FeatureToggle>, String> {
        let _ = name;
        Ok(None)
    }

    /// Persist a newly created toggle.
    pub async fn create_feature(
        &self,
        toggle: &crate::models::FeatureToggle,
    ) -> Result<(), String> {
        let _ = toggle;
        Ok(())
    }

    /// Update an existing toggle's mutable fields.
    pub async fn update_feature(
        &self,
        toggle: &crate::models::FeatureToggle,
    ) -> Result<(), String> {
        let _ = toggle;
        Ok(())
    }

    /// Archive (soft-delete) a toggle.
    pub async fn archive_feature(&self, name: &str) -> Result<(), String> {
        let _ = name;
        Ok(())
    }

    // ── Feature environments ─────────────────────────────────────

    /// Enable or disable a toggle for a specific environment.
    pub async fn set_environment_enabled(
        &self,
        feature_name: &str,
        environment: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let _ = (feature_name, environment, enabled);
        Ok(())
    }

    // ── Segments ─────────────────────────────────────────────────

    /// List all segments.
    pub async fn list_segments(&self) -> Result<Vec<crate::models::Segment>, String> {
        Ok(vec![])
    }

    /// Fetch a segment by ID.
    pub async fn get_segment(&self, id: i64) -> Result<Option<crate::models::Segment>, String> {
        let _ = id;
        Ok(None)
    }

    /// Create a new segment.
    pub async fn create_segment(
        &self,
        segment: &crate::models::Segment,
    ) -> Result<(), String> {
        let _ = segment;
        Ok(())
    }

    // ── Events ───────────────────────────────────────────────────

    /// Append an event to the audit log.
    pub async fn append_event(&self, event: &crate::models::Event) -> Result<(), String> {
        let _ = event;
        Ok(())
    }

    /// List recent events, newest first.
    pub async fn list_events(&self, limit: i64) -> Result<Vec<crate::models::Event>, String> {
        let _ = limit;
        Ok(vec![])
    }

    // ── Metrics ──────────────────────────────────────────────────

    /// Record a metrics bucket from a client SDK.
    pub async fn record_metrics(
        &self,
        app_name: &str,
        environment: &str,
        toggles: &std::collections::HashMap<String, crate::models::ToggleMetrics>,
    ) -> Result<(), String> {
        let _ = (app_name, environment, toggles);
        Ok(())
    }

    // ── Client instances ─────────────────────────────────────────

    /// Upsert a client SDK registration (ON CONFLICT UPDATE last_seen).
    pub async fn upsert_client_instance(
        &self,
        app_name: &str,
        instance_id: &str,
        sdk_version: Option<&str>,
        strategies: &[String],
    ) -> Result<(), String> {
        let _ = (app_name, instance_id, sdk_version, strategies);
        Ok(())
    }
}
