// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SessionCatalog — registry of tables visible to a SessionContext.
//!
//! Upstream: `crates/datafusion-catalog/src/{schema.rs,catalog.rs}`
//!
//! DataFusion's catalog hierarchy is `CatalogProviderList → CatalogProvider
//! → SchemaProvider → TableProvider`. For the MVP we collapse the
//! hierarchy to a single flat map (table-name → TableProvider) — a single
//! SessionContext talks to a single SchemaProvider, which is the common
//! case for embedded use. Multi-catalog support is deferred.

use crate::data_source::TableProvider;
use crate::error::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct SessionCatalog {
    tables: Arc<RwLock<HashMap<String, Arc<dyn TableProvider>>>>,
}

impl SessionCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register_table(
        &self,
        name: impl Into<String>,
        provider: Arc<dyn TableProvider>,
    ) -> Result<()> {
        let name = name.into();
        let mut g = self.tables.write().await;
        if g.contains_key(&name) {
            return Err(Error::Plan(format!("table {} already registered", name)));
        }
        g.insert(name, provider);
        Ok(())
    }

    pub async fn deregister_table(&self, name: &str) -> Result<()> {
        let mut g = self.tables.write().await;
        g.remove(name)
            .ok_or_else(|| Error::NotFound(format!("table {}", name)))?;
        Ok(())
    }

    pub async fn table(&self, name: &str) -> Result<Arc<dyn TableProvider>> {
        let g = self.tables.read().await;
        g.get(name)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("table {}", name)))
    }

    pub async fn list_tables(&self) -> Vec<String> {
        let g = self.tables.read().await;
        let mut v: Vec<String> = g.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_source::MemTable;
    use crate::schema::{DataType, Field, TableSchema};

    fn empty_provider() -> Arc<dyn TableProvider> {
        Arc::new(
            MemTable::new(
                Arc::new(TableSchema::new(vec![Field::new(
                    "x",
                    DataType::Int64,
                    false,
                )])),
                vec![],
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn register_and_lookup() {
        let cat = SessionCatalog::new();
        cat.register_table("t", empty_provider()).await.unwrap();
        assert!(cat.table("t").await.is_ok());
        assert_eq!(cat.list_tables().await, vec!["t".to_string()]);
    }

    #[tokio::test]
    async fn register_duplicate_errors() {
        let cat = SessionCatalog::new();
        cat.register_table("t", empty_provider()).await.unwrap();
        let r = cat.register_table("t", empty_provider()).await;
        assert!(matches!(r, Err(Error::Plan(_))));
    }

    #[tokio::test]
    async fn deregister_missing_errors() {
        let cat = SessionCatalog::new();
        let r = cat.deregister_table("nope").await;
        assert!(matches!(r, Err(Error::NotFound(_))));
    }
}
