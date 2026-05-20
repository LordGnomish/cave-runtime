// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! In-memory Iceberg catalog implementation.
//!
//! Upstream: `crates/iceberg-catalog-memory/src/lib.rs`
//!
//! Backs the Catalog trait with HashMaps under a tokio RwLock. This is
//! the canonical impl for unit tests + single-node deployments. The
//! commit primitive `replace_table_metadata` simply swaps the pointer
//! atomically — there is no transactional rollback (the caller commits
//! by constructing the new metadata and handing it over).

use crate::catalog::Catalog;
use crate::error::{Error, Result};
use crate::namespace::{Namespace, NamespaceIdent};
use crate::table::{Table, TableIdent};
use crate::table_metadata::TableMetadata;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub struct MemoryCatalog {
    namespaces: Arc<RwLock<HashMap<NamespaceIdent, Namespace>>>,
    tables: Arc<RwLock<HashMap<TableIdent, TableMetadata>>>,
}

impl MemoryCatalog {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Catalog for MemoryCatalog {
    async fn create_namespace(&self, ns: &Namespace) -> Result<()> {
        let mut g = self.namespaces.write().await;
        if g.contains_key(&ns.ident) {
            return Err(Error::AlreadyExists(format!("namespace {}", ns.ident.as_dot())));
        }
        g.insert(ns.ident.clone(), ns.clone());
        Ok(())
    }

    async fn drop_namespace(&self, ident: &NamespaceIdent) -> Result<()> {
        let mut g = self.namespaces.write().await;
        g.remove(ident)
            .ok_or_else(|| Error::NotFound(format!("namespace {}", ident.as_dot())))?;
        // Cascade — drop tables in that namespace.
        let mut t = self.tables.write().await;
        t.retain(|k, _| &k.namespace != ident);
        Ok(())
    }

    async fn list_namespaces(&self, parent: Option<&NamespaceIdent>) -> Result<Vec<NamespaceIdent>> {
        let g = self.namespaces.read().await;
        let mut out: Vec<NamespaceIdent> = match parent {
            None => g.keys().cloned().collect(),
            Some(p) => g
                .keys()
                .filter(|k| k.0.starts_with(&p.0) && k.0.len() == p.0.len() + 1)
                .cloned()
                .collect(),
        };
        out.sort_by(|a, b| a.as_dot().cmp(&b.as_dot()));
        Ok(out)
    }

    async fn namespace_exists(&self, ident: &NamespaceIdent) -> Result<bool> {
        Ok(self.namespaces.read().await.contains_key(ident))
    }

    async fn create_table(&self, ident: &TableIdent, metadata: TableMetadata) -> Result<Table> {
        // Require namespace exists.
        if !self.namespace_exists(&ident.namespace).await? && !ident.namespace.is_root() {
            return Err(Error::NotFound(format!(
                "namespace {}",
                ident.namespace.as_dot()
            )));
        }
        let mut g = self.tables.write().await;
        if g.contains_key(ident) {
            return Err(Error::AlreadyExists(format!(
                "table {}.{}",
                ident.namespace.as_dot(),
                ident.name
            )));
        }
        g.insert(ident.clone(), metadata.clone());
        Ok(Table::new(ident.clone(), metadata))
    }

    async fn drop_table(&self, ident: &TableIdent) -> Result<()> {
        let mut g = self.tables.write().await;
        g.remove(ident)
            .ok_or_else(|| Error::NotFound(format!("table {}", ident.name)))?;
        Ok(())
    }

    async fn load_table(&self, ident: &TableIdent) -> Result<Table> {
        let g = self.tables.read().await;
        let m = g
            .get(ident)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("table {}", ident.name)))?;
        Ok(Table::new(ident.clone(), m))
    }

    async fn list_tables(&self, ns: &NamespaceIdent) -> Result<Vec<TableIdent>> {
        let g = self.tables.read().await;
        let mut out: Vec<TableIdent> = g
            .keys()
            .filter(|k| &k.namespace == ns)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn table_exists(&self, ident: &TableIdent) -> Result<bool> {
        Ok(self.tables.read().await.contains_key(ident))
    }

    async fn rename_table(&self, from: &TableIdent, to: &TableIdent) -> Result<()> {
        let mut g = self.tables.write().await;
        let m = g
            .remove(from)
            .ok_or_else(|| Error::NotFound(format!("table {}", from.name)))?;
        if g.contains_key(to) {
            // Restore + error.
            g.insert(from.clone(), m);
            return Err(Error::AlreadyExists(format!("table {}", to.name)));
        }
        g.insert(to.clone(), m);
        Ok(())
    }

    async fn replace_table_metadata(
        &self,
        ident: &TableIdent,
        new_metadata: TableMetadata,
    ) -> Result<Table> {
        let mut g = self.tables.write().await;
        if !g.contains_key(ident) {
            return Err(Error::NotFound(format!("table {}", ident.name)));
        }
        g.insert(ident.clone(), new_metadata.clone());
        Ok(Table::new(ident.clone(), new_metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;
    use crate::table_metadata::TableMetadataBuilder;

    async fn bootstrap() -> (MemoryCatalog, TableMetadata) {
        let cat = MemoryCatalog::new();
        let ns = Namespace::new(NamespaceIdent::from_dot("analytics"));
        cat.create_namespace(&ns).await.unwrap();
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(Schema::default())
            .build()
            .unwrap();
        (cat, m)
    }

    #[tokio::test]
    async fn create_and_load_table() {
        let (cat, m) = bootstrap().await;
        let ident = TableIdent::from_dot("analytics.t1");
        cat.create_table(&ident, m).await.unwrap();
        let t = cat.load_table(&ident).await.unwrap();
        assert_eq!(t.ident.name, "t1");
    }

    #[tokio::test]
    async fn create_table_requires_namespace() {
        let cat = MemoryCatalog::new();
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(Schema::default())
            .build()
            .unwrap();
        let ident = TableIdent::from_dot("nope.t1");
        let r = cat.create_table(&ident, m).await;
        assert!(matches!(r, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn duplicate_table_errors() {
        let (cat, m) = bootstrap().await;
        let ident = TableIdent::from_dot("analytics.t1");
        cat.create_table(&ident, m.clone()).await.unwrap();
        let r = cat.create_table(&ident, m).await;
        assert!(matches!(r, Err(Error::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn drop_namespace_cascades() {
        let (cat, m) = bootstrap().await;
        cat.create_table(&TableIdent::from_dot("analytics.t1"), m.clone())
            .await
            .unwrap();
        cat.drop_namespace(&NamespaceIdent::from_dot("analytics"))
            .await
            .unwrap();
        assert!(!cat
            .table_exists(&TableIdent::from_dot("analytics.t1"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn rename_swaps_pointer() {
        let (cat, m) = bootstrap().await;
        cat.create_table(&TableIdent::from_dot("analytics.t1"), m.clone())
            .await
            .unwrap();
        cat.rename_table(
            &TableIdent::from_dot("analytics.t1"),
            &TableIdent::from_dot("analytics.t2"),
        )
        .await
        .unwrap();
        assert!(cat
            .table_exists(&TableIdent::from_dot("analytics.t2"))
            .await
            .unwrap());
        assert!(!cat
            .table_exists(&TableIdent::from_dot("analytics.t1"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn rename_into_existing_fails_and_restores() {
        let (cat, m) = bootstrap().await;
        cat.create_table(&TableIdent::from_dot("analytics.t1"), m.clone())
            .await
            .unwrap();
        cat.create_table(&TableIdent::from_dot("analytics.t2"), m.clone())
            .await
            .unwrap();
        let r = cat
            .rename_table(
                &TableIdent::from_dot("analytics.t1"),
                &TableIdent::from_dot("analytics.t2"),
            )
            .await;
        assert!(matches!(r, Err(Error::AlreadyExists(_))));
        assert!(cat
            .table_exists(&TableIdent::from_dot("analytics.t1"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn replace_metadata_updates_pointer() {
        let (cat, m) = bootstrap().await;
        let ident = TableIdent::from_dot("analytics.t1");
        cat.create_table(&ident, m).await.unwrap();
        let mut new_meta = TableMetadataBuilder::new()
            .location("s3://x/t-new")
            .schema(Schema::default())
            .build()
            .unwrap();
        new_meta.last_updated_ms = 42;
        let t = cat.replace_table_metadata(&ident, new_meta).await.unwrap();
        assert_eq!(t.metadata.location, "s3://x/t-new");
        assert_eq!(t.metadata.last_updated_ms, 42);
    }
}
