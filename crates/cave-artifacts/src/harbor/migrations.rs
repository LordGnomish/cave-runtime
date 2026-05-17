// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/common/dao/migration.go
//! PostgreSQL schema migrations for cave_registry.

use cave_db::CavePool;

const MODULE: &str = "registry";

pub const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS blobs (
    digest      TEXT        PRIMARY KEY,
    size        BIGINT      NOT NULL,
    content     BYTEA       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS repositories (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT        NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS manifests (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    repository      TEXT        NOT NULL,
    digest          TEXT        NOT NULL,
    media_type      TEXT        NOT NULL,
    content         BYTEA       NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(repository, digest)
);

CREATE INDEX IF NOT EXISTS idx_manifests_repo ON manifests(repository);

CREATE TABLE IF NOT EXISTS tags (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    repository      TEXT        NOT NULL,
    name            TEXT        NOT NULL,
    manifest_digest TEXT        NOT NULL,
    immutable       BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(repository, name)
);

CREATE INDEX IF NOT EXISTS idx_tags_repo ON tags(repository);

CREATE TABLE IF NOT EXISTS upload_sessions (
    id          UUID        PRIMARY KEY,
    repository  TEXT        NOT NULL,
    data        BYTEA       NOT NULL DEFAULT '',
    offset      BIGINT      NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '1 hour'
);

CREATE TABLE IF NOT EXISTS webhooks (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    repository  TEXT,
    url         TEXT        NOT NULL,
    events      TEXT[]      NOT NULL,
    enabled     BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS replication_targets (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT        NOT NULL UNIQUE,
    url         TEXT        NOT NULL,
    enabled     BOOLEAN     NOT NULL DEFAULT TRUE,
    username    TEXT,
    password    TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS repository_access (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    repository  TEXT        NOT NULL,
    subject     TEXT        NOT NULL,
    permission  TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(repository, subject)
);

CREATE TABLE IF NOT EXISTS tag_policies (
    repository      TEXT    PRIMARY KEY,
    all_immutable   BOOLEAN NOT NULL DEFAULT FALSE,
    immutable_tags  TEXT[]  NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS scan_results (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    manifest_digest TEXT        NOT NULL,
    scanner         TEXT        NOT NULL,
    status          TEXT        NOT NULL,
    result          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scan_results_digest ON scan_results(manifest_digest);
"#;

pub async fn run(pool: &CavePool) -> Result<(), String> {
    cave_db::migrate::run_migrations(pool, MODULE, &[(1, MIGRATION_1)]).await
}
