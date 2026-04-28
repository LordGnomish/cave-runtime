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
}
