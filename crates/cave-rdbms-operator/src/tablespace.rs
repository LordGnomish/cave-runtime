// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: cloudnative-pg/cloudnative-pg@v1.24.0 internal/controller/cluster_tablespaces.go
// Source: cloudnative-pg/cloudnative-pg@v1.24.0 api/v1/cluster_types.go (TablespaceConfiguration)
//
//! Tablespace controller — manages additional PostgreSQL tablespaces backed by
//! their own PVC. Each tablespace has an owning cluster, a name, a storage
//! request, and a mount path under `/var/lib/postgresql/tablespaces/<name>`.
//!
//! Reconciliation:
//!
//! 1. Compare desired (CRD `spec.tablespaces`) vs actual (controller state).
//! 2. New entries → reserve a mount and emit `CREATE TABLESPACE`.
//! 3. Dropped entries → emit `DROP TABLESPACE` and unmount, **unless**
//!    `tables_referencing > 0`, in which case the drop is refused.
//!
//! The SQL emission and the kubelet mount call are both injected via traits so
//! tests run hermetically.

use crate::error::{PgError, PgResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TablespaceSpec {
    /// SQL identifier; must match `[A-Za-z_][A-Za-z0-9_]*`.
    pub name: String,
    pub cluster: String,
    pub storage_bytes: u64,
    pub storage_class: String,
    /// e.g. "/var/lib/postgresql/tablespaces/idx_hot"
    pub mount_path: String,
    /// PG owner role. Defaults to `postgres`.
    pub owner: Option<String>,
    /// If true, the controller emits `ALTER TABLESPACE … SET (random_page_cost…)`.
    pub temporary: bool,
}

impl TablespaceSpec {
    pub fn default_mount_path(cluster: &str, name: &str) -> String {
        format!("/var/lib/postgresql/tablespaces/{}/{}", cluster, name)
    }

    pub fn validate(&self) -> PgResult<()> {
        if self.name.is_empty() {
            return Err(PgError::ConfigError("tablespace name empty".into()));
        }
        let valid_first = self
            .name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false);
        if !valid_first {
            return Err(PgError::ConfigError(format!(
                "tablespace {} must start with letter or underscore",
                self.name
            )));
        }
        if !self.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(PgError::ConfigError(format!(
                "tablespace {} contains invalid characters",
                self.name
            )));
        }
        if self.storage_bytes == 0 {
            return Err(PgError::ConfigError("tablespace storage must be > 0".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TablespacePhase {
    Pending,
    Mounting,
    Mounted,
    Dropping,
    Dropped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TablespaceRecord {
    pub spec: TablespaceSpec,
    pub phase: TablespacePhase,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tables_referencing: u64,
    pub last_error: Option<String>,
}

/// Emit DDL to the cluster. Injected so tests collect the statements.
pub trait SqlExecutor: Send + Sync {
    fn execute(&self, cluster: &str, sql: &str) -> Result<(), String>;
}

/// PVC + node mount management. Injected so tests collect operations.
pub trait MountManager: Send + Sync {
    fn mount(&self, cluster: &str, path: &str, storage_bytes: u64) -> Result<(), String>;
    fn unmount(&self, cluster: &str, path: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct MockSqlExecutor {
    pub statements: Mutex<Vec<(String, String)>>,
}

impl SqlExecutor for MockSqlExecutor {
    fn execute(&self, cluster: &str, sql: &str) -> Result<(), String> {
        self.statements
            .lock()
            .unwrap()
            .push((cluster.into(), sql.into()));
        Ok(())
    }
}

#[derive(Default)]
pub struct MockMountManager {
    pub mounts: Mutex<Vec<(String, String, u64)>>,
    pub unmounts: Mutex<Vec<(String, String)>>,
}

impl MountManager for MockMountManager {
    fn mount(&self, cluster: &str, path: &str, storage_bytes: u64) -> Result<(), String> {
        self.mounts
            .lock()
            .unwrap()
            .push((cluster.into(), path.into(), storage_bytes));
        Ok(())
    }
    fn unmount(&self, cluster: &str, path: &str) -> Result<(), String> {
        self.unmounts.lock().unwrap().push((cluster.into(), path.into()));
        Ok(())
    }
}

pub struct TablespaceController {
    records: Arc<RwLock<HashMap<String, TablespaceRecord>>>, // key = cluster/name
    sql: Arc<dyn SqlExecutor>,
    mount: Arc<dyn MountManager>,
}

fn key(cluster: &str, name: &str) -> String {
    format!("{}/{}", cluster, name)
}

/// SQL string for `CREATE TABLESPACE`. Quotes `name` and `location` so an
/// injected single-quote in the location is escaped (PG-compatible).
pub fn render_create_tablespace(spec: &TablespaceSpec) -> String {
    let owner = spec.owner.as_deref().unwrap_or("postgres");
    let safe_location = spec.mount_path.replace('\'', "''");
    format!(
        "CREATE TABLESPACE \"{}\" OWNER \"{}\" LOCATION '{}';",
        spec.name, owner, safe_location
    )
}

pub fn render_drop_tablespace(name: &str) -> String {
    format!("DROP TABLESPACE \"{}\";", name)
}

impl TablespaceController {
    pub fn new(sql: Arc<dyn SqlExecutor>, mount: Arc<dyn MountManager>) -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            sql,
            mount,
        }
    }

    pub fn list(&self, cluster: &str) -> Vec<TablespaceRecord> {
        self.records
            .read()
            .unwrap()
            .values()
            .filter(|r| r.spec.cluster == cluster)
            .cloned()
            .collect()
    }

    pub fn list_all(&self) -> Vec<TablespaceRecord> {
        self.records.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, cluster: &str, name: &str) -> Option<TablespaceRecord> {
        self.records.read().unwrap().get(&key(cluster, name)).cloned()
    }

    /// Create + mount a tablespace. Idempotent on duplicates.
    pub fn reconcile_create(&self, spec: TablespaceSpec) -> PgResult<TablespaceRecord> {
        spec.validate()?;
        let k = key(&spec.cluster, &spec.name);
        if let Some(existing) = self.records.read().unwrap().get(&k).cloned() {
            if existing.phase != TablespacePhase::Dropped {
                return Ok(existing);
            }
        }
        self.mount
            .mount(&spec.cluster, &spec.mount_path, spec.storage_bytes)
            .map_err(PgError::ConfigError)?;
        let sql_stmt = render_create_tablespace(&spec);
        self.sql
            .execute(&spec.cluster, &sql_stmt)
            .map_err(PgError::ConfigError)?;
        let now = Utc::now();
        let rec = TablespaceRecord {
            spec: spec.clone(),
            phase: TablespacePhase::Mounted,
            created_at: now,
            updated_at: now,
            tables_referencing: 0,
            last_error: None,
        };
        self.records.write().unwrap().insert(k, rec.clone());
        Ok(rec)
    }

    /// Record that some PG tables now sit on this tablespace. Used as the
    /// drop-safety gate.
    pub fn set_table_count(&self, cluster: &str, name: &str, n: u64) -> PgResult<()> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(&key(cluster, name)).ok_or_else(|| {
            PgError::ConfigError(format!("tablespace {}/{} not found", cluster, name))
        })?;
        rec.tables_referencing = n;
        rec.updated_at = Utc::now();
        Ok(())
    }

    /// Drop tablespace. Refuses if `tables_referencing > 0` (PG would error).
    pub fn reconcile_drop(&self, cluster: &str, name: &str) -> PgResult<()> {
        let k = key(cluster, name);
        let rec = self
            .records
            .read()
            .unwrap()
            .get(&k)
            .cloned()
            .ok_or_else(|| {
                PgError::ConfigError(format!("tablespace {}/{} not found", cluster, name))
            })?;
        if rec.tables_referencing > 0 {
            return Err(PgError::ConfigError(format!(
                "tablespace {} has {} tables referencing — drop refused",
                name, rec.tables_referencing
            )));
        }
        if rec.phase == TablespacePhase::Dropped {
            return Ok(());
        }
        let sql_stmt = render_drop_tablespace(name);
        self.sql
            .execute(cluster, &sql_stmt)
            .map_err(PgError::ConfigError)?;
        self.mount
            .unmount(cluster, &rec.spec.mount_path)
            .map_err(PgError::ConfigError)?;
        let mut records = self.records.write().unwrap();
        let r = records.get_mut(&k).unwrap();
        r.phase = TablespacePhase::Dropped;
        r.updated_at = Utc::now();
        Ok(())
    }

    /// Bulk reconcile against a desired set. Adds missing, drops removed.
    pub fn reconcile_set(&self, desired: &[TablespaceSpec]) -> PgResult<()> {
        // Group by cluster.
        let mut by_cluster: HashMap<String, Vec<&TablespaceSpec>> = HashMap::new();
        for s in desired {
            by_cluster.entry(s.cluster.clone()).or_default().push(s);
        }
        // Add missing.
        for s in desired {
            self.reconcile_create(s.clone())?;
        }
        // Drop entries not in desired.
        let existing: Vec<(String, String)> = self
            .records
            .read()
            .unwrap()
            .values()
            .filter(|r| r.phase != TablespacePhase::Dropped)
            .map(|r| (r.spec.cluster.clone(), r.spec.name.clone()))
            .collect();
        for (cluster, name) in existing {
            let want = by_cluster
                .get(&cluster)
                .map(|v| v.iter().any(|s| s.name == name))
                .unwrap_or(false);
            if !want {
                let _ = self.reconcile_drop(&cluster, &name);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl() -> (
        TablespaceController,
        Arc<MockSqlExecutor>,
        Arc<MockMountManager>,
    ) {
        let sql = Arc::new(MockSqlExecutor::default());
        let mount = Arc::new(MockMountManager::default());
        let c = TablespaceController::new(sql.clone(), mount.clone());
        (c, sql, mount)
    }

    fn spec(cluster: &str, name: &str) -> TablespaceSpec {
        TablespaceSpec {
            name: name.into(),
            cluster: cluster.into(),
            storage_bytes: 10 * 1024 * 1024 * 1024,
            storage_class: "fast-ssd".into(),
            mount_path: TablespaceSpec::default_mount_path(cluster, name),
            owner: None,
            temporary: false,
        }
    }

    #[test]
    fn validate_rejects_bad_names() {
        let mut s = spec("c1", "");
        assert!(s.validate().is_err());
        s.name = "1abc".into();
        assert!(s.validate().is_err());
        s.name = "ok; DROP TABLE x".into();
        assert!(s.validate().is_err());
        s.name = "good_name".into();
        s.storage_bytes = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn create_emits_sql_and_mount() {
        let (c, sql, mount) = ctl();
        c.reconcile_create(spec("c1", "idx_hot")).unwrap();
        let stmts = sql.statements.lock().unwrap();
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].1.starts_with("CREATE TABLESPACE \"idx_hot\""));
        assert!(stmts[0].1.contains("OWNER \"postgres\""));
        let mounts = mount.mounts.lock().unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].1, "/var/lib/postgresql/tablespaces/c1/idx_hot");
    }

    #[test]
    fn create_is_idempotent_on_duplicate() {
        let (c, sql, _) = ctl();
        c.reconcile_create(spec("c1", "idx_hot")).unwrap();
        c.reconcile_create(spec("c1", "idx_hot")).unwrap();
        c.reconcile_create(spec("c1", "idx_hot")).unwrap();
        assert_eq!(sql.statements.lock().unwrap().len(), 1);
        assert_eq!(c.list("c1").len(), 1);
    }

    #[test]
    fn drop_refused_if_tables_reference() {
        let (c, _, _) = ctl();
        c.reconcile_create(spec("c1", "idx_hot")).unwrap();
        c.set_table_count("c1", "idx_hot", 4).unwrap();
        let err = c.reconcile_drop("c1", "idx_hot").unwrap_err();
        assert!(err.to_string().contains("tables referencing"));
    }

    #[test]
    fn drop_succeeds_when_empty() {
        let (c, sql, mount) = ctl();
        c.reconcile_create(spec("c1", "idx_hot")).unwrap();
        c.reconcile_drop("c1", "idx_hot").unwrap();
        // CREATE + DROP statements
        assert_eq!(sql.statements.lock().unwrap().len(), 2);
        assert!(sql.statements.lock().unwrap()[1]
            .1
            .starts_with("DROP TABLESPACE \"idx_hot\""));
        assert_eq!(mount.unmounts.lock().unwrap().len(), 1);
        let r = c.get("c1", "idx_hot").unwrap();
        assert_eq!(r.phase, TablespacePhase::Dropped);
    }

    #[test]
    fn drop_unknown_errors() {
        let (c, _, _) = ctl();
        let err = c.reconcile_drop("c1", "ghost").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn list_filters_by_cluster() {
        let (c, _, _) = ctl();
        c.reconcile_create(spec("c1", "a")).unwrap();
        c.reconcile_create(spec("c1", "b")).unwrap();
        c.reconcile_create(spec("c2", "a")).unwrap();
        assert_eq!(c.list("c1").len(), 2);
        assert_eq!(c.list("c2").len(), 1);
        assert_eq!(c.list_all().len(), 3);
    }

    #[test]
    fn reconcile_set_adds_and_drops() {
        let (c, _, _) = ctl();
        c.reconcile_create(spec("c1", "a")).unwrap();
        c.reconcile_create(spec("c1", "b")).unwrap();
        // Desired: keep `a`, drop `b`, add `c`.
        c.reconcile_set(&[spec("c1", "a"), spec("c1", "c")]).unwrap();
        assert_eq!(c.get("c1", "a").unwrap().phase, TablespacePhase::Mounted);
        assert_eq!(c.get("c1", "b").unwrap().phase, TablespacePhase::Dropped);
        assert_eq!(c.get("c1", "c").unwrap().phase, TablespacePhase::Mounted);
    }

    #[test]
    fn render_create_quotes_identifiers() {
        let mut s = spec("c1", "idx_hot");
        s.owner = Some("warehouse".into());
        let sql = render_create_tablespace(&s);
        assert!(sql.contains("\"idx_hot\""));
        assert!(sql.contains("\"warehouse\""));
        assert!(sql.ends_with(";"));
    }

    #[test]
    fn render_drop_quotes_name() {
        let sql = render_drop_tablespace("idx_hot");
        assert_eq!(sql, "DROP TABLESPACE \"idx_hot\";");
    }

    #[test]
    fn default_mount_path_contains_cluster_and_name() {
        let p = TablespaceSpec::default_mount_path("warehouse", "cold");
        assert!(p.contains("warehouse"));
        assert!(p.contains("cold"));
        assert!(p.starts_with("/var/lib/postgresql/tablespaces/"));
    }

    #[test]
    fn set_table_count_updates_timestamp() {
        let (c, _, _) = ctl();
        c.reconcile_create(spec("c1", "a")).unwrap();
        let before = c.get("c1", "a").unwrap().updated_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        c.set_table_count("c1", "a", 3).unwrap();
        let after = c.get("c1", "a").unwrap().updated_at;
        assert!(after > before);
        assert_eq!(c.get("c1", "a").unwrap().tables_referencing, 3);
    }

    #[test]
    fn drop_after_clearing_tables_succeeds() {
        let (c, _, _) = ctl();
        c.reconcile_create(spec("c1", "a")).unwrap();
        c.set_table_count("c1", "a", 5).unwrap();
        assert!(c.reconcile_drop("c1", "a").is_err());
        c.set_table_count("c1", "a", 0).unwrap();
        assert!(c.reconcile_drop("c1", "a").is_ok());
    }
}
