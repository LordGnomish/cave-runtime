// SPDX-License-Identifier: AGPL-3.0-or-later
//! PostgreSQL schema for the flags module (Unleash-compatible).

/// v1: feature flags, environments, projects, segments, variants.
pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    description         TEXT NOT NULL DEFAULT '',
    default_stickiness  TEXT NOT NULL DEFAULT 'default',
    mode                TEXT NOT NULL DEFAULT 'open',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO projects (id, name) VALUES ('default', 'Default')
    ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS environments (
    name       TEXT PRIMARY KEY,
    type       TEXT NOT NULL DEFAULT 'production',
    enabled    BOOLEAN NOT NULL DEFAULT true,
    protected  BOOLEAN NOT NULL DEFAULT false,
    sort_order INT NOT NULL DEFAULT 0
);
INSERT INTO environments (name, type, sort_order) VALUES
    ('development', 'development', 1),
    ('staging',     'staging',     2),
    ('production',  'production',  3)
    ON CONFLICT (name) DO NOTHING;

CREATE TABLE IF NOT EXISTS features (
    name            TEXT PRIMARY KEY,
    feature_type    TEXT NOT NULL DEFAULT 'release',
    description     TEXT NOT NULL DEFAULT '',
    enabled         BOOLEAN NOT NULL DEFAULT true,
    stale           BOOLEAN NOT NULL DEFAULT false,
    impression_data BOOLEAN NOT NULL DEFAULT false,
    project         TEXT NOT NULL DEFAULT 'default' REFERENCES projects(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at    TIMESTAMPTZ,
    archived_at     TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_features_project ON features(project);
CREATE INDEX IF NOT EXISTS idx_features_type ON features(feature_type);
CREATE INDEX IF NOT EXISTS idx_features_stale ON features(stale);

CREATE TABLE IF NOT EXISTS feature_environments (
    feature_name TEXT NOT NULL REFERENCES features(name) ON DELETE CASCADE,
    environment  TEXT NOT NULL REFERENCES environments(name),
    enabled      BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (feature_name, environment)
);

CREATE TABLE IF NOT EXISTS feature_strategies (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    feature_name TEXT NOT NULL REFERENCES features(name) ON DELETE CASCADE,
    environment  TEXT NOT NULL REFERENCES environments(name),
    name         TEXT NOT NULL,
    parameters   JSONB NOT NULL DEFAULT '{}',
    constraints  JSONB NOT NULL DEFAULT '[]',
    segments     BIGINT[] NOT NULL DEFAULT '{}',
    sort_order   INT NOT NULL DEFAULT 0,
    disabled     BOOLEAN NOT NULL DEFAULT false,
    variants     JSONB NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS idx_strategies_feature_env ON feature_strategies(feature_name, environment);

CREATE TABLE IF NOT EXISTS feature_variants (
    feature_name  TEXT NOT NULL REFERENCES features(name) ON DELETE CASCADE,
    environment   TEXT NOT NULL REFERENCES environments(name),
    name          TEXT NOT NULL,
    weight        INT NOT NULL DEFAULT 1000,
    weight_type   TEXT NOT NULL DEFAULT 'variable',
    stickiness    TEXT NOT NULL DEFAULT 'default',
    payload_type  TEXT,
    payload_value TEXT,
    overrides     JSONB NOT NULL DEFAULT '[]',
    sort_order    INT NOT NULL DEFAULT 0,
    PRIMARY KEY (feature_name, environment, name)
);

CREATE TABLE IF NOT EXISTS segments (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT,
    constraints JSONB NOT NULL DEFAULT '[]',
    project     TEXT REFERENCES projects(id),
    created_by  TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS tags (
    feature_name TEXT NOT NULL REFERENCES features(name) ON DELETE CASCADE,
    tag_type     TEXT NOT NULL DEFAULT 'simple',
    value        TEXT NOT NULL,
    PRIMARY KEY (feature_name, tag_type, value)
);
"#;

/// v2: tokens, metrics, context fields, banners, change requests, impressions.
pub const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS api_tokens (
    secret       TEXT PRIMARY KEY,
    username     TEXT NOT NULL,
    token_type   TEXT NOT NULL DEFAULT 'client',
    environment  TEXT REFERENCES environments(name),
    projects     TEXT[] NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at   TIMESTAMPTZ,
    seen_at      TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_tokens_type ON api_tokens(token_type);

CREATE TABLE IF NOT EXISTS metrics (
    id           BIGSERIAL PRIMARY KEY,
    app_name     TEXT NOT NULL,
    instance_id  TEXT NOT NULL,
    feature_name TEXT NOT NULL,
    yes_count    BIGINT NOT NULL DEFAULT 0,
    no_count     BIGINT NOT NULL DEFAULT 0,
    bucket_start TIMESTAMPTZ NOT NULL,
    bucket_stop  TIMESTAMPTZ NOT NULL,
    variants     JSONB NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_metrics_feature ON metrics(feature_name, bucket_start DESC);

CREATE TABLE IF NOT EXISTS client_applications (
    app_name    TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    sdk_version TEXT,
    strategies  TEXT[] NOT NULL DEFAULT '{}',
    started     TIMESTAMPTZ,
    interval    INT NOT NULL DEFAULT 15000,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_name, instance_id)
);

CREATE TABLE IF NOT EXISTS context_fields (
    name         TEXT PRIMARY KEY,
    description  TEXT NOT NULL DEFAULT '',
    legal_values JSONB NOT NULL DEFAULT '[]',
    stickiness   BOOLEAN NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO context_fields (name, description, stickiness) VALUES
    ('userId',        'The user ID',         true),
    ('sessionId',     'The session ID',      true),
    ('remoteAddress', 'The remote address',  false),
    ('environment',   'The environment',     false),
    ('appName',       'The application name',false),
    ('currentTime',   'The current time',    false)
    ON CONFLICT (name) DO NOTHING;

CREATE TABLE IF NOT EXISTS banners (
    id         BIGSERIAL PRIMARY KEY,
    message    TEXT NOT NULL,
    variant    TEXT NOT NULL DEFAULT 'info',
    link       TEXT,
    link_text  TEXT,
    enabled    BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS change_requests (
    id           BIGSERIAL PRIMARY KEY,
    title        TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'Draft',
    project      TEXT NOT NULL REFERENCES projects(id),
    environment  TEXT NOT NULL REFERENCES environments(name),
    min_approvals INT NOT NULL DEFAULT 1,
    approvals    JSONB NOT NULL DEFAULT '[]',
    rejections   JSONB NOT NULL DEFAULT '[]',
    changes      JSONB NOT NULL DEFAULT '[]',
    created_by   TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_cr_project_env ON change_requests(project, environment, state);

CREATE TABLE IF NOT EXISTS impression_events (
    id           BIGSERIAL PRIMARY KEY,
    event_type   TEXT NOT NULL,
    enabled      BOOLEAN,
    variant      TEXT,
    feature_name TEXT NOT NULL,
    app_name     TEXT,
    environment  TEXT,
    user_id      TEXT,
    occurred_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_impression_feature ON impression_events(feature_name, occurred_at DESC);
"#;
