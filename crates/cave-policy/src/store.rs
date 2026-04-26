//! PostgreSQL store for cave-policy — persists policies, data documents, decision logs.

use cave_db::CavePool;

/// Schema migration v1: create all cave_policy tables.
pub const MIGRATION_V1: &str = r#"
-- OPA policies (Rego source)
CREATE TABLE IF NOT EXISTS policies (
    id TEXT PRIMARY KEY,
    raw TEXT NOT NULL,
    package_path TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- OPA data documents (JSON store)
CREATE TABLE IF NOT EXISTS data_documents (
    path TEXT PRIMARY KEY,   -- e.g. 'servers', 'users/alice'
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Kyverno ClusterPolicies
CREATE TABLE IF NOT EXISTS cluster_policies (
    name TEXT PRIMARY KEY,
    spec JSONB NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Kyverno namespaced Policies
CREATE TABLE IF NOT EXISTS kyverno_policies (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    spec JSONB NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (namespace, name)
);

-- Decision log
CREATE TABLE IF NOT EXISTS decision_log (
    decision_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    input JSONB,
    result JSONB,
    error TEXT,
    requested_by TEXT NOT NULL DEFAULT 'unknown',
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metrics JSONB,
    bundle_name TEXT,
    revision TEXT
);

CREATE INDEX IF NOT EXISTS idx_decision_log_path ON decision_log(path);
CREATE INDEX IF NOT EXISTS idx_decision_log_ts ON decision_log(timestamp DESC);

-- Bundle metadata
CREATE TABLE IF NOT EXISTS bundles (
    name TEXT PRIMARY KEY,
    active_revision TEXT,
    last_successful_activation TIMESTAMPTZ,
    last_successful_download TIMESTAMPTZ,
    last_successful_request TIMESTAMPTZ,
    config JSONB NOT NULL DEFAULT '{}'
);

-- Policy reports
CREATE TABLE IF NOT EXISTS policy_reports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace TEXT,
    policy TEXT NOT NULL,
    rule TEXT NOT NULL,
    resource_kind TEXT,
    resource_name TEXT,
    resource_namespace TEXT,
    result TEXT NOT NULL,  -- pass/fail/warn/error/skip
    message TEXT,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_policy_reports_ns ON policy_reports(namespace);
CREATE INDEX IF NOT EXISTS idx_policy_reports_policy ON policy_reports(policy);
"#;

/// Policy store operations.
pub struct PolicyStore<'a> {
    pub pool: &'a CavePool,
}

impl<'a> PolicyStore<'a> {
    pub fn new(pool: &'a CavePool) -> Self {
        Self { pool }
    }

    /// Initialize the cave_policy schema and run migrations.
    pub async fn migrate(&self) -> Result<(), String> {
        self.pool.ensure_schema("policy").await?;
        self.pool.migrate("policy", 1, MIGRATION_V1).await?;
        Ok(())
    }

    // ── OPA policy CRUD ───────────────────────────────────────────────────────

    pub async fn save_policy(&self, id: &str, raw: &str, package_path: &str) -> Result<(), String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        client
            .execute(
                "INSERT INTO cave_policy.policies (id, raw, package_path, updated_at) \
                 VALUES ($1, $2, $3, NOW()) \
                 ON CONFLICT (id) DO UPDATE SET raw = $2, package_path = $3, updated_at = NOW()",
                &[&id, &raw, &package_path],
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete_policy(&self, id: &str) -> Result<bool, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .execute("DELETE FROM cave_policy.policies WHERE id = $1", &[&id])
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows > 0)
    }

    pub async fn get_policy(&self, id: &str) -> Result<Option<(String, String)>, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = client
            .query_opt("SELECT raw, package_path FROM cave_policy.policies WHERE id = $1", &[&id])
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))))
    }

    pub async fn list_policies(&self) -> Result<Vec<(String, String)>, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .query("SELECT id, raw FROM cave_policy.policies ORDER BY id", &[])
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.iter().map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))).collect())
    }

    // ── OPA data document CRUD ────────────────────────────────────────────────

    pub async fn set_data(&self, path: &str, value: &serde_json::Value) -> Result<(), String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let value_pg = tokio_postgres::types::Json(value);
        client
            .execute(
                "INSERT INTO cave_policy.data_documents (path, value, updated_at) \
                 VALUES ($1, $2, NOW()) \
                 ON CONFLICT (path) DO UPDATE SET value = $2, updated_at = NOW()",
                &[&path, &value_pg],
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_data(&self, path: &str) -> Result<Option<serde_json::Value>, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = client
            .query_opt("SELECT value FROM cave_policy.data_documents WHERE path = $1", &[&path])
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|r| {
            let v: tokio_postgres::types::Json<serde_json::Value> = r.get(0);
            v.0
        }))
    }

    pub async fn delete_data(&self, path: &str) -> Result<bool, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .execute("DELETE FROM cave_policy.data_documents WHERE path = $1", &[&path])
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows > 0)
    }

    pub async fn load_all_data(&self) -> Result<serde_json::Value, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .query("SELECT path, value FROM cave_policy.data_documents", &[])
            .await
            .map_err(|e| e.to_string())?;
        let mut root = serde_json::Value::Object(Default::default());
        for row in &rows {
            let path: String = row.get(0);
            let value: tokio_postgres::types::Json<serde_json::Value> = row.get(1);
            let parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
            crate::rego::value::set_nested_data(&mut root, &parts, value.0);
        }
        Ok(root)
    }

    // ── Decision log ──────────────────────────────────────────────────────────

    pub async fn save_decision(&self, entry: &crate::models::DecisionLogEntry) -> Result<(), String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let input_pg = entry.input.as_ref().map(tokio_postgres::types::Json);
        let result_pg = entry.result.as_ref().map(tokio_postgres::types::Json);
        let metrics_pg = entry.metrics.as_ref().map(tokio_postgres::types::Json);
        client
            .execute(
                "INSERT INTO cave_policy.decision_log \
                 (decision_id, path, input, result, error, requested_by, timestamp, metrics, bundle_name, revision) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
                 ON CONFLICT (decision_id) DO NOTHING",
                &[
                    &entry.decision_id,
                    &entry.path,
                    &input_pg,
                    &result_pg,
                    &entry.error,
                    &entry.requested_by,
                    &entry.timestamp,
                    &metrics_pg,
                    &entry.bundle_name,
                    &entry.revision,
                ],
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_decisions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<crate::models::DecisionLogEntry>, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .query(
                "SELECT decision_id, path, input, result, error, requested_by, \
                 timestamp, metrics, bundle_name, revision \
                 FROM cave_policy.decision_log \
                 ORDER BY timestamp DESC LIMIT $1 OFFSET $2",
                &[&limit, &offset],
            )
            .await
            .map_err(|e| e.to_string())?;

        rows.iter()
            .map(|r| {
                Ok(crate::models::DecisionLogEntry {
                    decision_id: r.get(0),
                    path: r.get(1),
                    input: r.get::<_, Option<tokio_postgres::types::Json<serde_json::Value>>>(2).map(|j| j.0),
                    result: r.get::<_, Option<tokio_postgres::types::Json<serde_json::Value>>>(3).map(|j| j.0),
                    error: r.get(4),
                    requested_by: r.get(5),
                    timestamp: r.get(6),
                    metrics: r.get::<_, Option<tokio_postgres::types::Json<serde_json::Value>>>(7).map(|j| j.0),
                    bundle_name: r.get(8),
                    revision: r.get(9),
                })
            })
            .collect()
    }

    // ── Kyverno policy CRUD ───────────────────────────────────────────────────

    pub async fn save_cluster_policy(&self, policy: &crate::kyverno::models::ClusterPolicy) -> Result<(), String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let spec = tokio_postgres::types::Json(&policy.spec);
        let meta = tokio_postgres::types::Json(&policy.metadata);
        client
            .execute(
                "INSERT INTO cave_policy.cluster_policies (name, spec, metadata, updated_at) \
                 VALUES ($1, $2, $3, NOW()) \
                 ON CONFLICT (name) DO UPDATE SET spec = $2, metadata = $3, updated_at = NOW()",
                &[&policy.metadata.name, &spec, &meta],
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete_cluster_policy(&self, name: &str) -> Result<bool, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .execute("DELETE FROM cave_policy.cluster_policies WHERE name = $1", &[&name])
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows > 0)
    }

    pub async fn list_cluster_policies(&self) -> Result<Vec<crate::kyverno::models::ClusterPolicy>, String> {
        let client = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = client
            .query("SELECT spec, metadata FROM cave_policy.cluster_policies ORDER BY name", &[])
            .await
            .map_err(|e| e.to_string())?;
        let mut policies = Vec::new();
        for row in &rows {
            let spec: tokio_postgres::types::Json<crate::kyverno::models::PolicySpec> = row.get(0);
            let meta: tokio_postgres::types::Json<crate::kyverno::models::ObjectMeta> = row.get(1);
            policies.push(crate::kyverno::models::ClusterPolicy {
                api_version: "kyverno.io/v1".into(),
                kind: "ClusterPolicy".into(),
                metadata: meta.0,
                spec: spec.0,
                status: None,
            });
        }
        Ok(policies)
    }
}
