// SPDX-License-Identifier: AGPL-3.0-or-later
//! SQL schema for the catalog module.
//!
//! Upstream: backstage/plugins/catalog-backend/src/database/migrations/
//!
//! Run `CATALOG_SCHEMA_SQL` once on startup to ensure all tables exist.

/// DDL for all catalog tables.  Safe to run repeatedly (`CREATE … IF NOT EXISTS`).
pub const CATALOG_SCHEMA_SQL: &str = "
CREATE SCHEMA IF NOT EXISTS cave_portal;

CREATE TABLE IF NOT EXISTS cave_portal.catalog_entities (
    uid         TEXT        PRIMARY KEY,
    api_version TEXT        NOT NULL,
    kind        TEXT        NOT NULL,
    namespace   TEXT        NOT NULL DEFAULT 'default',
    name        TEXT        NOT NULL,
    title       TEXT,
    spec        JSONB,
    metadata    JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(namespace, kind, name)
);

CREATE INDEX IF NOT EXISTS idx_catalog_entities_kind
    ON cave_portal.catalog_entities(kind);
CREATE INDEX IF NOT EXISTS idx_catalog_entities_namespace
    ON cave_portal.catalog_entities(namespace);

CREATE TABLE IF NOT EXISTS cave_portal.catalog_locations (
    id          TEXT        PRIMARY KEY,
    type        TEXT        NOT NULL,
    target      TEXT        NOT NULL UNIQUE,
    presence    TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS cave_portal.catalog_entity_search (
    entity_uid  TEXT        NOT NULL REFERENCES cave_portal.catalog_entities(uid) ON DELETE CASCADE,
    key         TEXT        NOT NULL,
    value       TEXT,
    PRIMARY KEY (entity_uid, key)
);

CREATE TABLE IF NOT EXISTS cave_portal.catalog_refresh_state (
    entity_ref          TEXT        PRIMARY KEY,
    unprocessed_entity  JSONB       NOT NULL,
    errors              TEXT,
    next_update_at      TIMESTAMPTZ,
    last_discovery_at   TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
";
