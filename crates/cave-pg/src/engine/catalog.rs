//! In-memory schema catalog — namespaces, tables, columns, indexes.
//!
//! Mirrors the role of `pg_class` + `pg_attribute` + `pg_namespace` from a
//! live PostgreSQL cluster, kept in process memory.

use std::collections::BTreeMap;

use super::value::ScalarType;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CatalogError {
    #[error("table already exists: {0}")]
    TableExists(String),
    #[error("table not found: {0}")]
    TableNotFound(String),
    #[error("column not found: {0}")]
    ColumnNotFound(String),
    #[error("invalid column type: {0}")]
    InvalidColumnType(String),
    #[error("schema not found: {0}")]
    SchemaNotFound(String),
}

pub type CatalogResult<T> = Result<T, CatalogError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: String,
    pub ty: ScalarType,
    pub nullable: bool,
    pub primary_key: bool,
    pub default_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub schema: String,
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub oid: u32,
}

impl TableSchema {
    pub fn qualified_name(&self) -> String {
        if self.schema == "public" {
            self.name.clone()
        } else {
            format!("{}.{}", self.schema, self.name)
        }
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    pub fn column(&self, name: &str) -> Option<&ColumnDef> {
        self.columns.iter().find(|c| c.name == name)
    }
}

#[derive(Debug, Default)]
pub struct Catalog {
    /// schema → table name → schema record
    tables: BTreeMap<String, BTreeMap<String, TableSchema>>,
    next_oid: u32,
}

impl Catalog {
    pub fn new() -> Self {
        let mut c = Self {
            tables: BTreeMap::new(),
            next_oid: 16384,
        };
        c.tables.insert("public".into(), BTreeMap::new());
        c.tables.insert("pg_catalog".into(), BTreeMap::new());
        c
    }

    pub fn create_table(
        &mut self,
        schema: &str,
        name: &str,
        columns: Vec<ColumnDef>,
    ) -> CatalogResult<u32> {
        let bucket = self
            .tables
            .get_mut(schema)
            .ok_or_else(|| CatalogError::SchemaNotFound(schema.into()))?;
        if bucket.contains_key(name) {
            return Err(CatalogError::TableExists(name.into()));
        }
        let oid = self.next_oid;
        self.next_oid += 1;
        bucket.insert(
            name.into(),
            TableSchema {
                schema: schema.into(),
                name: name.into(),
                columns,
                oid,
            },
        );
        Ok(oid)
    }

    pub fn drop_table(&mut self, schema: &str, name: &str) -> CatalogResult<()> {
        let bucket = self
            .tables
            .get_mut(schema)
            .ok_or_else(|| CatalogError::SchemaNotFound(schema.into()))?;
        bucket
            .remove(name)
            .ok_or_else(|| CatalogError::TableNotFound(name.into()))?;
        Ok(())
    }

    pub fn lookup(&self, schema: &str, name: &str) -> CatalogResult<&TableSchema> {
        self.tables
            .get(schema)
            .ok_or_else(|| CatalogError::SchemaNotFound(schema.into()))?
            .get(name)
            .ok_or_else(|| CatalogError::TableNotFound(name.into()))
    }

    /// Lookup unqualified — search public first, then pg_catalog.
    pub fn resolve(&self, name: &str) -> CatalogResult<&TableSchema> {
        if let Ok(t) = self.lookup("public", name) {
            return Ok(t);
        }
        self.lookup("pg_catalog", name)
    }

    pub fn list_tables(&self, schema: &str) -> CatalogResult<Vec<&TableSchema>> {
        Ok(self
            .tables
            .get(schema)
            .ok_or_else(|| CatalogError::SchemaNotFound(schema.into()))?
            .values()
            .collect())
    }

    pub fn create_schema(&mut self, name: &str) {
        self.tables.entry(name.into()).or_default();
    }

    pub fn schemas(&self) -> Vec<&str> {
        self.tables.keys().map(String::as_str).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `pg_database` / `pg_authid` shim — minimal in-memory mirrors of the two
// global cluster catalogs.  These are not meant to drive query planning;
// they exist so that introspection tools (libpq, psql `\l`, `\du`,
// CloudNativePG's bootstrap scripts) can read the cluster's databases and
// roles via the standard catalog names.
// ─────────────────────────────────────────────────────────────────────────────

/// Row in `pg_database` (subset of `src/include/catalog/pg_database.h`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgDatabaseRow {
    pub oid: u32,
    pub datname: String,
    pub datdba: u32,
    pub encoding: i32,
    pub datcollate: String,
    pub datctype: String,
    pub datistemplate: bool,
    pub datallowconn: bool,
}

/// Row in `pg_authid` (subset of `src/include/catalog/pg_authid.h`).
///
/// `rolpassword` is intentionally `Option<String>` and never serialised in
/// plaintext when present — callers should hash before storing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgAuthidRow {
    pub oid: u32,
    pub rolname: String,
    pub rolsuper: bool,
    pub rolinherit: bool,
    pub rolcreaterole: bool,
    pub rolcreatedb: bool,
    pub rolcanlogin: bool,
    pub rolreplication: bool,
    pub rolbypassrls: bool,
    pub rolconnlimit: i32,
    pub rolpassword: Option<String>,
}

/// In-memory store for `pg_database` + `pg_authid`.  Backs the SHOW commands
/// and the `cave-portal` PG read paths that ask "what databases / roles
/// exist on this cluster".
#[derive(Debug, Default)]
pub struct GlobalCatalog {
    databases: BTreeMap<String, PgDatabaseRow>,
    roles: BTreeMap<String, PgAuthidRow>,
    next_oid: u32,
}

impl GlobalCatalog {
    /// Fresh catalog seeded with the canonical defaults a real cluster has
    /// after `initdb`: the `template0`, `template1`, and `postgres` databases
    /// owned by the bootstrap superuser, plus the `postgres` superuser role.
    pub fn with_defaults() -> Self {
        let mut g = Self {
            databases: BTreeMap::new(),
            roles: BTreeMap::new(),
            next_oid: 16384,
        };
        // Bootstrap superuser — fixed OID 10 mirrors upstream.
        g.roles.insert(
            "postgres".into(),
            PgAuthidRow {
                oid: 10,
                rolname: "postgres".into(),
                rolsuper: true,
                rolinherit: true,
                rolcreaterole: true,
                rolcreatedb: true,
                rolcanlogin: true,
                rolreplication: true,
                rolbypassrls: true,
                rolconnlimit: -1,
                rolpassword: None,
            },
        );
        for (oid, name, allowconn, istemplate) in [
            (1u32, "template1", true, true),
            (4u32, "template0", false, true),
            (5u32, "postgres", true, false),
        ] {
            g.databases.insert(
                name.into(),
                PgDatabaseRow {
                    oid,
                    datname: name.into(),
                    datdba: 10,
                    encoding: 6, // PG_UTF8
                    datcollate: "C".into(),
                    datctype: "C".into(),
                    datistemplate: istemplate,
                    datallowconn: allowconn,
                },
            );
        }
        g
    }

    fn alloc_oid(&mut self) -> u32 {
        let oid = self.next_oid;
        self.next_oid += 1;
        oid
    }

    pub fn create_database(
        &mut self,
        name: &str,
        owner_oid: u32,
    ) -> CatalogResult<u32> {
        if self.databases.contains_key(name) {
            return Err(CatalogError::TableExists(name.into()));
        }
        let oid = self.alloc_oid();
        self.databases.insert(
            name.into(),
            PgDatabaseRow {
                oid,
                datname: name.into(),
                datdba: owner_oid,
                encoding: 6,
                datcollate: "C".into(),
                datctype: "C".into(),
                datistemplate: false,
                datallowconn: true,
            },
        );
        Ok(oid)
    }

    pub fn drop_database(&mut self, name: &str) -> CatalogResult<()> {
        self.databases
            .remove(name)
            .ok_or_else(|| CatalogError::TableNotFound(name.into()))?;
        Ok(())
    }

    pub fn database(&self, name: &str) -> Option<&PgDatabaseRow> {
        self.databases.get(name)
    }

    pub fn list_databases(&self) -> Vec<&PgDatabaseRow> {
        self.databases.values().collect()
    }

    pub fn create_role(&mut self, name: &str, can_login: bool) -> CatalogResult<u32> {
        if self.roles.contains_key(name) {
            return Err(CatalogError::TableExists(name.into()));
        }
        let oid = self.alloc_oid();
        self.roles.insert(
            name.into(),
            PgAuthidRow {
                oid,
                rolname: name.into(),
                rolsuper: false,
                rolinherit: true,
                rolcreaterole: false,
                rolcreatedb: false,
                rolcanlogin: can_login,
                rolreplication: false,
                rolbypassrls: false,
                rolconnlimit: -1,
                rolpassword: None,
            },
        );
        Ok(oid)
    }

    pub fn drop_role(&mut self, name: &str) -> CatalogResult<()> {
        self.roles
            .remove(name)
            .ok_or_else(|| CatalogError::TableNotFound(name.into()))?;
        Ok(())
    }

    pub fn role(&self, name: &str) -> Option<&PgAuthidRow> {
        self.roles.get(name)
    }

    pub fn list_roles(&self) -> Vec<&PgAuthidRow> {
        self.roles.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: "id".into(),
                ty: ScalarType::Int4,
                nullable: false,
                primary_key: true,
                default_text: None,
            },
            ColumnDef {
                name: "name".into(),
                ty: ScalarType::Text,
                nullable: true,
                primary_key: false,
                default_text: None,
            },
        ]
    }

    #[test]
    fn create_then_lookup() {
        let mut c = Catalog::new();
        let oid = c.create_table("public", "users", cols()).unwrap();
        assert!(oid >= 16384);
        let t = c.lookup("public", "users").unwrap();
        assert_eq!(t.columns.len(), 2);
    }

    #[test]
    fn create_duplicate_errors() {
        let mut c = Catalog::new();
        c.create_table("public", "t", cols()).unwrap();
        let err = c.create_table("public", "t", cols()).unwrap_err();
        assert_eq!(err, CatalogError::TableExists("t".into()));
    }

    #[test]
    fn lookup_missing_errors() {
        let c = Catalog::new();
        let err = c.lookup("public", "nope").unwrap_err();
        assert_eq!(err, CatalogError::TableNotFound("nope".into()));
    }

    #[test]
    fn drop_then_lookup_fails() {
        let mut c = Catalog::new();
        c.create_table("public", "t", cols()).unwrap();
        c.drop_table("public", "t").unwrap();
        assert!(c.lookup("public", "t").is_err());
    }

    #[test]
    fn drop_unknown_errors() {
        let mut c = Catalog::new();
        let err = c.drop_table("public", "nope").unwrap_err();
        assert_eq!(err, CatalogError::TableNotFound("nope".into()));
    }

    #[test]
    fn create_in_unknown_schema_errors() {
        let mut c = Catalog::new();
        let err = c.create_table("ghost", "t", cols()).unwrap_err();
        assert_eq!(err, CatalogError::SchemaNotFound("ghost".into()));
    }

    #[test]
    fn resolve_searches_public_first() {
        let mut c = Catalog::new();
        c.create_table("public", "t", cols()).unwrap();
        c.create_table("pg_catalog", "t", cols()).unwrap();
        let resolved = c.resolve("t").unwrap();
        assert_eq!(resolved.schema, "public");
    }

    #[test]
    fn resolve_falls_back_to_pg_catalog() {
        let mut c = Catalog::new();
        c.create_table("pg_catalog", "pg_class", cols()).unwrap();
        let resolved = c.resolve("pg_class").unwrap();
        assert_eq!(resolved.schema, "pg_catalog");
    }

    #[test]
    fn list_tables_returns_all() {
        let mut c = Catalog::new();
        c.create_table("public", "a", cols()).unwrap();
        c.create_table("public", "b", cols()).unwrap();
        let list = c.list_tables("public").unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn create_schema_then_create_table() {
        let mut c = Catalog::new();
        c.create_schema("acme");
        c.create_table("acme", "users", cols()).unwrap();
        assert!(c.lookup("acme", "users").is_ok());
    }

    #[test]
    fn column_index_returns_position() {
        let mut c = Catalog::new();
        c.create_table("public", "t", cols()).unwrap();
        let t = c.lookup("public", "t").unwrap();
        assert_eq!(t.column_index("id"), Some(0));
        assert_eq!(t.column_index("name"), Some(1));
        assert_eq!(t.column_index("missing"), None);
    }

    #[test]
    fn qualified_name_omits_public() {
        let mut c = Catalog::new();
        c.create_table("public", "t", cols()).unwrap();
        let t = c.lookup("public", "t").unwrap();
        assert_eq!(t.qualified_name(), "t");
    }

    #[test]
    fn qualified_name_includes_other_schema() {
        let mut c = Catalog::new();
        c.create_schema("acme");
        c.create_table("acme", "t", cols()).unwrap();
        let t = c.lookup("acme", "t").unwrap();
        assert_eq!(t.qualified_name(), "acme.t");
    }

    #[test]
    fn oid_is_monotonic() {
        let mut c = Catalog::new();
        let o1 = c.create_table("public", "a", cols()).unwrap();
        let o2 = c.create_table("public", "b", cols()).unwrap();
        assert!(o2 > o1);
    }

    // ── GlobalCatalog (pg_database + pg_authid) ─────────────────────────────

    #[test]
    fn global_catalog_seeds_canonical_databases() {
        let g = GlobalCatalog::with_defaults();
        assert!(g.database("template0").unwrap().datistemplate);
        assert!(!g.database("template0").unwrap().datallowconn);
        assert!(g.database("template1").unwrap().datistemplate);
        assert!(g.database("template1").unwrap().datallowconn);
        assert!(!g.database("postgres").unwrap().datistemplate);
    }

    #[test]
    fn global_catalog_seeds_bootstrap_superuser() {
        let g = GlobalCatalog::with_defaults();
        let r = g.role("postgres").unwrap();
        assert_eq!(r.oid, 10);
        assert!(r.rolsuper);
        assert!(r.rolcanlogin);
        assert!(r.rolreplication);
    }

    #[test]
    fn create_database_then_lookup() {
        let mut g = GlobalCatalog::with_defaults();
        let oid = g.create_database("appdb", 10).unwrap();
        let row = g.database("appdb").unwrap();
        assert_eq!(row.oid, oid);
        assert_eq!(row.datdba, 10);
        assert!(!row.datistemplate);
        assert!(row.datallowconn);
    }

    #[test]
    fn create_database_duplicate_errors() {
        let mut g = GlobalCatalog::with_defaults();
        let err = g.create_database("postgres", 10).unwrap_err();
        assert_eq!(err, CatalogError::TableExists("postgres".into()));
    }

    #[test]
    fn drop_database_then_missing() {
        let mut g = GlobalCatalog::with_defaults();
        g.drop_database("template1").unwrap();
        assert!(g.database("template1").is_none());
    }

    #[test]
    fn create_role_then_lookup() {
        let mut g = GlobalCatalog::with_defaults();
        let oid = g.create_role("alice", true).unwrap();
        let r = g.role("alice").unwrap();
        assert_eq!(r.oid, oid);
        assert!(r.rolcanlogin);
        assert!(!r.rolsuper);
    }

    #[test]
    fn create_role_no_login_role() {
        let mut g = GlobalCatalog::with_defaults();
        g.create_role("readers", false).unwrap();
        assert!(!g.role("readers").unwrap().rolcanlogin);
    }

    #[test]
    fn list_databases_and_roles_includes_seed_data() {
        let g = GlobalCatalog::with_defaults();
        assert_eq!(g.list_databases().len(), 3);
        assert_eq!(g.list_roles().len(), 1);
    }

    #[test]
    fn drop_role_then_missing() {
        let mut g = GlobalCatalog::with_defaults();
        g.create_role("bob", true).unwrap();
        g.drop_role("bob").unwrap();
        assert!(g.role("bob").is_none());
    }

    #[test]
    fn drop_unknown_role_errors() {
        let mut g = GlobalCatalog::with_defaults();
        let err = g.drop_role("ghost").unwrap_err();
        assert_eq!(err, CatalogError::TableNotFound("ghost".into()));
    }
}
