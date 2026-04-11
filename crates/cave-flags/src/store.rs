//! PostgreSQL store for feature flags.

use cave_db::CavePool;

/// Migration v1: create flags tables.
pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS flags (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    enabled BOOLEAN NOT NULL DEFAULT true,
    flag_type TEXT NOT NULL DEFAULT 'boolean',
    strategy JSONB NOT NULL DEFAULT '{"type": "default", "enabled": true}',
    environments TEXT[] NOT NULL DEFAULT '{}',
    tenant_id TEXT,
    kill_switch BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by UUID NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_flags_name ON flags(name);
CREATE INDEX IF NOT EXISTS idx_flags_tenant ON flags(tenant_id);
CREATE INDEX IF NOT EXISTS idx_flags_env ON flags USING GIN(environments);

-- Flag change audit log (feeds into Sovereign Ledger)
CREATE TABLE IF NOT EXISTS flag_audit (
    id BIGSERIAL PRIMARY KEY,
    flag_id UUID NOT NULL REFERENCES flags(id),
    action TEXT NOT NULL, -- 'created', 'updated', 'toggled', 'killed'
    old_value JSONB,
    new_value JSONB,
    changed_by UUID NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#;

/// Flag store operations.
pub struct FlagStore<'a> {
    pub pool: &'a CavePool,
}

impl<'a> FlagStore<'a> {
    pub fn new(pool: &'a CavePool) -> Self {
        Self { pool }
    }

    // TODO: implement CRUD operations
    // - list_flags(tenant_id, env) -> Vec<FeatureFlag>
    // - get_flag(name) -> Option<FeatureFlag>
    // - create_flag(req) -> FeatureFlag
    // - update_flag(id, req) -> FeatureFlag
    // - toggle_flag(id, enabled) -> FeatureFlag
    // - kill_flag(id) -> FeatureFlag (sets kill_switch = true)
    // - delete_flag(id) -> ()
}
