//! Iceberg REST catalog state — namespaces + tables + commits.
//!
//! Mirrors `apache/iceberg/open-api/rest-catalog-open-api.yaml` data model:
//!
//! * Namespaces are arbitrary dot-joined string paths (e.g. `acme.warehouse`).
//! * Each table belongs to one namespace and has a `metadata_location`.
//! * Commits use **optimistic concurrency**: client supplies the
//!   `expected_metadata_location`; server rejects the commit if the current
//!   stored location differs.

use std::collections::BTreeMap;

use parking_lot::RwLock;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CatalogError {
    #[error("namespace not found: {0}")]
    NamespaceNotFound(String),
    #[error("namespace already exists: {0}")]
    NamespaceExists(String),
    #[error("namespace not empty: {0}")]
    NamespaceNotEmpty(String),
    #[error("table not found: {0}.{1}")]
    TableNotFound(String, String),
    #[error("table already exists: {0}.{1}")]
    TableExists(String, String),
    #[error("commit conflict: expected {expected}, found {found}")]
    CommitConflict { expected: String, found: String },
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
}

pub type CatalogResult<T> = Result<T, CatalogError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEntry {
    pub namespace: String,
    pub name: String,
    pub metadata_location: String,
    pub current_snapshot_id: Option<i64>,
    pub last_updated_ms: i64,
    pub properties: BTreeMap<String, String>,
}

impl TableEntry {
    pub fn full_name(&self) -> String {
        if self.namespace.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.namespace, self.name)
        }
    }
}

#[derive(Debug, Default)]
pub struct RestCatalog {
    /// namespace → properties
    namespaces: RwLock<BTreeMap<String, BTreeMap<String, String>>>,
    /// (namespace, name) → entry
    tables: RwLock<BTreeMap<(String, String), TableEntry>>,
}

impl RestCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_namespace(
        &self,
        namespace: &str,
        properties: BTreeMap<String, String>,
    ) -> CatalogResult<()> {
        validate_namespace(namespace)?;
        let mut ns = self.namespaces.write();
        if ns.contains_key(namespace) {
            return Err(CatalogError::NamespaceExists(namespace.into()));
        }
        ns.insert(namespace.into(), properties);
        Ok(())
    }

    pub fn drop_namespace(&self, namespace: &str) -> CatalogResult<()> {
        let tables = self.tables.read();
        let has_tables = tables.keys().any(|(ns, _)| ns == namespace);
        if has_tables {
            return Err(CatalogError::NamespaceNotEmpty(namespace.into()));
        }
        drop(tables);
        let mut ns = self.namespaces.write();
        ns.remove(namespace)
            .ok_or_else(|| CatalogError::NamespaceNotFound(namespace.into()))?;
        Ok(())
    }

    pub fn list_namespaces(&self, parent: Option<&str>) -> Vec<String> {
        let ns = self.namespaces.read();
        ns.keys()
            .filter(|k| match parent {
                None => !k.contains('.'),
                Some(p) => {
                    k.starts_with(&format!("{}.", p))
                        && !k[p.len() + 1..].contains('.')
                }
            })
            .cloned()
            .collect()
    }

    pub fn namespace_properties(&self, namespace: &str) -> CatalogResult<BTreeMap<String, String>> {
        let ns = self.namespaces.read();
        ns.get(namespace)
            .cloned()
            .ok_or_else(|| CatalogError::NamespaceNotFound(namespace.into()))
    }

    pub fn update_namespace_properties(
        &self,
        namespace: &str,
        updates: BTreeMap<String, String>,
        removals: Vec<String>,
    ) -> CatalogResult<()> {
        let mut ns = self.namespaces.write();
        let props = ns
            .get_mut(namespace)
            .ok_or_else(|| CatalogError::NamespaceNotFound(namespace.into()))?;
        for (k, v) in updates {
            props.insert(k, v);
        }
        for k in removals {
            props.remove(&k);
        }
        Ok(())
    }

    pub fn create_table(
        &self,
        namespace: &str,
        name: &str,
        metadata_location: &str,
        properties: BTreeMap<String, String>,
        now_ms: i64,
    ) -> CatalogResult<TableEntry> {
        validate_identifier(name)?;
        let ns = self.namespaces.read();
        if !ns.contains_key(namespace) {
            return Err(CatalogError::NamespaceNotFound(namespace.into()));
        }
        drop(ns);
        let mut tables = self.tables.write();
        let key = (namespace.to_string(), name.to_string());
        if tables.contains_key(&key) {
            return Err(CatalogError::TableExists(namespace.into(), name.into()));
        }
        let entry = TableEntry {
            namespace: namespace.into(),
            name: name.into(),
            metadata_location: metadata_location.into(),
            current_snapshot_id: None,
            last_updated_ms: now_ms,
            properties,
        };
        tables.insert(key, entry.clone());
        Ok(entry)
    }

    pub fn drop_table(&self, namespace: &str, name: &str) -> CatalogResult<()> {
        let mut tables = self.tables.write();
        tables
            .remove(&(namespace.to_string(), name.to_string()))
            .ok_or_else(|| CatalogError::TableNotFound(namespace.into(), name.into()))?;
        Ok(())
    }

    pub fn rename_table(
        &self,
        from_ns: &str,
        from_name: &str,
        to_ns: &str,
        to_name: &str,
    ) -> CatalogResult<()> {
        validate_identifier(to_name)?;
        let ns = self.namespaces.read();
        if !ns.contains_key(to_ns) {
            return Err(CatalogError::NamespaceNotFound(to_ns.into()));
        }
        drop(ns);
        let mut tables = self.tables.write();
        if tables.contains_key(&(to_ns.to_string(), to_name.to_string())) {
            return Err(CatalogError::TableExists(to_ns.into(), to_name.into()));
        }
        let mut entry = tables
            .remove(&(from_ns.to_string(), from_name.to_string()))
            .ok_or_else(|| CatalogError::TableNotFound(from_ns.into(), from_name.into()))?;
        entry.namespace = to_ns.into();
        entry.name = to_name.into();
        tables.insert((to_ns.into(), to_name.into()), entry);
        Ok(())
    }

    pub fn load_table(&self, namespace: &str, name: &str) -> CatalogResult<TableEntry> {
        let tables = self.tables.read();
        tables
            .get(&(namespace.to_string(), name.to_string()))
            .cloned()
            .ok_or_else(|| CatalogError::TableNotFound(namespace.into(), name.into()))
    }

    pub fn list_tables(&self, namespace: &str) -> CatalogResult<Vec<String>> {
        let ns = self.namespaces.read();
        if !ns.contains_key(namespace) {
            return Err(CatalogError::NamespaceNotFound(namespace.into()));
        }
        drop(ns);
        let tables = self.tables.read();
        Ok(tables
            .keys()
            .filter(|(n, _)| n == namespace)
            .map(|(_, name)| name.clone())
            .collect())
    }

    /// Optimistic-concurrency commit: caller specifies the metadata location
    /// they read; server rejects if the current location has moved.
    pub fn commit(
        &self,
        namespace: &str,
        name: &str,
        expected_metadata_location: &str,
        new_metadata_location: &str,
        new_snapshot_id: Option<i64>,
        now_ms: i64,
    ) -> CatalogResult<TableEntry> {
        let mut tables = self.tables.write();
        let entry = tables
            .get_mut(&(namespace.to_string(), name.to_string()))
            .ok_or_else(|| CatalogError::TableNotFound(namespace.into(), name.into()))?;
        if entry.metadata_location != expected_metadata_location {
            return Err(CatalogError::CommitConflict {
                expected: expected_metadata_location.into(),
                found: entry.metadata_location.clone(),
            });
        }
        entry.metadata_location = new_metadata_location.into();
        entry.current_snapshot_id = new_snapshot_id;
        entry.last_updated_ms = now_ms;
        Ok(entry.clone())
    }

    /// Upstream-spec commit:
    ///   POST /v1/{prefix}/namespaces/{ns}/tables/{tbl}
    ///   body = CommitTableRequest{ requirements[], updates[] }
    ///
    /// We honour the requirement subset that maps to in-memory state we
    /// actually track:
    ///   * `assert-table-uuid`           → no-op (we don't model UUID yet; we
    ///                                     accept any UUID rather than fail).
    ///   * `assert-current-snapshot-id`  → must match `entry.current_snapshot_id`.
    ///   * `assert-create`               → table must NOT already exist.
    /// We honour the update subset:
    ///   * `set-snapshot` / `add-snapshot` → updates `current_snapshot_id`.
    ///   * `set-current-snapshot`          → updates `current_snapshot_id`.
    ///   * `set-location`                  → ignored (we don't track location
    ///                                       separately from `metadata_location`).
    ///
    /// `new_metadata_location` is supplied out-of-band (the catalog itself
    /// doesn't synthesise metadata.json files; the client writes them and
    /// tells us the new path). For multi-table transactions the caller wraps
    /// this method per table.
    pub fn commit_with_updates(
        &self,
        namespace: &str,
        name: &str,
        new_metadata_location: &str,
        requirements: &[CommitRequirement],
        updates: &[CommitUpdate],
        now_ms: i64,
    ) -> CatalogResult<TableEntry> {
        let mut tables = self.tables.write();
        let key = (namespace.to_string(), name.to_string());

        // assert-create: table must not already exist.
        let asserts_create = requirements
            .iter()
            .any(|r| matches!(r, CommitRequirement::AssertCreate));
        if asserts_create {
            if tables.contains_key(&key) {
                return Err(CatalogError::TableExists(namespace.into(), name.into()));
            }
            // Synthesize a fresh entry; namespace must exist.
            drop(tables);
            let ns = self.namespaces.read();
            if !ns.contains_key(namespace) {
                return Err(CatalogError::NamespaceNotFound(namespace.into()));
            }
            drop(ns);
            let mut tables = self.tables.write();
            let mut entry = TableEntry {
                namespace: namespace.into(),
                name: name.into(),
                metadata_location: new_metadata_location.into(),
                current_snapshot_id: None,
                last_updated_ms: now_ms,
                properties: BTreeMap::new(),
            };
            apply_updates(&mut entry, updates);
            tables.insert(key, entry.clone());
            return Ok(entry);
        }

        let entry = tables
            .get_mut(&key)
            .ok_or_else(|| CatalogError::TableNotFound(namespace.into(), name.into()))?;

        for req in requirements {
            match req {
                CommitRequirement::AssertCreate => {} // handled above
                CommitRequirement::AssertTableUuid { .. } => {
                    // We don't track UUIDs; accept by convention.
                }
                CommitRequirement::AssertCurrentSnapshotId { snapshot_id } => {
                    if entry.current_snapshot_id != *snapshot_id {
                        return Err(CatalogError::CommitConflict {
                            expected: format!("snapshot-id={:?}", snapshot_id),
                            found: format!("snapshot-id={:?}", entry.current_snapshot_id),
                        });
                    }
                }
                CommitRequirement::AssertRefSnapshotId { snapshot_id, .. } => {
                    if entry.current_snapshot_id != *snapshot_id {
                        return Err(CatalogError::CommitConflict {
                            expected: format!("ref-snapshot-id={:?}", snapshot_id),
                            found: format!("snapshot-id={:?}", entry.current_snapshot_id),
                        });
                    }
                }
                CommitRequirement::Other { .. } => {
                    // Unrecognised requirement types: accept (forward-compat).
                }
            }
        }

        entry.metadata_location = new_metadata_location.into();
        entry.last_updated_ms = now_ms;
        apply_updates(entry, updates);
        Ok(entry.clone())
    }

    pub fn table_count(&self) -> usize {
        self.tables.read().len()
    }

    pub fn namespace_count(&self) -> usize {
        self.namespaces.read().len()
    }

    /// Register an existing metadata.json under a (namespace, name) without
    /// rewriting it — mirrors `Catalog.registerTable` and the
    /// `POST /v1/{prefix}/namespaces/{ns}/register` endpoint.
    ///
    /// `overwrite=false` rejects collisions; `overwrite=true` replaces any
    /// existing entry with the same (namespace, name).
    pub fn register_table(
        &self,
        namespace: &str,
        name: &str,
        metadata_location: &str,
        overwrite: bool,
        now_ms: i64,
    ) -> CatalogResult<TableEntry> {
        validate_identifier(name)?;
        let ns = self.namespaces.read();
        if !ns.contains_key(namespace) {
            return Err(CatalogError::NamespaceNotFound(namespace.into()));
        }
        drop(ns);
        let mut tables = self.tables.write();
        let key = (namespace.to_string(), name.to_string());
        if !overwrite && tables.contains_key(&key) {
            return Err(CatalogError::TableExists(namespace.into(), name.into()));
        }
        let entry = TableEntry {
            namespace: namespace.into(),
            name: name.into(),
            metadata_location: metadata_location.into(),
            current_snapshot_id: None,
            last_updated_ms: now_ms,
            properties: BTreeMap::new(),
        };
        tables.insert(key, entry.clone());
        Ok(entry)
    }
}

// ── CommitTable spec types (subset) ─────────────────────────────────────────
//
// Mirrors `apache/iceberg/open-api/rest-catalog-open-api.yaml`:
//   * TableRequirement (discriminated union on `type`)
//   * TableUpdate      (discriminated union on `action`)
//
// We model the variants the in-memory catalog can actually evaluate; unknown
// types are tolerated (Other) so the wire payload deserialises even when a
// client sends a richer body than we evaluate.

/// Subset of upstream `TableRequirement`. The discriminator on the wire is
/// `"type": "assert-..."`; here we map only the requirements that touch
/// state we actually track (snapshot id, table existence).
#[derive(Debug, Clone, PartialEq)]
pub enum CommitRequirement {
    AssertCreate,
    AssertTableUuid {
        uuid: String,
    },
    AssertCurrentSnapshotId {
        snapshot_id: Option<i64>,
    },
    AssertRefSnapshotId {
        ref_name: String,
        snapshot_id: Option<i64>,
    },
    /// Catch-all so unfamiliar requirement types deserialize cleanly.
    Other {
        type_tag: String,
    },
}

/// Subset of upstream `TableUpdate`. We honour only those that mutate the
/// fields TableEntry actually carries (current_snapshot_id, properties).
#[derive(Debug, Clone, PartialEq)]
pub enum CommitUpdate {
    /// `set-snapshot-ref` (main branch) or `add-snapshot` set the current id.
    SetCurrentSnapshot { snapshot_id: i64 },
    /// `set-properties` merges into `entry.properties`.
    SetProperties { updates: BTreeMap<String, String> },
    /// `remove-properties` removes keys.
    RemoveProperties { removals: Vec<String> },
    /// Catch-all so unfamiliar action types deserialize cleanly.
    Other { action: String },
}

fn apply_updates(entry: &mut TableEntry, updates: &[CommitUpdate]) {
    for u in updates {
        match u {
            CommitUpdate::SetCurrentSnapshot { snapshot_id } => {
                entry.current_snapshot_id = Some(*snapshot_id);
            }
            CommitUpdate::SetProperties { updates } => {
                for (k, v) in updates {
                    entry.properties.insert(k.clone(), v.clone());
                }
            }
            CommitUpdate::RemoveProperties { removals } => {
                for k in removals {
                    entry.properties.remove(k);
                }
            }
            CommitUpdate::Other { .. } => {}
        }
    }
}

pub fn validate_namespace(name: &str) -> CatalogResult<()> {
    if name.is_empty() {
        return Err(CatalogError::InvalidIdentifier("empty namespace".into()));
    }
    for part in name.split('.') {
        validate_identifier(part)?;
    }
    Ok(())
}

pub fn validate_identifier(name: &str) -> CatalogResult<()> {
    if name.is_empty() {
        return Err(CatalogError::InvalidIdentifier("empty".into()));
    }
    if name.len() > 255 {
        return Err(CatalogError::InvalidIdentifier(format!(
            "len {} > 255",
            name.len()
        )));
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(CatalogError::InvalidIdentifier(
            "must start with letter/underscore".into(),
        ));
    }
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' {
            return Err(CatalogError::InvalidIdentifier(format!(
                "invalid char '{}'",
                ch
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_props() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn create_then_list_namespace() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let l = c.list_namespaces(None);
        assert_eq!(l, vec!["acme"]);
    }

    #[test]
    fn create_namespace_duplicate_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let err = c.create_namespace("acme", empty_props()).unwrap_err();
        assert_eq!(err, CatalogError::NamespaceExists("acme".into()));
    }

    #[test]
    fn drop_namespace_when_empty_succeeds() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.drop_namespace("acme").unwrap();
        assert_eq!(c.namespace_count(), 0);
    }

    #[test]
    fn drop_namespace_with_tables_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "m1.json", empty_props(), 0).unwrap();
        let err = c.drop_namespace("acme").unwrap_err();
        assert_eq!(err, CatalogError::NamespaceNotEmpty("acme".into()));
    }

    #[test]
    fn list_namespaces_filters_by_parent() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_namespace("acme.warehouse", empty_props()).unwrap();
        c.create_namespace("acme.warehouse.gold", empty_props()).unwrap();
        let top = c.list_namespaces(None);
        assert_eq!(top, vec!["acme"]);
        let under_acme = c.list_namespaces(Some("acme"));
        assert_eq!(under_acme, vec!["acme.warehouse"]);
    }

    #[test]
    fn namespace_properties_round_trip() {
        let c = RestCatalog::new();
        let mut p = empty_props();
        p.insert("owner".into(), "alice".into());
        c.create_namespace("acme", p.clone()).unwrap();
        let got = c.namespace_properties("acme").unwrap();
        assert_eq!(got, p);
    }

    #[test]
    fn update_namespace_properties_merges() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let mut updates = empty_props();
        updates.insert("k1".into(), "v1".into());
        c.update_namespace_properties("acme", updates, vec![]).unwrap();
        let got = c.namespace_properties("acme").unwrap();
        assert_eq!(got.get("k1").unwrap(), "v1");
    }

    #[test]
    fn update_namespace_removes_keys() {
        let c = RestCatalog::new();
        let mut p = empty_props();
        p.insert("k1".into(), "v1".into());
        p.insert("k2".into(), "v2".into());
        c.create_namespace("acme", p).unwrap();
        c.update_namespace_properties("acme", empty_props(), vec!["k1".into()])
            .unwrap();
        let got = c.namespace_properties("acme").unwrap();
        assert!(!got.contains_key("k1"));
        assert!(got.contains_key("k2"));
    }

    #[test]
    fn create_table_in_unknown_namespace_errors() {
        let c = RestCatalog::new();
        let err = c.create_table("ghost", "t", "m1", empty_props(), 0).unwrap_err();
        assert!(matches!(err, CatalogError::NamespaceNotFound(_)));
    }

    #[test]
    fn create_then_load_table() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "users", "v1.metadata.json", empty_props(), 100)
            .unwrap();
        let entry = c.load_table("acme", "users").unwrap();
        assert_eq!(entry.metadata_location, "v1.metadata.json");
        assert_eq!(entry.last_updated_ms, 100);
    }

    #[test]
    fn create_table_duplicate_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "m1", empty_props(), 0).unwrap();
        let err = c.create_table("acme", "t", "m2", empty_props(), 0).unwrap_err();
        assert!(matches!(err, CatalogError::TableExists(_, _)));
    }

    #[test]
    fn drop_table_removes_entry() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "m1", empty_props(), 0).unwrap();
        c.drop_table("acme", "t").unwrap();
        assert!(c.load_table("acme", "t").is_err());
    }

    #[test]
    fn rename_table_changes_key() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "old", "m1", empty_props(), 0).unwrap();
        c.rename_table("acme", "old", "acme", "new").unwrap();
        assert!(c.load_table("acme", "new").is_ok());
        assert!(c.load_table("acme", "old").is_err());
    }

    #[test]
    fn rename_to_existing_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "a", "m1", empty_props(), 0).unwrap();
        c.create_table("acme", "b", "m2", empty_props(), 0).unwrap();
        let err = c.rename_table("acme", "a", "acme", "b").unwrap_err();
        assert!(matches!(err, CatalogError::TableExists(_, _)));
    }

    #[test]
    fn list_tables_in_namespace() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "a", "m1", empty_props(), 0).unwrap();
        c.create_table("acme", "b", "m2", empty_props(), 0).unwrap();
        let mut t = c.list_tables("acme").unwrap();
        t.sort();
        assert_eq!(t, vec!["a", "b"]);
    }

    #[test]
    fn commit_with_matching_expected_succeeds() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1.json", empty_props(), 0).unwrap();
        let entry = c
            .commit("acme", "t", "v1.json", "v2.json", Some(42), 100)
            .unwrap();
        assert_eq!(entry.metadata_location, "v2.json");
        assert_eq!(entry.current_snapshot_id, Some(42));
    }

    #[test]
    fn commit_with_stale_expected_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1.json", empty_props(), 0).unwrap();
        // First writer commits.
        c.commit("acme", "t", "v1.json", "v2.json", None, 0).unwrap();
        // Second writer's read was at v1, now stale.
        let err = c.commit("acme", "t", "v1.json", "v3.json", None, 0).unwrap_err();
        match err {
            CatalogError::CommitConflict { expected, found } => {
                assert_eq!(expected, "v1.json");
                assert_eq!(found, "v2.json");
            }
            other => panic!("expected CommitConflict, got {:?}", other),
        }
    }

    #[test]
    fn validate_identifier_accepts_underscore_start() {
        assert!(validate_identifier("_x").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_digit_start() {
        assert!(validate_identifier("1x").is_err());
    }

    #[test]
    fn validate_namespace_accepts_dot_path() {
        assert!(validate_namespace("acme.warehouse.gold").is_ok());
    }

    #[test]
    fn validate_namespace_rejects_empty_segment() {
        assert!(validate_namespace("acme..gold").is_err());
    }

    #[test]
    fn create_table_with_invalid_name_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let err = c.create_table("acme", "1bad", "m", empty_props(), 0).unwrap_err();
        assert!(matches!(err, CatalogError::InvalidIdentifier(_)));
    }

    #[test]
    fn full_name_qualifies() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let e = c.create_table("acme", "t", "m", empty_props(), 0).unwrap();
        assert_eq!(e.full_name(), "acme.t");
    }

    #[test]
    fn commit_with_updates_assert_current_snapshot_matches() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1.json", empty_props(), 0).unwrap();
        // Put the table at snapshot 7.
        c.commit("acme", "t", "v1.json", "v2.json", Some(7), 0).unwrap();
        let entry = c
            .commit_with_updates(
                "acme",
                "t",
                "v3.json",
                &[CommitRequirement::AssertCurrentSnapshotId { snapshot_id: Some(7) }],
                &[CommitUpdate::SetCurrentSnapshot { snapshot_id: 8 }],
                100,
            )
            .unwrap();
        assert_eq!(entry.metadata_location, "v3.json");
        assert_eq!(entry.current_snapshot_id, Some(8));
    }

    #[test]
    fn commit_with_updates_assert_current_snapshot_mismatch_errors() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1.json", empty_props(), 0).unwrap();
        let err = c
            .commit_with_updates(
                "acme",
                "t",
                "v2.json",
                &[CommitRequirement::AssertCurrentSnapshotId { snapshot_id: Some(99) }],
                &[],
                0,
            )
            .unwrap_err();
        assert!(matches!(err, CatalogError::CommitConflict { .. }));
    }

    #[test]
    fn commit_with_updates_assert_create_creates_new_table() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let entry = c
            .commit_with_updates(
                "acme",
                "fresh",
                "v1.json",
                &[CommitRequirement::AssertCreate],
                &[CommitUpdate::SetCurrentSnapshot { snapshot_id: 1 }],
                0,
            )
            .unwrap();
        assert_eq!(entry.metadata_location, "v1.json");
        assert_eq!(entry.current_snapshot_id, Some(1));
    }

    #[test]
    fn commit_with_updates_assert_create_rejects_existing() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1", empty_props(), 0).unwrap();
        let err = c
            .commit_with_updates(
                "acme",
                "t",
                "v2",
                &[CommitRequirement::AssertCreate],
                &[],
                0,
            )
            .unwrap_err();
        assert!(matches!(err, CatalogError::TableExists(_, _)));
    }

    #[test]
    fn commit_with_updates_set_properties_merges() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1", empty_props(), 0).unwrap();
        let mut props = BTreeMap::new();
        props.insert("owner".into(), "alice".into());
        let entry = c
            .commit_with_updates(
                "acme",
                "t",
                "v2",
                &[],
                &[CommitUpdate::SetProperties { updates: props }],
                0,
            )
            .unwrap();
        assert_eq!(entry.properties.get("owner").unwrap(), "alice");
    }

    #[test]
    fn register_table_creates_entry() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        let entry = c
            .register_table("acme", "imported", "s3://bucket/v1.metadata.json", false, 0)
            .unwrap();
        assert_eq!(entry.metadata_location, "s3://bucket/v1.metadata.json");
        assert!(c.load_table("acme", "imported").is_ok());
    }

    #[test]
    fn register_table_rejects_collision_without_overwrite() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1", empty_props(), 0).unwrap();
        let err = c.register_table("acme", "t", "v2", false, 0).unwrap_err();
        assert!(matches!(err, CatalogError::TableExists(_, _)));
    }

    #[test]
    fn register_table_overwrite_true_replaces() {
        let c = RestCatalog::new();
        c.create_namespace("acme", empty_props()).unwrap();
        c.create_table("acme", "t", "v1", empty_props(), 0).unwrap();
        let entry = c
            .register_table("acme", "t", "v9", true, 0)
            .unwrap();
        assert_eq!(entry.metadata_location, "v9");
    }

    #[test]
    fn register_table_unknown_namespace_errors() {
        let c = RestCatalog::new();
        let err = c
            .register_table("ghost", "t", "v1", false, 0)
            .unwrap_err();
        assert!(matches!(err, CatalogError::NamespaceNotFound(_)));
    }
}
