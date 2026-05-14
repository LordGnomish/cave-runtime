// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PostgreSQL migrations for Harbor-compatible metadata
//! (projects, robots, audit, webhooks, quotas, labels, scan results).
//! The actual blob/manifest bytes live in RegistryStorage (in-memory / object store).

/// v1: core Harbor schema — projects, robots, audit, webhooks, quotas, labels.
pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,
    public      BOOLEAN NOT NULL DEFAULT false,
    owner_name  TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    repo_count  BIGINT NOT NULL DEFAULT 0,
    metadata    JSONB NOT NULL DEFAULT '{}',
    creation_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS robot_accounts (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    level        TEXT NOT NULL DEFAULT 'project',
    project_id   UUID REFERENCES projects(id) ON DELETE CASCADE,
    secret_hash  TEXT NOT NULL,
    expires_at   TIMESTAMPTZ,
    disabled     BOOLEAN NOT NULL DEFAULT false,
    permissions  JSONB NOT NULL DEFAULT '[]',
    creation_time TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_robot_name_project
    ON robot_accounts(name, project_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    id            BIGSERIAL PRIMARY KEY,
    username      TEXT NOT NULL,
    resource      TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    operation     TEXT NOT NULL,
    op_time       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_audit_op_time ON audit_logs(op_time DESC);
CREATE INDEX IF NOT EXISTS idx_audit_resource ON audit_logs(resource_type, operation);

CREATE TABLE IF NOT EXISTS webhook_policies (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    targets      JSONB NOT NULL DEFAULT '[]',
    event_types  TEXT[] NOT NULL DEFAULT '{}',
    enabled      BOOLEAN NOT NULL DEFAULT true,
    creation_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_webhook_name_proj ON webhook_policies(project_id, name);

CREATE TABLE IF NOT EXISTS quotas (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ref_id        BIGINT NOT NULL,
    ref_kind      TEXT NOT NULL DEFAULT 'project',
    ref_name      TEXT NOT NULL DEFAULT '',
    hard_count    BIGINT NOT NULL DEFAULT -1,
    hard_storage  BIGINT NOT NULL DEFAULT -1,
    used_count    BIGINT NOT NULL DEFAULT 0,
    used_storage  BIGINT NOT NULL DEFAULT 0,
    creation_date TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_date   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS labels (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    color         TEXT NOT NULL DEFAULT '#0000FF',
    scope         TEXT NOT NULL DEFAULT 'g',
    project_id    UUID REFERENCES projects(id) ON DELETE CASCADE,
    creation_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_label_name_scope ON labels(name, scope, project_id);
"#;

/// v2: scanning, replication, retention, immutable tag rules.
pub const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS scan_reports (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_digest TEXT NOT NULL,
    scan_status     TEXT NOT NULL DEFAULT 'not_scanned',
    severity        TEXT NOT NULL DEFAULT 'NONE',
    scanner         JSONB NOT NULL DEFAULT '{}',
    vulnerabilities JSONB NOT NULL DEFAULT '[]',
    start_time      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    end_time        TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_scan_digest ON scan_reports(artifact_digest);

CREATE TABLE IF NOT EXISTS replication_registries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL UNIQUE,
    url             TEXT NOT NULL,
    credential_type TEXT NOT NULL DEFAULT 'basic',
    access_key      TEXT,
    access_secret   TEXT,
    insecure        BOOLEAN NOT NULL DEFAULT false,
    creation_time   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS replication_policies (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name              TEXT NOT NULL UNIQUE,
    description       TEXT NOT NULL DEFAULT '',
    src_registry_id   UUID REFERENCES replication_registries(id),
    dest_registry_id  UUID REFERENCES replication_registries(id),
    dest_namespace    TEXT NOT NULL DEFAULT '',
    trigger           JSONB NOT NULL DEFAULT '{"trigger_type":"manual"}',
    filters           JSONB NOT NULL DEFAULT '[]',
    deletion          BOOLEAN NOT NULL DEFAULT false,
    override_dest     BOOLEAN NOT NULL DEFAULT false,
    enabled           BOOLEAN NOT NULL DEFAULT true,
    speed             INT,
    creation_time     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS replication_executions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id   UUID NOT NULL REFERENCES replication_policies(id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'InProgress',
    trigger     TEXT NOT NULL DEFAULT 'manual',
    start_time  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    end_time    TIMESTAMPTZ,
    succeeded   BIGINT NOT NULL DEFAULT 0,
    failed      BIGINT NOT NULL DEFAULT 0,
    in_progress BIGINT NOT NULL DEFAULT 0,
    stopped     BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS retention_policies (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scope       JSONB NOT NULL DEFAULT '{"level":"project","ref":0}',
    trigger     JSONB NOT NULL DEFAULT '{"kind":"Schedule"}',
    rules       JSONB NOT NULL DEFAULT '[]'
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_retention_project ON retention_policies(project_id);

CREATE TABLE IF NOT EXISTS immutable_tag_rules (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    disabled     BOOLEAN NOT NULL DEFAULT false,
    tag_selectors JSONB NOT NULL DEFAULT '[]',
    scope_selectors JSONB NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS preheat_providers (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,
    endpoint    TEXT NOT NULL,
    auth_mode   TEXT NOT NULL DEFAULT 'none',
    enabled     BOOLEAN NOT NULL DEFAULT true,
    status      TEXT NOT NULL DEFAULT 'healthy'
);

CREATE TABLE IF NOT EXISTS preheat_policies (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    provider_id  UUID NOT NULL REFERENCES preheat_providers(id),
    filters      JSONB,
    trigger      JSONB,
    enabled      BOOLEAN NOT NULL DEFAULT true,
    creation_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_time   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#;
