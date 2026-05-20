// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lakehouse wrap — native Iceberg + DataFusion catalog & query UI.
//!
//! Replaces the Iceberg REST UI and any vendor lakehouse console. Tenants
//! browse their own catalogs / namespaces / tables, run DataFusion SQL
//! queries scoped to their tenant, and review snapshot history for
//! time-travel queries. **No** redirect to a vendor UI exists.
//!
//! Panels (per ADR-147 portal contract):
//!   * `dashboard` — catalog list, table count, total bytes, snapshot stats
//!   * `tables`    — browser with schema view per table
//!   * `query`     — DataFusion SQL editor + result preview
//!   * `time_travel` — snapshot list per table with summary stats

use super::ViewPersona;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Catalog / namespace / table model ────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub name: String,
    pub tenant: String,
    pub warehouse_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    pub catalog: String,
    pub name: String,
    pub tenant: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSummary {
    pub catalog: String,
    pub namespace: String,
    pub name: String,
    pub tenant: String,
    pub current_snapshot_id: Option<i64>,
    pub partition_keys: Vec<String>,
    pub data_files: u64,
    pub data_bytes: u64,
    pub small_files: u64,
}

impl TableSummary {
    /// Heuristic: tables with > 100 small files are compaction candidates.
    pub fn needs_compaction(&self) -> bool {
        self.small_files > 100
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub catalog: String,
    pub namespace: String,
    pub table: String,
    pub snapshot_id: i64,
    pub parent_snapshot_id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub operation: SnapshotOperation,
    pub added_files: u64,
    pub removed_files: u64,
    pub added_bytes: u64,
    pub removed_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotOperation {
    Append,
    Replace,
    Overwrite,
    Delete,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LakehouseError {
    #[error("catalog {0:?} not found")]
    CatalogNotFound(String),
    #[error("namespace {0:?} not found in catalog {1:?}")]
    NamespaceNotFound(String, String),
    #[error("table {0:?} not found")]
    TableNotFound(String),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("invalid SQL: {0}")]
    InvalidSql(String),
    #[error("cross-tenant access blocked: {0}")]
    CrossTenant(String),
}

// ── Plugin state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct LakehousePlugin {
    catalogs: Vec<Catalog>,
    namespaces: Vec<Namespace>,
    tables: Vec<TableSummary>,
    snapshots: Vec<SnapshotEntry>,
}

impl LakehousePlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_catalog(&mut self, catalog: Catalog) -> Result<(), LakehouseError> {
        validate_ident(&catalog.name)?;
        self.catalogs.retain(|c| c.name != catalog.name);
        self.catalogs.push(catalog);
        Ok(())
    }

    pub fn register_namespace(&mut self, ns: Namespace) -> Result<(), LakehouseError> {
        validate_ident(&ns.name)?;
        if !self.catalogs.iter().any(|c| c.name == ns.catalog) {
            return Err(LakehouseError::CatalogNotFound(ns.catalog.clone()));
        }
        self.namespaces
            .retain(|n| !(n.catalog == ns.catalog && n.name == ns.name));
        self.namespaces.push(ns);
        Ok(())
    }

    pub fn register_table(&mut self, table: TableSummary) -> Result<(), LakehouseError> {
        validate_ident(&table.name)?;
        if !self
            .namespaces
            .iter()
            .any(|n| n.catalog == table.catalog && n.name == table.namespace)
        {
            return Err(LakehouseError::NamespaceNotFound(
                table.namespace.clone(),
                table.catalog.clone(),
            ));
        }
        self.tables.retain(|t| {
            !(t.catalog == table.catalog && t.namespace == table.namespace && t.name == table.name)
        });
        self.tables.push(table);
        Ok(())
    }

    pub fn record_snapshot(&mut self, snap: SnapshotEntry) {
        self.snapshots.push(snap);
    }

    /// Dashboard panel summary for the home page.
    pub fn dashboard(&self, persona: ViewPersona, tenant: &str) -> DashboardPanel {
        let visible = |t: &&TableSummary| persona == ViewPersona::Admin || t.tenant == tenant;
        let tables: Vec<&TableSummary> = self.tables.iter().filter(visible).collect();
        let total_bytes: u64 = tables.iter().map(|t| t.data_bytes).sum();
        let total_files: u64 = tables.iter().map(|t| t.data_files).sum();
        let small_files: u64 = tables.iter().map(|t| t.small_files).sum();
        let needs_compaction: usize = tables.iter().filter(|t| t.needs_compaction()).count();
        let catalog_names: Vec<String> = self
            .catalogs
            .iter()
            .filter(|c| persona == ViewPersona::Admin || c.tenant == tenant)
            .map(|c| c.name.clone())
            .collect();
        DashboardPanel {
            catalogs: catalog_names,
            table_count: tables.len(),
            total_bytes,
            total_files,
            small_files,
            needs_compaction,
            snapshot_count: self.snapshots.len(),
        }
    }

    pub fn list_catalogs(&self, persona: ViewPersona, tenant: &str) -> Vec<&Catalog> {
        self.catalogs
            .iter()
            .filter(|c| persona == ViewPersona::Admin || c.tenant == tenant)
            .collect()
    }

    pub fn list_namespaces<'a>(
        &'a self,
        catalog: &str,
        persona: ViewPersona,
        tenant: &str,
    ) -> Vec<&'a Namespace> {
        self.namespaces
            .iter()
            .filter(|n| n.catalog == catalog)
            .filter(|n| persona == ViewPersona::Admin || n.tenant == tenant)
            .collect()
    }

    pub fn list_tables<'a>(
        &'a self,
        catalog: &str,
        namespace: &str,
        persona: ViewPersona,
        tenant: &str,
    ) -> Vec<&'a TableSummary> {
        self.tables
            .iter()
            .filter(|t| t.catalog == catalog && t.namespace == namespace)
            .filter(|t| persona == ViewPersona::Admin || t.tenant == tenant)
            .collect()
    }

    pub fn snapshot_history<'a>(
        &'a self,
        catalog: &str,
        namespace: &str,
        table: &str,
    ) -> Vec<&'a SnapshotEntry> {
        let mut entries: Vec<&SnapshotEntry> = self
            .snapshots
            .iter()
            .filter(|s| s.catalog == catalog && s.namespace == namespace && s.table == table)
            .collect();
        entries.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
        entries
    }

    /// Validate a SQL query: forbid DDL/DML/cross-tenant references.
    pub fn validate_query(
        &self,
        sql: &str,
        persona: ViewPersona,
        tenant: &str,
    ) -> Result<QueryPlanPreview, LakehouseError> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Err(LakehouseError::InvalidSql("empty query".into()));
        }
        let upper = trimmed.to_ascii_uppercase();
        if !upper.starts_with("SELECT") && !upper.starts_with("EXPLAIN") {
            return Err(LakehouseError::InvalidSql(
                "only SELECT / EXPLAIN allowed in portal".into(),
            ));
        }
        for forbidden in [
            "DROP ", "DELETE ", "UPDATE ", "INSERT ", "CREATE ", "ALTER ",
        ] {
            if upper.contains(forbidden) {
                return Err(LakehouseError::InvalidSql(format!(
                    "forbidden keyword: {}",
                    forbidden.trim()
                )));
            }
        }
        // Enforce tenant scoping for non-admin personas.
        if persona != ViewPersona::Admin {
            for cat in self.catalogs.iter().filter(|c| c.tenant != tenant) {
                if upper.contains(&cat.name.to_ascii_uppercase()) {
                    return Err(LakehouseError::CrossTenant(cat.name.clone()));
                }
            }
        }
        Ok(QueryPlanPreview {
            sql: trimmed.to_string(),
            scoped_to_tenant: persona != ViewPersona::Admin,
            tenant: tenant.to_string(),
        })
    }
}

// ── View-model panels ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub catalogs: Vec<String>,
    pub table_count: usize,
    pub total_bytes: u64,
    pub total_files: u64,
    pub small_files: u64,
    pub needs_compaction: usize,
    pub snapshot_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryPlanPreview {
    pub sql: String,
    pub scoped_to_tenant: bool,
    pub tenant: String,
}

// ── Validation ───────────────────────────────────────────────────────────────

fn validate_ident(name: &str) -> Result<(), LakehouseError> {
    if name.is_empty() || name.len() > 128 {
        return Err(LakehouseError::InvalidSql(format!(
            "invalid identifier length: {name:?}"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(LakehouseError::InvalidSql(format!(
            "invalid char in identifier: {name:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog(name: &str, tenant: &str) -> Catalog {
        Catalog {
            name: name.into(),
            tenant: tenant.into(),
            warehouse_uri: format!("s3://acme/{name}"),
        }
    }

    fn sample_ns(catalog: &str, name: &str, tenant: &str) -> Namespace {
        Namespace {
            catalog: catalog.into(),
            name: name.into(),
            tenant: tenant.into(),
        }
    }

    fn sample_table(catalog: &str, ns: &str, name: &str, tenant: &str) -> TableSummary {
        TableSummary {
            catalog: catalog.into(),
            namespace: ns.into(),
            name: name.into(),
            tenant: tenant.into(),
            current_snapshot_id: Some(1),
            partition_keys: vec!["dt".into()],
            data_files: 200,
            data_bytes: 1_000_000,
            small_files: 150,
        }
    }

    #[test]
    fn register_catalog_and_namespace() {
        let mut p = LakehousePlugin::new();
        p.register_catalog(sample_catalog("acme_warehouse", "acme"))
            .unwrap();
        p.register_namespace(sample_ns("acme_warehouse", "raw", "acme"))
            .unwrap();
        assert_eq!(p.list_catalogs(ViewPersona::Admin, "any").len(), 1);
        assert_eq!(
            p.list_namespaces("acme_warehouse", ViewPersona::Admin, "any")
                .len(),
            1
        );
    }

    #[test]
    fn register_namespace_requires_existing_catalog() {
        let mut p = LakehousePlugin::new();
        let err = p
            .register_namespace(sample_ns("missing", "raw", "acme"))
            .unwrap_err();
        assert!(matches!(err, LakehouseError::CatalogNotFound(_)));
    }

    #[test]
    fn register_table_requires_existing_namespace() {
        let mut p = LakehousePlugin::new();
        p.register_catalog(sample_catalog("c", "acme")).unwrap();
        let err = p
            .register_table(sample_table("c", "missing", "events", "acme"))
            .unwrap_err();
        assert!(matches!(err, LakehouseError::NamespaceNotFound(_, _)));
    }

    #[test]
    fn list_tables_scopes_to_tenant() {
        let mut p = LakehousePlugin::new();
        p.register_catalog(sample_catalog("c1", "acme")).unwrap();
        p.register_catalog(sample_catalog("c2", "globex")).unwrap();
        p.register_namespace(sample_ns("c1", "raw", "acme"))
            .unwrap();
        p.register_namespace(sample_ns("c2", "raw", "globex"))
            .unwrap();
        p.register_table(sample_table("c1", "raw", "ev_a", "acme"))
            .unwrap();
        p.register_table(sample_table("c2", "raw", "ev_b", "globex"))
            .unwrap();
        assert_eq!(
            p.list_tables("c1", "raw", ViewPersona::Tenant, "acme")
                .len(),
            1
        );
        assert_eq!(
            p.list_tables("c2", "raw", ViewPersona::Tenant, "acme")
                .len(),
            0
        );
        assert_eq!(p.list_tables("c1", "raw", ViewPersona::Admin, "x").len(), 1);
    }

    #[test]
    fn dashboard_aggregates_table_metrics() {
        let mut p = LakehousePlugin::new();
        p.register_catalog(sample_catalog("c", "acme")).unwrap();
        p.register_namespace(sample_ns("c", "raw", "acme")).unwrap();
        p.register_table(sample_table("c", "raw", "events", "acme"))
            .unwrap();
        let panel = p.dashboard(ViewPersona::Admin, "acme");
        assert_eq!(panel.table_count, 1);
        assert_eq!(panel.total_bytes, 1_000_000);
        assert_eq!(panel.small_files, 150);
        assert_eq!(panel.needs_compaction, 1);
        assert_eq!(panel.catalogs, vec!["c".to_string()]);
    }

    #[test]
    fn snapshot_history_sorts_descending_by_timestamp() {
        let mut p = LakehousePlugin::new();
        let now = Utc::now();
        for (i, op) in [
            (1_i64, SnapshotOperation::Append),
            (2, SnapshotOperation::Append),
            (3, SnapshotOperation::Overwrite),
        ] {
            p.record_snapshot(SnapshotEntry {
                catalog: "c".into(),
                namespace: "raw".into(),
                table: "events".into(),
                snapshot_id: i,
                parent_snapshot_id: if i == 1 { None } else { Some(i - 1) },
                timestamp: now + chrono::Duration::seconds(i),
                operation: op,
                added_files: 1,
                removed_files: 0,
                added_bytes: 100,
                removed_bytes: 0,
            });
        }
        let hist = p.snapshot_history("c", "raw", "events");
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[0].snapshot_id, 3);
        assert_eq!(hist[2].snapshot_id, 1);
    }

    #[test]
    fn validate_query_blocks_forbidden_keywords() {
        let p = LakehousePlugin::new();
        for sql in [
            "",
            "DROP TABLE foo",
            "DELETE FROM bar",
            "UPDATE baz SET x = 1",
            "INSERT INTO qux VALUES (1)",
            "CREATE TABLE quux ()",
            "ALTER TABLE q ADD COLUMN c TEXT",
        ] {
            let err = p
                .validate_query(sql, ViewPersona::Admin, "acme")
                .unwrap_err();
            assert!(matches!(err, LakehouseError::InvalidSql(_)));
        }
    }

    #[test]
    fn validate_query_allows_select_and_explain() {
        let p = LakehousePlugin::new();
        let r = p
            .validate_query("SELECT * FROM events", ViewPersona::Tenant, "acme")
            .unwrap();
        assert!(r.scoped_to_tenant);
        let r = p
            .validate_query("EXPLAIN SELECT 1", ViewPersona::Operator, "acme")
            .unwrap();
        assert!(r.scoped_to_tenant);
    }

    #[test]
    fn validate_query_blocks_cross_tenant_reference_for_non_admin() {
        let mut p = LakehousePlugin::new();
        p.register_catalog(sample_catalog("globex_warehouse", "globex"))
            .unwrap();
        // Tenant 'acme' tries to query globex_warehouse explicitly.
        let err = p
            .validate_query(
                "SELECT * FROM globex_warehouse.raw.events",
                ViewPersona::Tenant,
                "acme",
            )
            .unwrap_err();
        assert!(matches!(err, LakehouseError::CrossTenant(_)));
        // Admin is allowed.
        let ok = p
            .validate_query(
                "SELECT * FROM globex_warehouse.raw.events",
                ViewPersona::Admin,
                "acme",
            )
            .unwrap();
        assert!(!ok.scoped_to_tenant);
    }

    #[test]
    fn table_needs_compaction_threshold() {
        let mut t = sample_table("c", "raw", "t", "acme");
        t.small_files = 50;
        assert!(!t.needs_compaction());
        t.small_files = 101;
        assert!(t.needs_compaction());
    }
}
