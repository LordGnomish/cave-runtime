//! PostgreSQL schema for rollouts, analysis templates, and analysis runs.

/// v1: core rollout schema.
pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS rollouts (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL,
    namespace    TEXT NOT NULL DEFAULT 'default',
    workload_ref JSONB NOT NULL,
    strategy     JSONB NOT NULL,
    status       JSONB NOT NULL DEFAULT '{"phase":"Pending","canary_weight":0,"conditions":[]}',
    traffic      JSONB,
    analysis     JSONB,
    notifications JSONB NOT NULL DEFAULT '[]',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_rollout_name_ns ON rollouts(namespace, name);
CREATE INDEX IF NOT EXISTS idx_rollout_phase ON rollouts((status->>'phase'));

CREATE TABLE IF NOT EXISTS analysis_templates (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    namespace   TEXT NOT NULL DEFAULT 'default',
    metrics     JSONB NOT NULL DEFAULT '[]',
    dry_run_metrics TEXT[] NOT NULL DEFAULT '{}',
    args        JSONB NOT NULL DEFAULT '[]',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_analysis_tmpl_name_ns ON analysis_templates(namespace, name);

CREATE TABLE IF NOT EXISTS analysis_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rollout_id    UUID NOT NULL REFERENCES rollouts(id) ON DELETE CASCADE,
    template_name TEXT NOT NULL,
    phase         TEXT NOT NULL DEFAULT 'Pending',
    metrics       JSONB NOT NULL DEFAULT '[]',
    args          JSONB NOT NULL DEFAULT '[]',
    started_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at  TIMESTAMPTZ,
    message       TEXT
);
CREATE INDEX IF NOT EXISTS idx_analysis_run_rollout ON analysis_runs(rollout_id);
CREATE INDEX IF NOT EXISTS idx_analysis_run_phase ON analysis_runs(phase);

CREATE TABLE IF NOT EXISTS rollout_events (
    id          BIGSERIAL PRIMARY KEY,
    rollout_id  UUID NOT NULL REFERENCES rollouts(id) ON DELETE CASCADE,
    event_type  TEXT NOT NULL,
    reason      TEXT NOT NULL,
    message     TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_rollout_events_rid ON rollout_events(rollout_id, occurred_at DESC);
"#;
